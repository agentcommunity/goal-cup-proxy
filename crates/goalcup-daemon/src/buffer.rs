//! The encrypted write-ahead buffer.
//!
//! Partition, then redact, then hash, then **buffer**. `ChainWriter` has already computed
//! the link and put the line in `chain.jsonl` by the time an envelope reaches this module,
//! so a daemon that dies between the two restarts from a durable head rather than from a
//! record only it remembers.
//!
//! **The registry is a separate database.** The entry registry ([`crate::registry`]) is its
//! own redb file, so a buffer corrupted mid-run still leaves the daemon able to say what it
//! was running, whose keys it held and which receipt it had.
//!
//! **What the encryption is for.** Each envelope is sealed on its own with `age` under an
//! `age::scrypt` passphrase recipient whose passphrase is the run's
//! `RunKeys.buffer_key_hex`. Individually, so a torn write costs one record rather than
//! the run; a passphrase rather than an identity file, because a key file beside the
//! buffer protects nothing and adds a file to lose.
//!
//! This buys **confidentiality against a third party who takes the laptop, and nothing
//! else**. It is not integrity against the participant and never can be: D14 rules out
//! participant-held keys for the evidence, the daemon runs as the participant, and anyone
//! who can read `RunKeys.buffer_key_hex` out of the registry can re-encrypt whatever they
//! like into this file. The integrity anchor is the server's acknowledgements — which is
//! why the uploader verifies them against the pinned key and why V12 exists. No assertion
//! in this module should ever be read as a tamper check.
//!
//! **Contiguous, or short** (the devserver's chunk contract). The server accepts a chunk
//! that starts at `last_acked + 1` and carries an unbroken range, and it refuses anything
//! else *permanently* — a retry of an older range is refused too, and only the chunk it last
//! acknowledged is idempotent. So this module owes the uploader two things a plain range
//! scan does not give it. [`Buffer::read_from`] stops at the first missing seq instead of
//! stepping over it, and reports where it stopped, because a chunk with a hole in it is not
//! a slow upload but a dead one. And [`Buffer::ack`] writes the acknowledged seq **to disk**
//! in the same transaction as the deletion, so a restarted daemon resumes at
//! `last_acked + 1` rather than re-offering a range the server has already anchored.
//!
//! **Bounded, at both ends**. One envelope per write batch, because
//! a many-file poll batch can exceed 1 MiB even though each file is capped; and a read
//! takes at most the chunk maximum it was handed. Nothing here holds a run in memory, and
//! raw bodies over 168 hours can reach tens of GB per entry.
//!
//! **Disk pressure is reported, not discovered.** [`Buffer::free_bytes`] is one `statvfs`
//! call so the daemon can reach `HealthState::DiskPressure` before `append` starts failing
//! with an opaque database error. The health ladder owns the transition; this module owns
//! the number.

use crate::DaemonError;
use age::secrecy::{ExposeSecret, SecretString};
use goalcup_platform::tracecap::EntryArtifactRoot;
use goalcup_protocol::envelope::Envelope;
use redb::{
    Database, ReadTransaction, ReadableTable, ReadableTableMetadata, TableDefinition, TableHandle,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One sealed envelope per row, keyed by its chain `seq`. The key is the seq and not a
/// counter of this module's own: the chain assigns positions, and a buffer that numbered
/// its own rows would be a second answer to "where is this run" for a restart to disagree
/// with.
const ENVELOPES: TableDefinition<u64, &[u8]> = TableDefinition::new("envelopes");

/// Redacted bytes that arrived after admission closed. This is deliberately not an
/// envelope table: these records are retained evidence, never chain input and never an
/// upload source.
const OUT_OF_WINDOW: TableDefinition<u64, &[u8]> = TableDefinition::new("out_of_window_v1");

/// Buffer-wide scalars: the key check, and the acknowledgement watermark.
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

const KEY_CHECK: &str = "key_check";

/// The highest seq the server has acknowledged, big-endian, written by [`Buffer::ack`] in
/// the same transaction as the deletion it records.
///
/// Durable rather than in-process because the devserver's chunk contract is not forgiving:
/// the next chunk must start at `last_acked + 1`, a retry of an older range is refused
/// rather than re-acknowledged, and the refusal does not clear. A daemon that came back from
/// a restart and read from the beginning of what was left in the buffer would offer the
/// acknowledged record again and be refused for the rest of the run.
///
/// **Not sealed**, unlike [`KEY_CHECK`]. The row keys of the envelope table are already
/// plaintext seqs, so a number saying how far the run got tells a third party with the file
/// nothing they could not get by counting rows — and sealing it would put a KDF run on every
/// ack and every open for no confidentiality at all.
const LAST_ACKED: &str = "last_acked";

const OUT_OF_WINDOW_RECORDS: &str = "out_of_window_records_v1";
const OUT_OF_WINDOW_BYTES: &str = "out_of_window_bytes_v1";
const OUT_OF_WINDOW_OVERFLOWED: &str = "out_of_window_overflowed_v1";
const OUT_OF_WINDOW_SCHEMA: &str = "out_of_window_schema";
const OUT_OF_WINDOW_BATCH_SCHEMA: &str = "out_of_window_batch_schema_v1";
const OUT_OF_WINDOW_BATCH_ADMISSION: &str = "out_of_window_batch_admission_v1";
const OUT_OF_WINDOW_PROGRESS: TableDefinition<[u8; 32], &[u8]> =
    TableDefinition::new("out_of_window_progress_v1");
const OUT_OF_WINDOW_TRANSITIONS: TableDefinition<[u8; 32], &[u8]> =
    TableDefinition::new("out_of_window_transitions_v1");
const OUT_OF_WINDOW_BATCH_FRAME_VERSION: u8 = 1;
const OUT_OF_WINDOW_BATCH_SCHEMA_VERSION: u8 = 1;
const OUT_OF_WINDOW_MAX_CURSOR_BYTES: usize = 1 << 20;
const OUT_OF_WINDOW_MAX_SOURCE_COORDINATE_BYTES: usize = 1 << 20;
const OUT_OF_WINDOW_RECORD_VERSION: u8 = 1;
const OUT_OF_WINDOW_HEADER_BYTES: usize = 9;

/// Sealed under the run key when the buffer is created and unsealed on every open, so
/// "this file belongs to this run" is answered on an **empty** buffer too. Without it a
/// wrong key would open a fresh-looking store, and the daemon would start a second chain
/// over the top of the first one's file.
const KEY_CHECK_PLAINTEXT: &[u8] = b"goalcup write-ahead buffer v1";

/// `N = 2^1`, and deliberately the floor rather than a compromise.
///
/// scrypt's work factor exists to make a *human* passphrase expensive to guess. This
/// passphrase is `RunKeys.buffer_key_hex` — 32 bytes straight out of `OsRng`, 256 bits of
/// entropy that no amount of stretching improves and no attacker enumerates. So the work
/// factor buys nothing here, and it is not free: it is paid once per envelope on write and
/// once per envelope on read, which at `upload_chunk_max` of 256 is 256 KDF runs per
/// upload cycle. Measured in a debug build on the development machine: log_n 1 costs
/// 0.8 ms a record, log_n 6 costs 4.6 ms, log_n 10 costs 65 ms — the last of which is
/// sixteen seconds to read one chunk, for no security at all.
///
/// The same number caps [`age::scrypt::Identity::set_max_work_factor`], so a buffer file
/// somebody else wrote cannot make this daemon spend minutes and gigabytes on a stanza
/// that asks for them. Since every record this module writes uses exactly this factor, the
/// cap costs nothing and closes the denial-of-service that a low floor would otherwise
/// leave open in the other direction.
const SCRYPT_LOG_N: u8 = 1;

/// One run's write-ahead buffer.
///
/// `&self` on every operation, including the writes: redb owns the transactions, so a
/// caller does not need a `&mut` to append and the runtime can hand a shared reference to the
/// polling side and the uploading side at once.
pub struct Buffer {
    path: PathBuf,
    db: Database,
    /// The `age` passphrase, kept so the two below can be built on demand.
    passphrase: SecretString,
    /// Built at most **once per buffer, and only if used**.
    ///
    /// Two costs are in play. `age::scrypt::Recipient::new` and `Identity::new` each run
    /// scrypt at log_n 10 and extrapolate a work factor that would take about a second on
    /// this machine — and this module then overwrites that number with [`SCRYPT_LOG_N`] on
    /// the very next line, so every cycle of the measurement is spent on an answer that is
    /// discarded. `age` offers no constructor that skips it. What it does allow is not
    /// paying twice: an open that only creates a buffer seals the key check and never
    /// decrypts, and an open that only drains one never encrypts, so building each side
    /// where it is first needed halves the fixed cost of `Buffer::open`.
    /// Keeping them after that is the other half: the measurement must not be repeated per
    /// record, where it would cost far more than the encryption it is deciding about.
    recipient: OnceLock<age::scrypt::Recipient>,
    identity: OnceLock<age::scrypt::Identity>,
}

/// What one read of the buffer found: a contiguous run of envelopes, and where the run
/// stopped if it stopped at a hole.
///
/// The two halves travel together because the uploader needs both. It may send
/// `envelopes` — they are contiguous from the seq the server is waiting for, which is the
/// only thing the server will accept — and it must **not** wait quietly for the rest: a
/// `gap_at` means the missing seq is never coming, so the run is capped at `gap_at - 1`
/// whatever else happens, and that belongs in a health fact rather than in a retry loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadChunk {
    /// Contiguous, in seq order, starting at the first unacknowledged seq. Possibly empty.
    pub envelopes: Vec<Envelope>,
    /// The first seq the read wanted and did not find, when something above it is buffered.
    ///
    /// `None` when the read ran out of rows or filled its bound — in both of those cases it
    /// stepped over nothing, and the rest may still arrive. The **missing** seq rather than
    /// the last present one, because a hole at seq 0 has no last present one and an
    /// `Option<u64>` holding "the seq before 0" would have to lie about it.
    pub gap_at: Option<u64>,
}

/// Explicit V1 limits supplied by the finalization intent. This private type does not
/// choose policy or defaults; it only makes it impossible for a caller to retain an
/// unbounded late-record stream by omission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutOfWindowLimitsV1 {
    pub(crate) max_records: u64,
    pub(crate) max_record_bytes: u64,
    pub(crate) max_total_bytes: u64,
}

/// Durable summary of the private late-record stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutOfWindowStatsV1 {
    pub(crate) records: u64,
    pub(crate) bytes: u64,
    pub(crate) overflowed: u64,
}

/// An opaque identity for one tailer/source progress stream.  Only [`Buffer`] can create
/// one, from a source coordinate under the buffer's run key; callers cannot inject a raw
/// digest or a low-entropy pathname hash into the progress tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutOfWindowStoreKeyV1([u8; 32]);

impl OutOfWindowStoreKeyV1 {
    #[cfg(test)]
    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The exact bounded set of tailer streams admitted for this run.  It is committed once,
/// encrypted under the buffer key, before any batch can advance a progress row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutOfWindowAdmissionV1 {
    pub(crate) stores: Vec<(OutOfWindowStoreKeyV1, Vec<u8>)>,
    pub(crate) max_transitions_per_store: u64,
}

/// Validation facts that must survive a crash even when the registry's marker did not.
/// The bit assignments are a caller contract; the buffer only ORs them monotonically and
/// binds the exact per-batch reason digest in the encrypted transition row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OutOfWindowBatchValidationV1 {
    pub(crate) flags: u64,
    pub(crate) reason_digest: [u8; 32],
}

impl OutOfWindowBatchValidationV1 {
    pub(crate) const CLEAN: Self = Self {
        flags: 0,
        reason_digest: [0; 32],
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutOfWindowBatchApplyV1 {
    Applied {
        stats: OutOfWindowStatsV1,
        overflowed: u64,
        validation: OutOfWindowBatchValidationV1,
    },
    AlreadyApplied {
        stats: OutOfWindowStatsV1,
        validation: OutOfWindowBatchValidationV1,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchProgressFrame {
    version: u8,
    store_key: [u8; 32],
    cursor: Vec<u8>,
    head: Option<[u8; 32]>,
    transition_count: u64,
    validation: OutOfWindowBatchValidationV1,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchAdmissionFrame {
    version: u8,
    stores: Vec<BatchAdmissionStore>,
    max_transitions_per_store: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchAdmissionStore {
    store_key: [u8; 32],
    cursor: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchTransitionFrame {
    version: u8,
    store_key: [u8; 32],
    before: Vec<u8>,
    after: Vec<u8>,
    previous_head: Option<[u8; 32]>,
    record_count: u64,
    input_bytes: u64,
    batch_digest: [u8; 32],
    validation: OutOfWindowBatchValidationV1,
}

#[derive(Debug, Clone, Copy)]
struct BatchDigest {
    record_count: u64,
    input_bytes: u64,
    digest: [u8; 32],
}

impl ReadChunk {
    /// The last seq before the hole, when the read stopped at one: `gap_at - 1`.
    ///
    /// `None` at a hole in position zero, which is a buffer with no contiguous prefix at
    /// all — read [`Self::gap_at`] when that difference matters.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn gap_after(&self) -> Option<u64> {
        self.gap_at.and_then(|n| n.checked_sub(1))
    }
}

/// The path and nothing else. `recipient` and `identity` hold the run key, and a derived
/// `Debug` would put it in every log line and every interpolated error carrying a
/// `Buffer` — the same leak `RunKeys` and `control::WorkspaceInfo` hand-write their `Debug`
/// to close.
impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer")
            .field("path", &self.path)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl Buffer {
    /// Where an entry's buffer belongs: `<artifact_dir>/buffer.redb`.
    ///
    /// Under the artifact root, beside `trace/`, and never in the shared `config_dir` the
    /// CLI and the MCP server read — neither of those may see a capture buffer. Here
    /// rather than at each call site, so the daemon has one answer to where a run's
    /// buffered bytes are.
    pub fn default_path(artifact_dir: &Path) -> PathBuf {
        artifact_dir.join("buffer.redb")
    }

    /// Open the buffer at `path` under `run_key_hex`, creating it and its directory if
    /// either is missing.
    ///
    /// The key is checked **first**, before a directory is created or a file is opened, so
    /// a caller that fumbled the key leaves nothing behind to be opened later by a caller
    /// that did not. On unix the file is `0o600`, set on creation and re-applied on every
    /// open, exactly as the registry's is: a buffer that was created loose, or loosened
    /// afterwards, is tightened rather than trusted.
    ///
    /// Opening an existing buffer under a key that is not its own is
    /// [`DaemonError::Crypto`] and not an empty store — see [`KEY_CHECK_PLAINTEXT`].
    pub fn open(path: &Path, run_key_hex: &str) -> Result<Buffer, DaemonError> {
        let passphrase = passphrase(run_key_hex)?;

        // A recovered registry row is not authority to follow a buffer symlink. The
        // encrypted store is private evidence, and opening/chmodding a participant-named
        // link could otherwise follow it outside the entry artifact directory before the
        // key check has a chance to reject its contents.
        if path
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(DaemonError::Refused(format!(
                "buffer path {} is a symbolic link",
                path.display()
            )));
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.is_dir()
        {
            std::fs::create_dir_all(parent).map_err(|source| DaemonError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let file = open_private_buffer_file(path)?;
        Self::open_acquired(path, passphrase, file)
    }

    /// Open the buffer through the entry capability retained by the runtime.  Unlike the
    /// legacy path-taking wrapper, this cannot re-resolve `artifacts/<entry>` after an
    /// ancestor has been acquired and later exchanged.
    pub fn open_from_entry_artifact(
        artifact: &EntryArtifactRoot,
        run_key_hex: &str,
    ) -> Result<Buffer, DaemonError> {
        let passphrase = passphrase(run_key_hex)?;
        let file = artifact.open_buffer().map_err(|error| {
            DaemonError::Refused(format!(
                "buffer storage cannot be acquired through retained entry artifact authority: {error}"
            ))
        })?;
        Self::open_acquired(
            &artifact.display_path().join("buffer.redb"),
            passphrase,
            file,
        )
    }

    fn open_acquired(
        path: &Path,
        passphrase: SecretString,
        file: std::fs::File,
    ) -> Result<Buffer, DaemonError> {
        // `mode` above only applies to a file this call created; repair an existing mode on
        // the already-open handle, not by reopening the pathname after an attacker could
        // exchange it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|source| DaemonError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
        }

        let db = Database::builder()
            .create_file(file)
            .map_err(|e| db_err(path, "opening the buffer", e))?;
        let buffer = Buffer {
            path: path.to_path_buf(),
            db,
            passphrase,
            recipient: OnceLock::new(),
            identity: OnceLock::new(),
        };
        buffer.check_key()?;
        Ok(buffer)
    }

    /// Buffer one envelope, keyed by its chain `seq`.
    ///
    /// **One envelope per write batch**, which is the whole reason the tailer's poll batch
    /// is not handed over as a batch: a many-file poll can exceed 1 MiB even though each
    /// file is capped, and a transaction sized by what a poll happened to find is a
    /// transaction with no bound at all.
    ///
    /// Sealing happens outside the transaction, so the write is open for the length of a
    /// redb commit rather than for the length of a KDF.
    ///
    /// Re-appending a seq that is already buffered replaces it, and that is deliberate:
    /// the chain is written before the buffer, so a daemon that died between the two comes
    /// back and appends the same envelope again. Refusing it would turn a clean recovery
    /// into a stuck run.
    pub fn append(&self, envelope: &Envelope) -> Result<(), DaemonError> {
        let sealed = self.seal(&serde_json::to_vec(envelope)?)?;
        let txn = self.db.begin_write().map_err(|e| plain(e, "append"))?;
        {
            let mut table = txn.open_table(ENVELOPES).map_err(|e| plain(e, "append"))?;
            table
                .insert(envelope.link.seq, sealed.as_slice())
                .map_err(|e| plain(e, "append"))?;
        }
        txn.commit().map_err(|e| plain(e, "append"))
    }

    /// Commit the exact bounded set of source streams once.  Repeating the same admission is
    /// harmless; a different set or transition cap is a permanent refusal.
    pub(crate) fn initialize_out_of_window_batch_progress(
        &self,
        admission: OutOfWindowAdmissionV1,
    ) -> Result<(), DaemonError> {
        let mut stores = admission
            .stores
            .into_iter()
            .map(|(store, cursor)| {
                validate_cursor_bytes(&cursor, "admission")?;
                Ok(BatchAdmissionStore {
                    store_key: store.0,
                    cursor,
                })
            })
            .collect::<Result<Vec<_>, DaemonError>>()?;
        stores.sort_by_key(|store| store.store_key);
        if stores.is_empty() || admission.max_transitions_per_store == 0 {
            return Err(DaemonError::Refused(
                "out-of-window admission must contain bounded stores and transitions".to_string(),
            ));
        }
        if stores
            .windows(2)
            .any(|window| window[0].store_key == window[1].store_key)
        {
            return Err(DaemonError::Refused(
                "out-of-window admission contains duplicate stores".to_string(),
            ));
        }
        let desired = BatchAdmissionFrame {
            version: OUT_OF_WINDOW_BATCH_FRAME_VERSION,
            stores,
            max_transitions_per_store: admission.max_transitions_per_store,
        };
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
        let mut has_progress = false;
        let mut has_transitions = false;
        for table in txn
            .list_tables()
            .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?
        {
            has_progress |= table.name() == OUT_OF_WINDOW_PROGRESS.name();
            has_transitions |= table.name() == OUT_OF_WINDOW_TRANSITIONS.name();
        }
        let mut meta = txn
            .open_table(META)
            .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
        let schema = meta
            .get(OUT_OF_WINDOW_BATCH_SCHEMA)
            .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?
            .map(|row| row.value().to_vec());
        let admission = meta
            .get(OUT_OF_WINDOW_BATCH_ADMISSION)
            .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?
            .map(|row| row.value().to_vec());
        let state = classify_batch_journal(
            self,
            schema.as_deref(),
            admission.as_deref(),
            has_progress,
            has_transitions,
            "initialize_out_of_window_batch_progress",
        );
        match state {
            BatchJournalState::Absent => {
                meta.insert(
                    OUT_OF_WINDOW_BATCH_SCHEMA,
                    [OUT_OF_WINDOW_BATCH_SCHEMA_VERSION].as_slice(),
                )
                .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
                let sealed = self.seal(&encode_frame(&desired)?)?;
                meta.insert(OUT_OF_WINDOW_BATCH_ADMISSION, sealed.as_slice())
                    .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
                drop(meta);
                txn.open_table(OUT_OF_WINDOW_PROGRESS)
                    .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
                txn.open_table(OUT_OF_WINDOW_TRANSITIONS)
                    .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
                let mut progress = txn
                    .open_table(OUT_OF_WINDOW_PROGRESS)
                    .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
                for store in &desired.stores {
                    let frame = BatchProgressFrame {
                        version: OUT_OF_WINDOW_BATCH_FRAME_VERSION,
                        store_key: store.store_key,
                        cursor: store.cursor.clone(),
                        head: None,
                        transition_count: 0,
                        validation: OutOfWindowBatchValidationV1::CLEAN,
                    };
                    let sealed = self.seal(&encode_frame(&frame)?)?;
                    if progress
                        .insert(store.store_key, sealed.as_slice())
                        .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?
                        .is_some()
                    {
                        return Err(DaemonError::Db(
                            "buffer initialize_out_of_window_batch_progress: duplicate progress"
                                .to_string(),
                        ));
                    }
                }
            }
            BatchJournalState::FullyPresent(existing) => {
                drop(meta);
                let progress = txn
                    .open_table(OUT_OF_WINDOW_PROGRESS)
                    .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
                let transitions = txn
                    .open_table(OUT_OF_WINDOW_TRANSITIONS)
                    .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))?;
                validate_batch_layout(
                    self,
                    &progress,
                    &transitions,
                    &existing,
                    "initialize_out_of_window_batch_progress",
                )?;
                if existing != desired {
                    return Err(DaemonError::Refused(
                        "out-of-window admission has already been set differently".to_string(),
                    ));
                }
            }
            BatchJournalState::Corrupt => {
                return Err(DaemonError::Db(
                    "buffer initialize_out_of_window_batch_progress: batch journal state is absent only when all artifacts are absent; partial or corrupt state found"
                        .to_string(),
                ));
            }
        }
        txn.commit()
            .map_err(|e| plain(e, "initialize_out_of_window_batch_progress"))
    }

    /// Atomically apply one opaque source-coordinate transition.
    ///
    /// The transition key is a keyed digest of `(store_key, before_cursor)` and contains no
    /// pathname or cursor bytes.  The encrypted row carries the exact before/after cursors,
    /// canonical batch digest, and validation facts, so swapping keys or rebinding a row
    /// fails closed.  An existing transition is checked before limits are charged, making a
    /// crash replay safe even after the store reached its caps.  New transitions require
    /// the durable progress cursor to equal `before_cursor`, then append all admissible
    /// records and advance progress in this same redb transaction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_out_of_window_batch_with_validation(
        &self,
        store_key: OutOfWindowStoreKeyV1,
        before_cursor: &[u8],
        after_cursor: &[u8],
        canonical_records: &[Vec<u8>],
        validation: OutOfWindowBatchValidationV1,
        limits: OutOfWindowLimitsV1,
    ) -> Result<OutOfWindowBatchApplyV1, DaemonError> {
        validate_cursor_bytes(before_cursor, "before")?;
        validate_cursor_bytes(after_cursor, "after")?;
        let batch = batch_digest(canonical_records)?;
        let transition_key = self.batch_transition_key(&store_key, before_cursor);

        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?;
        let mut has_progress = false;
        let mut has_transitions = false;
        for table in txn
            .list_tables()
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?
        {
            has_progress |= table.name() == OUT_OF_WINDOW_PROGRESS.name();
            has_transitions |= table.name() == OUT_OF_WINDOW_TRANSITIONS.name();
        }

        let mut out_of_window = txn
            .open_table(OUT_OF_WINDOW)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?;
        let mut meta = txn
            .open_table(META)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?;
        let schema = meta
            .get(OUT_OF_WINDOW_BATCH_SCHEMA)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?
            .map(|row| row.value().to_vec());
        let admission = meta
            .get(OUT_OF_WINDOW_BATCH_ADMISSION)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?
            .map(|row| row.value().to_vec());
        let admission = match classify_batch_journal(
            self,
            schema.as_deref(),
            admission.as_deref(),
            has_progress,
            has_transitions,
            "apply_out_of_window_batch",
        ) {
            BatchJournalState::FullyPresent(admission) => admission,
            BatchJournalState::Absent => {
                return Err(DaemonError::Refused(
                    "out-of-window batch progress has not been admitted".to_string(),
                ));
            }
            BatchJournalState::Corrupt => {
                return Err(DaemonError::Db(
                    "buffer apply_out_of_window_batch: batch journal state is partial or corrupt"
                        .to_string(),
                ));
            }
        };
        if !admission
            .stores
            .iter()
            .any(|store| store.store_key == store_key.0)
        {
            return Err(DaemonError::Refused(
                "out-of-window batch store is outside the admitted set".to_string(),
            ));
        }
        let mut progress = txn
            .open_table(OUT_OF_WINDOW_PROGRESS)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?;
        let mut transitions = txn
            .open_table(OUT_OF_WINDOW_TRANSITIONS)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?;

        let existing_transition = transitions
            .get(transition_key)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?
            .map(|row| row.value().to_vec());
        if let Some(sealed) = existing_transition {
            let existing = decode_transition(self, &sealed, "apply_out_of_window_batch")?;
            if existing.store_key != store_key.0
                || existing.before != before_cursor
                || existing.after != after_cursor
                || existing.record_count != batch.record_count
                || existing.input_bytes != batch.input_bytes
                || existing.batch_digest != batch.digest
                || existing.validation != validation
            {
                return Err(DaemonError::Refused(
                    "out-of-window transition replay diverged".to_string(),
                ));
            }
            let current = progress
                .get(store_key.0)
                .map_err(|e| plain(e, "apply_out_of_window_batch"))?
                .map(|row| row.value().to_vec())
                .ok_or_else(|| {
                    DaemonError::Db(
                        "buffer apply_out_of_window_batch: transition has no progress row"
                            .to_string(),
                    )
                })?;
            let current = decode_progress(self, &current, "apply_out_of_window_batch")?;
            if current.store_key != store_key.0 {
                return Err(DaemonError::Db(
                    "buffer apply_out_of_window_batch: progress key binding failed".to_string(),
                ));
            }
            if (current.transition_count == 0) != current.head.is_none() {
                return Err(DaemonError::Db(
                    "buffer apply_out_of_window_batch: progress count/head binding failed"
                        .to_string(),
                ));
            }
            let stats = out_of_window_stats_from_meta(&meta, "apply_out_of_window_batch")?;
            drop(out_of_window);
            drop(meta);
            drop(progress);
            drop(transitions);
            txn.commit()
                .map_err(|e| plain(e, "apply_out_of_window_batch"))?;
            return Ok(OutOfWindowBatchApplyV1::AlreadyApplied {
                stats,
                validation: current.validation,
            });
        }

        let current_progress = progress
            .get(store_key.0)
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?
            .map(|row| decode_progress(self, row.value(), "apply_out_of_window_batch"))
            .transpose()?;
        let previous_validation = match &current_progress {
            Some(current) => {
                if current.store_key != store_key.0 || current.cursor != before_cursor {
                    return Err(DaemonError::Refused(
                        "out-of-window source progress does not match the before cursor"
                            .to_string(),
                    ));
                }
                if (current.transition_count == 0) != current.head.is_none() {
                    return Err(DaemonError::Db(
                        "buffer apply_out_of_window_batch: progress count/head binding failed"
                            .to_string(),
                    ));
                }
                current.validation
            }
            None => {
                return Err(DaemonError::Refused(
                    "out-of-window admitted source has no progress row".to_string(),
                ));
            }
        };

        let mut stats = out_of_window_stats_from_meta(&meta, "apply_out_of_window_batch")?;
        validate_out_of_window_layout(&out_of_window, stats.records, "apply_out_of_window_batch")?;
        let mut overflowed = 0u64;
        let mut next_ordinal = stats.records;
        for record in canonical_records {
            let len = u64::try_from(record.len()).map_err(|_| {
                DaemonError::Refused("out-of-window record length is not representable".to_string())
            })?;
            let next_bytes = stats.bytes.checked_add(len);
            let admissible = len <= limits.max_record_bytes
                && stats.records < limits.max_records
                && next_bytes.is_some_and(|bytes| bytes <= limits.max_total_bytes);
            if !admissible {
                overflowed = overflowed.checked_add(1).ok_or_else(|| {
                    DaemonError::Db(
                        "buffer apply_out_of_window_batch: overflow counter exhausted".to_string(),
                    )
                })?;
                continue;
            }
            let encoded = encode_out_of_window(record)?;
            let sealed = self.seal(&encoded)?;
            let replaced = out_of_window
                .insert(next_ordinal, sealed.as_slice())
                .map_err(|e| plain(e, "apply_out_of_window_batch"))?;
            if replaced.is_some() {
                return Err(DaemonError::Db(
                    "buffer apply_out_of_window_batch: arrival ordinal already exists".to_string(),
                ));
            }
            next_ordinal = next_ordinal.checked_add(1).ok_or_else(|| {
                DaemonError::Db(
                    "buffer apply_out_of_window_batch: record counter exhausted".to_string(),
                )
            })?;
            stats.records = next_ordinal;
            stats.bytes = next_bytes.expect("admissible byte count was checked");
        }
        stats.overflowed = stats.overflowed.checked_add(overflowed).ok_or_else(|| {
            DaemonError::Db(
                "buffer apply_out_of_window_batch: overflow counter exhausted".to_string(),
            )
        })?;
        put_counter(&mut meta, OUT_OF_WINDOW_RECORDS, stats.records)?;
        put_counter(&mut meta, OUT_OF_WINDOW_BYTES, stats.bytes)?;
        put_counter(&mut meta, OUT_OF_WINDOW_OVERFLOWED, stats.overflowed)?;

        let combined_validation = combine_validation(previous_validation, validation);
        let previous_head = current_progress.as_ref().and_then(|current| current.head);
        if let Some(previous_head) = previous_head {
            let previous = transitions
                .get(previous_head)
                .map_err(|e| plain(e, "apply_out_of_window_batch"))?
                .map(|row| row.value().to_vec())
                .ok_or_else(|| {
                    DaemonError::Db(
                        "buffer apply_out_of_window_batch: progress head has no transition"
                            .to_string(),
                    )
                })?;
            let previous = decode_transition(self, &previous, "apply_out_of_window_batch")?;
            if previous.store_key != store_key.0
                || previous.after != before_cursor
                || self.batch_transition_key(&store_key, &previous.before) != previous_head
            {
                return Err(DaemonError::Db(
                    "buffer apply_out_of_window_batch: progress head binding failed".to_string(),
                ));
            }
        }
        let transition_count = current_progress
            .as_ref()
            .map_or(0, |current| current.transition_count);
        if transition_count >= admission.max_transitions_per_store {
            return Err(DaemonError::Refused(
                "out-of-window source transition journal reached its bounded cap".to_string(),
            ));
        }
        let transition_count = transition_count.checked_add(1).ok_or_else(|| {
            DaemonError::Db(
                "buffer apply_out_of_window_batch: transition count exhausted".to_string(),
            )
        })?;
        let transition = BatchTransitionFrame {
            version: OUT_OF_WINDOW_BATCH_FRAME_VERSION,
            store_key: store_key.0,
            before: before_cursor.to_vec(),
            after: after_cursor.to_vec(),
            previous_head,
            record_count: batch.record_count,
            input_bytes: batch.input_bytes,
            batch_digest: batch.digest,
            validation,
        };
        let progress_frame = BatchProgressFrame {
            version: OUT_OF_WINDOW_BATCH_FRAME_VERSION,
            store_key: store_key.0,
            cursor: after_cursor.to_vec(),
            head: Some(transition_key),
            transition_count,
            validation: combined_validation,
        };
        let sealed_transition = self.seal(&encode_frame(&transition)?)?;
        let sealed_progress = self.seal(&encode_frame(&progress_frame)?)?;
        if transitions
            .insert(transition_key, sealed_transition.as_slice())
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?
            .is_some()
        {
            return Err(DaemonError::Db(
                "buffer apply_out_of_window_batch: transition key already exists".to_string(),
            ));
        }
        progress
            .insert(store_key.0, sealed_progress.as_slice())
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?;

        drop(out_of_window);
        drop(meta);
        drop(progress);
        drop(transitions);
        txn.commit()
            .map_err(|e| plain(e, "apply_out_of_window_batch"))?;
        Ok(OutOfWindowBatchApplyV1::Applied {
            stats,
            overflowed,
            validation: combined_validation,
        })
    }

    /// Return the monotone validation facts committed by batch progress.  This is separate
    /// from the stats API.
    pub(crate) fn verify_out_of_window_batch_state(
        &self,
    ) -> Result<OutOfWindowBatchValidationV1, DaemonError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?;
        let mut has_progress = false;
        let mut has_transitions = false;
        for table in txn
            .list_tables()
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?
        {
            has_progress |= table.name() == OUT_OF_WINDOW_PROGRESS.name();
            has_transitions |= table.name() == OUT_OF_WINDOW_TRANSITIONS.name();
        }
        let meta = txn
            .open_table(META)
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?;
        let schema = meta
            .get(OUT_OF_WINDOW_BATCH_SCHEMA)
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?
            .map(|row| row.value().to_vec());
        let admission = meta
            .get(OUT_OF_WINDOW_BATCH_ADMISSION)
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?
            .map(|row| row.value().to_vec());
        let state = classify_batch_journal(
            self,
            schema.as_deref(),
            admission.as_deref(),
            has_progress,
            has_transitions,
            "verify_out_of_window_batch_state",
        );
        let admission = match state {
            BatchJournalState::FullyPresent(admission) => admission,
            BatchJournalState::Absent => return Ok(OutOfWindowBatchValidationV1::CLEAN),
            BatchJournalState::Corrupt => {
                return Err(DaemonError::Db(
                    "buffer verify_out_of_window_batch_state: batch journal state is partial or corrupt"
                        .to_string(),
                ));
            }
        };
        drop(meta);
        let progress = txn
            .open_table(OUT_OF_WINDOW_PROGRESS)
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?;
        let transitions = txn
            .open_table(OUT_OF_WINDOW_TRANSITIONS)
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?;
        validate_batch_layout(
            self,
            &progress,
            &transitions,
            &admission,
            "verify_out_of_window_batch_state",
        )?;
        let mut aggregate = OutOfWindowBatchValidationV1::CLEAN;
        for row in progress
            .iter()
            .map_err(|e| plain(e, "verify_out_of_window_batch_state"))?
        {
            let (_, sealed) = row.map_err(|e| plain(e, "verify_out_of_window_batch_state"))?;
            let current =
                decode_progress(self, sealed.value(), "verify_out_of_window_batch_state")?;
            aggregate = combine_validation(aggregate, current.validation);
        }
        Ok(aggregate)
    }

    /// Return the authenticated current cursor for one admitted source coordinate.  Recovery
    /// uses this after reopening so a tailer resumes from the atomically committed transition
    /// head rather than replaying from the immutable admission cursor.  The complete journal
    /// is audited before the cursor is returned; no clear progress-row contents are exposed.
    pub(crate) fn out_of_window_current_cursor(
        &self,
        source_coordinate: &[u8],
    ) -> Result<Vec<u8>, DaemonError> {
        let store_key = self.derive_out_of_window_store_key(source_coordinate)?;
        let txn = self
            .db
            .begin_read()
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?;
        let mut has_progress = false;
        let mut has_transitions = false;
        for table in txn
            .list_tables()
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?
        {
            has_progress |= table.name() == OUT_OF_WINDOW_PROGRESS.name();
            has_transitions |= table.name() == OUT_OF_WINDOW_TRANSITIONS.name();
        }
        let meta = txn
            .open_table(META)
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?;
        let schema = meta
            .get(OUT_OF_WINDOW_BATCH_SCHEMA)
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?
            .map(|row| row.value().to_vec());
        let sealed_admission = meta
            .get(OUT_OF_WINDOW_BATCH_ADMISSION)
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?
            .map(|row| row.value().to_vec());
        let state = classify_batch_journal(
            self,
            schema.as_deref(),
            sealed_admission.as_deref(),
            has_progress,
            has_transitions,
            "out_of_window_current_cursor",
        );
        let admission = match state {
            BatchJournalState::FullyPresent(admission) => admission,
            BatchJournalState::Absent => {
                return Err(DaemonError::Refused(
                    "out-of-window batch progress has not been admitted".to_string(),
                ));
            }
            BatchJournalState::Corrupt => {
                return Err(DaemonError::Db(
                    "buffer out_of_window_current_cursor: batch journal state is partial or corrupt"
                        .to_string(),
                ));
            }
        };
        drop(meta);
        let progress = txn
            .open_table(OUT_OF_WINDOW_PROGRESS)
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?;
        let transitions = txn
            .open_table(OUT_OF_WINDOW_TRANSITIONS)
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?;
        validate_batch_layout(
            self,
            &progress,
            &transitions,
            &admission,
            "out_of_window_current_cursor",
        )?;
        if !admission
            .stores
            .iter()
            .any(|store| store.store_key == store_key.0)
        {
            return Err(DaemonError::Refused(
                "out-of-window source coordinate is outside the admitted set".to_string(),
            ));
        }
        let sealed_progress = progress
            .get(store_key.0)
            .map_err(|e| plain(e, "out_of_window_current_cursor"))?
            .map(|row| row.value().to_vec())
            .ok_or_else(|| {
                DaemonError::Db(
                    "buffer out_of_window_current_cursor: admitted source has no progress row"
                        .to_string(),
                )
            })?;
        let progress = decode_progress(self, &sealed_progress, "out_of_window_current_cursor")?;
        if progress.store_key != store_key.0 {
            return Err(DaemonError::Db(
                "buffer out_of_window_current_cursor: progress key binding failed".to_string(),
            ));
        }
        Ok(progress.cursor)
    }

    /// Verify the retained late-record stream without returning any payload bytes.
    ///
    /// Recovery calls this with the limits latched by finalization. It streams one sealed
    /// row at a time, checks the private version/length frame, reconciles the durable byte
    /// count, and fails closed on gaps, corruption, or a limit mismatch. Payloads never
    /// enter [`ReadChunk`] and cannot be handed to the uploader.
    pub(crate) fn verify_out_of_window(
        &self,
        limits: OutOfWindowLimitsV1,
    ) -> Result<OutOfWindowStatsV1, DaemonError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| plain(e, "verify_out_of_window"))?;
        let table = txn
            .open_table(OUT_OF_WINDOW)
            .map_err(|e| plain(e, "verify_out_of_window"))?;
        let meta = txn
            .open_table(META)
            .map_err(|e| plain(e, "verify_out_of_window"))?;
        let stats = out_of_window_stats_from_meta(&meta, "verify_out_of_window")?;
        validate_out_of_window_layout(&table, stats.records, "verify_out_of_window")?;
        if stats.records > limits.max_records || stats.bytes > limits.max_total_bytes {
            return Err(DaemonError::Refused(
                "out-of-window evidence exceeds the latched limits".to_string(),
            ));
        }

        let mut bytes = 0u64;
        for row in table.iter().map_err(|e| plain(e, "verify_out_of_window"))? {
            let (_, sealed) = row.map_err(|e| plain(e, "verify_out_of_window"))?;
            let opened = self.unseal_bytes(sealed.value())?;
            let record_bytes = decode_out_of_window(&opened)?;
            if record_bytes > limits.max_record_bytes {
                return Err(DaemonError::Refused(
                    "out-of-window record exceeds the latched per-record limit".to_string(),
                ));
            }
            bytes = bytes.checked_add(record_bytes).ok_or_else(|| {
                DaemonError::Db("buffer verify_out_of_window: byte count overflowed".to_string())
            })?;
        }
        if bytes != stats.bytes {
            return Err(DaemonError::Db(format!(
                "buffer verify_out_of_window: retained bytes total {bytes} does not match {}",
                stats.bytes
            )));
        }
        drop(table);
        drop(meta);
        drop(txn);
        self.verify_out_of_window_batch_state()?;
        Ok(stats)
    }

    /// The buffered envelopes after `after_seq`, **contiguous**, in seq order, at most `max`
    /// of them.
    ///
    /// `after_seq` is an `Option` and not a `u64` because the stream starts at **zero**:
    /// `TraceExport` numbers from zero, so a `0`
    /// meaning "from where this buffer is up to" would silently skip the genesis envelope on
    /// the first pump. `None` means "from wherever this buffer is up to", which is
    /// [`Buffer::floor`] + 1, or zero when nothing has been acknowledged; `Some(n)` overrides
    /// that and starts at `n + 1`.
    ///
    /// **Contiguous, and short rather than broken.** The rows are keyed by seq and redb's
    /// range iterator skips whatever is not there, so a hole would come back as a chunk
    /// that jumped it. The devserver refuses such a chunk — `first_seq` must be
    /// `last_acked + 1`, and its refusal is permanent, not a retry — so a silent jump does
    /// not cost the missing record, it costs every record after it. This stops at the first
    /// missing seq and puts it in [`ReadChunk::gap_at`], which is a fact for the health
    /// surface rather than something to retry: nothing refills a hole in a write-ahead log.
    ///
    /// `max` is the caller's `upload_chunk_max`, and it is a hard bound on what this
    /// allocates: no run, however long, is read into memory here.
    pub fn read_from(&self, after_seq: Option<u64>, max: usize) -> Result<ReadChunk, DaemonError> {
        let empty = ReadChunk {
            envelopes: Vec::new(),
            gap_at: None,
        };
        let txn = self.db.begin_read().map_err(|e| plain(e, "read_from"))?;
        let Some(start) = self.start_seq(&txn, after_seq, "read_from")? else {
            return Ok(empty);
        };
        if max == 0 {
            return Ok(empty);
        }
        let table = txn
            .open_table(ENVELOPES)
            .map_err(|e| plain(e, "read_from"))?;
        let mut envelopes = Vec::new();
        let mut gap_at = None;
        let mut expected = start;
        for row in table.range(start..).map_err(|e| plain(e, "read_from"))? {
            let (key, sealed) = row.map_err(|e| plain(e, "read_from"))?;
            if key.value() != expected {
                gap_at = Some(expected);
                break;
            }
            envelopes.push(self.unseal(sealed.value())?);
            // `checked_add` because the loop's bound is a seq: at `u64::MAX` there is no
            // next position to expect, and the read is over rather than wrapped.
            match expected.checked_add(1) {
                Some(next) if envelopes.len() < max => expected = next,
                _ => break,
            }
        }
        Ok(ReadChunk { envelopes, gap_at })
    }

    /// The same bounded contiguous read, with an optional terminal ceiling.  The limit is
    /// applied before the database is read so an uploader cannot even construct a chunk
    /// containing a post-terminal envelope.  `None` retains the ordinary open-run path.
    pub fn read_from_through(
        &self,
        after_seq: Option<u64>,
        max: usize,
        terminal_seq: Option<u64>,
    ) -> Result<ReadChunk, DaemonError> {
        let Some(terminal) = terminal_seq else {
            return self.read_from(after_seq, max);
        };
        let txn = self
            .db
            .begin_read()
            .map_err(|e| plain(e, "read_from_through"))?;
        let Some(start) = self.start_seq(&txn, after_seq, "read_from_through")? else {
            return Ok(ReadChunk {
                envelopes: Vec::new(),
                gap_at: None,
            });
        };
        if start > terminal || max == 0 {
            return Ok(ReadChunk {
                envelopes: Vec::new(),
                gap_at: None,
            });
        }
        let remaining = terminal
            .checked_sub(start)
            .and_then(|distance| distance.checked_add(1))
            .ok_or_else(|| DaemonError::Refused("buffer terminal range overflowed".to_string()))?;
        let capped = max.min(usize::try_from(remaining).unwrap_or(usize::MAX));
        drop(txn);
        self.read_from(after_seq, capped)
    }

    /// A terminal buffer may retain the terminal envelope itself, but never a later one.
    /// The caller checks this before every sealed or sealing pump so a corrupt tail cannot
    /// be sent now and only discovered on a later restart.
    pub fn ensure_no_records_after(&self, terminal_seq: u64) -> Result<(), DaemonError> {
        let tail = self.read_from(Some(terminal_seq), 1)?;
        if !tail.envelopes.is_empty() || tail.gap_at.is_some() {
            return Err(DaemonError::Refused(format!(
                "buffer has records after terminal seq {terminal_seq}"
            )));
        }
        Ok(())
    }

    /// The highest seq the server has acknowledged for this buffer, or `None` when nothing
    /// has been.
    ///
    /// The durable half of [`Buffer::ack`], and the answer to "where does the next chunk
    /// start" across a restart: `floor() + 1`, which is the only first seq the devserver
    /// accepts. It is also worth reporting on its own — the distance between this and
    /// [`Buffer::head`] is how far behind the upload has fallen.
    pub fn floor(&self) -> Result<Option<u64>, DaemonError> {
        let txn = self.db.begin_read().map_err(|e| plain(e, "floor"))?;
        floor_in(&txn, "floor")
    }

    /// Delete every record with `seq < up_to_seq`, **keep** the record at `up_to_seq`,
    /// record `up_to_seq` as the acknowledgement watermark, and return how many were
    /// deleted.
    ///
    /// Keeping the acknowledged record is this buffer's policy and costs exactly one record: a
    /// restarted daemon reads its own chain head back out of the buffer and resumes the
    /// chain, rather than finding an empty store and starting a second one. Truncating
    /// through the acknowledged seq would delete the only local copy of the head the
    /// server has anchored. Kept for [`Buffer::head`], though, and **not** for re-reading:
    /// [`Buffer::read_from`] resumes above the watermark, because offering the server a
    /// range it has already acknowledged is refused rather than acknowledged again.
    ///
    /// The watermark and the deletion go in **one** write transaction, so there is no
    /// window in which the records are gone and the record of why they went is not. A crash
    /// either side of the commit leaves a buffer that agrees with itself.
    ///
    /// Acknowledging the same seq twice deletes nothing and is not an error: an
    /// acknowledgement that arrives again after a retry must not become a second failure.
    pub fn ack(&self, up_to_seq: u64) -> Result<usize, DaemonError> {
        let txn = self.db.begin_write().map_err(|e| plain(e, "ack"))?;
        let mut removed = 0usize;
        {
            let mut table = txn.open_table(ENVELOPES).map_err(|e| plain(e, "ack"))?;
            // `extract_from_if` removes only what is read from the iterator, so the loop
            // is the deletion. It streams: the rows are dropped one at a time and no part
            // of the acknowledged range is ever held at once.
            let drained = table
                .extract_from_if(..up_to_seq, |_, _| true)
                .map_err(|e| plain(e, "ack"))?;
            for row in drained {
                row.map_err(|e| plain(e, "ack"))?;
                removed += 1;
            }
            let mut meta = txn.open_table(META).map_err(|e| plain(e, "ack"))?;
            meta.insert(LAST_ACKED, up_to_seq.to_be_bytes().as_slice())
                .map_err(|e| plain(e, "ack"))?;
        }
        txn.commit().map_err(|e| plain(e, "ack"))?;
        Ok(removed)
    }

    /// How many envelopes are still **unacknowledged** after `after_seq`. Keys only —
    /// nothing is decrypted, because a status line asking "how far behind is the upload"
    /// should not cost a KDF run per buffered record.
    ///
    /// `None` follows [`Buffer::read_from`]: it counts from the watermark, so the record
    /// kept at the watermark is on disk but is not pending. `Some(n)` counts above `n`,
    /// which is how a caller asks the other question — `pending(floor() - 1)` includes the
    /// kept record.
    pub fn pending(&self, after_seq: Option<u64>) -> Result<u64, DaemonError> {
        let txn = self.db.begin_read().map_err(|e| plain(e, "pending"))?;
        let Some(start) = self.start_seq(&txn, after_seq, "pending")? else {
            return Ok(0);
        };
        let table = txn.open_table(ENVELOPES).map_err(|e| plain(e, "pending"))?;
        // The common case — nothing acknowledged yet — is the table's own length.
        if start == 0 {
            return table.len().map_err(|e| plain(e, "pending"));
        }
        let mut n = 0u64;
        for row in table.range(start..).map_err(|e| plain(e, "pending"))? {
            row.map_err(|e| plain(e, "pending"))?;
            n += 1;
        }
        Ok(n)
    }

    /// The highest buffered seq and its chain hash, or `None` when the buffer is empty.
    ///
    /// The pair comes from the envelope's own `Link` rather than from the row key, because
    /// what a restart needs is the head the *chain* is at: the seq to resume numbering
    /// from and the hash the next link's `prev` must be.
    pub fn head(&self) -> Result<Option<(u64, String)>, DaemonError> {
        let txn = self.db.begin_read().map_err(|e| plain(e, "head"))?;
        let table = txn.open_table(ENVELOPES).map_err(|e| plain(e, "head"))?;
        match table.last().map_err(|e| plain(e, "head"))? {
            None => Ok(None),
            Some((_, sealed)) => {
                let env = self.unseal(sealed.value())?;
                Ok(Some((env.link.seq, env.link.hash)))
            }
        }
    }

    /// Free bytes on the volume the buffer sits on, or `None` when this platform has no
    /// answer this module trusts.
    ///
    /// One `statvfs` call on the buffer's own directory — the volume that fills is the one
    /// being written to, not the one the process happens to be started from. `f_bavail` is
    /// the blocks available to an **unprivileged** writer, which is what this daemon is;
    /// `f_bfree` would count the reserve the filesystem keeps for root and promise space
    /// the daemon cannot have.
    #[cfg(unix)]
    pub fn free_bytes(&self) -> Option<u64> {
        use std::os::unix::ffi::OsStrExt;
        let dir = match self.path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            // A bare file name: the volume is the working directory's.
            _ => Path::new("."),
        };
        let c_path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c_path` is a NUL-terminated C string that outlives the call, and
        // `stats` is a `libc::statvfs` this code owns and passes by unique pointer. The
        // struct is plain data with no padding invariants, so the zeroed value it starts
        // from is a valid one; it is read only after `statvfs` returned 0, which is the
        // contract under which the kernel filled it in.
        let stats = unsafe {
            let mut stats: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stats) != 0 {
                return None;
            }
            stats
        };
        // POSIX allows `f_frsize` to be zero, in which case the block size is the answer.
        let block = if stats.f_frsize == 0 {
            stats.f_bsize
        } else {
            stats.f_frsize
        };
        // `checked_mul` rather than `*`: the product is a byte count on somebody else's
        // volume, and a free-space report that overflowed into a small number would read
        // as disk pressure on a disk that has room.
        widen(stats.f_bavail).checked_mul(widen(block))
    }

    /// `None`, never a guess. There is no portable free-space call here, and a number this
    /// module invented would put `HealthState::DiskPressure` on a measurement nobody took.
    #[cfg(not(unix))]
    pub fn free_bytes(&self) -> Option<u64> {
        None
    }

    /// Whether free space has fallen under `reserve` — `DaemonConfig.disk_reserve_bytes`,
    /// which defaults to 256 MiB.
    ///
    /// A platform that cannot measure is **not** under pressure: the daemon degrades
    /// visibly on a number it took, and it does not stop a run on the absence of one.
    pub fn under_pressure(&self, reserve: u64) -> bool {
        match self.free_bytes() {
            Some(free) => free < reserve,
            None => false,
        }
    }

    /// Create both tables and settle whether this key is this buffer's, in one write
    /// transaction. Creating the tables here means no later read has to tell "empty" from
    /// "never written".
    fn check_key(&self) -> Result<(), DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| db_err(&self.path, "opening the buffer", e))?;
        {
            let had_out_of_window_table = txn
                .list_tables()
                .map_err(|e| db_err(&self.path, "listing buffer tables", e))?
                .any(|table| table.name() == OUT_OF_WINDOW.name());
            txn.open_table(ENVELOPES)
                .map_err(|e| db_err(&self.path, "creating the envelopes table", e))?;
            let mut meta = txn
                .open_table(META)
                .map_err(|e| db_err(&self.path, "creating the meta table", e))?;
            let stored = meta
                .get(KEY_CHECK)
                .map_err(|e| db_err(&self.path, "reading the key check", e))?
                .map(|v| v.value().to_vec());
            match stored {
                Some(sealed) => {
                    let opened = self.unseal_bytes(&sealed).map_err(|_| self.wrong_key())?;
                    if opened != KEY_CHECK_PLAINTEXT {
                        return Err(self.wrong_key());
                    }
                }
                None => {
                    let sealed = self.seal(KEY_CHECK_PLAINTEXT)?;
                    meta.insert(KEY_CHECK, sealed.as_slice())
                        .map_err(|e| db_err(&self.path, "writing the key check", e))?;
                }
            }

            let schema = meta
                .get(OUT_OF_WINDOW_SCHEMA)
                .map_err(|e| db_err(&self.path, "reading out-of-window schema", e))?
                .map(|row| row.value().to_vec());
            match schema.as_deref() {
                None if !had_out_of_window_table => {
                    for key in [
                        OUT_OF_WINDOW_RECORDS,
                        OUT_OF_WINDOW_BYTES,
                        OUT_OF_WINDOW_OVERFLOWED,
                    ] {
                        if meta
                            .get(key)
                            .map_err(|e| db_err(&self.path, "reading out-of-window metadata", e))?
                            .is_some()
                        {
                            return Err(db_err(
                                &self.path,
                                "opening the buffer",
                                "unmarked out-of-window accounting exists",
                            ));
                        }
                    }
                    meta.insert(
                        OUT_OF_WINDOW_SCHEMA,
                        [OUT_OF_WINDOW_RECORD_VERSION].as_slice(),
                    )
                    .map_err(|e| db_err(&self.path, "writing out-of-window schema", e))?;
                    for key in [
                        OUT_OF_WINDOW_RECORDS,
                        OUT_OF_WINDOW_BYTES,
                        OUT_OF_WINDOW_OVERFLOWED,
                    ] {
                        meta.insert(key, 0u64.to_be_bytes().as_slice())
                            .map_err(|e| db_err(&self.path, "writing out-of-window metadata", e))?;
                    }
                }
                None => {
                    return Err(db_err(
                        &self.path,
                        "opening the buffer",
                        "out-of-window table has no schema marker",
                    ));
                }
                Some([OUT_OF_WINDOW_RECORD_VERSION]) if had_out_of_window_table => {}
                Some([OUT_OF_WINDOW_RECORD_VERSION]) => {
                    return Err(db_err(
                        &self.path,
                        "opening the buffer",
                        "out-of-window schema marker has no table",
                    ));
                }
                Some(_) => {
                    return Err(db_err(
                        &self.path,
                        "opening the buffer",
                        "unsupported out-of-window schema marker",
                    ));
                }
            }
            drop(meta);

            let table = txn
                .open_table(OUT_OF_WINDOW)
                .map_err(|e| db_err(&self.path, "opening the out-of-window table", e))?;
            let meta = txn
                .open_table(META)
                .map_err(|e| db_err(&self.path, "opening the meta table", e))?;
            let stats = out_of_window_stats_from_meta(&meta, "open")?;
            validate_out_of_window_layout(&table, stats.records, "open")?;

            let mut has_progress = false;
            let mut has_transitions = false;
            for entry in txn
                .list_tables()
                .map_err(|e| db_err(&self.path, "listing batch progress tables", e))?
            {
                has_progress |= entry.name() == OUT_OF_WINDOW_PROGRESS.name();
                has_transitions |= entry.name() == OUT_OF_WINDOW_TRANSITIONS.name();
            }
            let batch_schema = meta
                .get(OUT_OF_WINDOW_BATCH_SCHEMA)
                .map_err(|e| db_err(&self.path, "reading batch progress schema", e))?
                .map(|row| row.value().to_vec());
            let batch_admission = meta
                .get(OUT_OF_WINDOW_BATCH_ADMISSION)
                .map_err(|e| db_err(&self.path, "reading batch admission", e))?
                .map(|row| row.value().to_vec());
            let state = classify_batch_journal(
                self,
                batch_schema.as_deref(),
                batch_admission.as_deref(),
                has_progress,
                has_transitions,
                "open",
            );
            match state {
                BatchJournalState::Absent => {}
                BatchJournalState::FullyPresent(admission) => {
                    drop(meta);
                    let progress = txn
                        .open_table(OUT_OF_WINDOW_PROGRESS)
                        .map_err(|e| db_err(&self.path, "opening batch progress table", e))?;
                    let transitions = txn
                        .open_table(OUT_OF_WINDOW_TRANSITIONS)
                        .map_err(|e| db_err(&self.path, "opening batch transition table", e))?;
                    validate_batch_layout(self, &progress, &transitions, &admission, "open")?;
                }
                BatchJournalState::Corrupt => {
                    return Err(db_err(
                        &self.path,
                        "opening the buffer",
                        "batch journal state is partial or corrupt",
                    ));
                }
            }
        }
        txn.commit()
            .map_err(|e| db_err(&self.path, "opening the buffer", e))
    }

    /// The first seq a read or a count covers, resolved inside the caller's read
    /// transaction so the watermark and the rows it bounds come from one snapshot.
    ///
    /// `None` in means "from the watermark": `floor() + 1`, or zero when nothing has been
    /// acknowledged — the stream starts at zero, so a bare `0` could not mean "the
    /// beginning" without skipping the genesis envelope. `Some(u64::MAX)` names a position
    /// nothing can follow, so the answer is "no rows" rather than a wrapped scan.
    fn start_seq(
        &self,
        txn: &ReadTransaction,
        after_seq: Option<u64>,
        op: &str,
    ) -> Result<Option<u64>, DaemonError> {
        let after = match after_seq {
            Some(n) => Some(n),
            None => floor_in(txn, op)?,
        };
        Ok(match after {
            None => Some(0),
            Some(n) => n.checked_add(1),
        })
    }

    /// The encrypting half, built on first use. See the field's documentation for why it is
    /// neither built eagerly nor rebuilt per record.
    fn recipient(&self) -> &age::scrypt::Recipient {
        self.recipient.get_or_init(|| {
            let mut recipient = age::scrypt::Recipient::new(self.passphrase.clone());
            recipient.set_work_factor(SCRYPT_LOG_N);
            recipient
        })
    }

    /// The decrypting half, built on first use.
    fn identity(&self) -> &age::scrypt::Identity {
        self.identity.get_or_init(|| {
            let mut identity = age::scrypt::Identity::new(self.passphrase.clone());
            identity.set_max_work_factor(SCRYPT_LOG_N);
            identity
        })
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, DaemonError> {
        age::encrypt(self.recipient(), plain)
            .map_err(|e| self.crypto(format!("a record could not be encrypted: {e}")))
    }

    fn unseal(&self, sealed: &[u8]) -> Result<Envelope, DaemonError> {
        Ok(serde_json::from_slice(&self.unseal_bytes(sealed)?)?)
    }

    fn unseal_bytes(&self, sealed: &[u8]) -> Result<Vec<u8>, DaemonError> {
        age::decrypt(self.identity(), sealed)
            .map_err(|e| self.crypto(format!("a record could not be decrypted: {e}")))
    }

    /// The one message a caller whose key did not open this buffer should read. It names the
    /// file and says nothing about the key: the passphrase is `RunKeys.buffer_key_hex`, and
    /// a secret that reaches an error message reaches every log that error is written to.
    ///
    /// It hedges, because a failed unseal genuinely cannot tell the two apart: a wrong key
    /// and a damaged key-check row both come back as "this did not decrypt". Naming only the
    /// key would send an operator looking for a mix-up that never happened while the real
    /// answer — the file is corrupt — went unsaid. The advice is the same either way, and
    /// saying which is unknown is cheaper than saying the wrong one.
    fn wrong_key(&self) -> DaemonError {
        self.crypto(
            "this buffer did not open under the key it was given — either a wrong run key \
             or a corrupted buffer, and nothing here can tell those apart. Opening it under \
             another run's key would start a second chain over the first one's file"
                .to_string(),
        )
    }

    fn crypto(&self, what: String) -> DaemonError {
        DaemonError::Crypto(format!("buffer {}: {what}", self.path.display()))
    }

    /// Derive the opaque progress-store identity from the caller's stable source coordinate.
    /// The coordinate is length-framed inside a domain-separated, run-keyed HMAC so neither
    /// a raw pathname nor an unkeyed digest becomes a clear redb key.
    pub(crate) fn derive_out_of_window_store_key(
        &self,
        source_coordinate: &[u8],
    ) -> Result<OutOfWindowStoreKeyV1, DaemonError> {
        if source_coordinate.is_empty() {
            return Err(DaemonError::Refused(
                "out-of-window source coordinate is empty".to_string(),
            ));
        }
        if source_coordinate.len() > OUT_OF_WINDOW_MAX_SOURCE_COORDINATE_BYTES {
            return Err(DaemonError::Refused(
                "out-of-window source coordinate exceeds the bounded frame size".to_string(),
            ));
        }
        let mut message = Vec::with_capacity(8 + source_coordinate.len() + 32);
        message.extend_from_slice(b"goalcup/out-of-window-store/v1\0");
        message.extend_from_slice(
            &u64::try_from(source_coordinate.len())
                .expect("bounded source coordinate length fits u64")
                .to_be_bytes(),
        );
        message.extend_from_slice(source_coordinate);
        Ok(OutOfWindowStoreKeyV1(hmac_sha256(
            self.passphrase.expose_secret().as_bytes(),
            &message,
        )))
    }

    fn batch_transition_key(&self, store_key: &OutOfWindowStoreKeyV1, before: &[u8]) -> [u8; 32] {
        let mut message = Vec::with_capacity(32 + 8 + before.len() + 32);
        message.extend_from_slice(b"goalcup/out-of-window-transition/v1\0");
        message.extend_from_slice(&store_key.0);
        message.extend_from_slice(
            &u64::try_from(before.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        message.extend_from_slice(before);
        hmac_sha256(self.passphrase.expose_secret().as_bytes(), &message)
    }
}

/// Open the encrypted database through an owned artifact-directory handle on Unix.  A
/// pathname `OpenOptions` call would be exposed to a rename of the parent after
/// validation, which could redirect the open.  Holding the parent fd and using `openat`
/// makes the leaf and its storage root one acquisition boundary.
#[cfg(unix)]
fn open_private_buffer_file(path: &Path) -> Result<std::fs::File, DaemonError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let parent_path = path.parent().ok_or_else(|| {
        DaemonError::Refused(format!(
            "buffer path {} has no artifact parent",
            path.display()
        ))
    })?;
    let leaf = path.file_name().ok_or_else(|| {
        DaemonError::Refused(format!("buffer path {} has no file name", path.display()))
    })?;
    let parent = CString::new(parent_path.as_os_str().as_bytes()).map_err(|_| {
        DaemonError::Refused(format!(
            "buffer artifact parent {} contains a NUL byte",
            parent_path.display()
        ))
    })?;
    let leaf = CString::new(leaf.as_bytes()).map_err(|_| {
        DaemonError::Refused(format!(
            "buffer path {} contains a NUL byte",
            path.display()
        ))
    })?;
    let directory = unsafe {
        // SAFETY: both C strings are NUL-terminated and remain live for these calls.
        libc::open(
            parent.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if directory < 0 {
        return Err(DaemonError::Io {
            path: parent_path.display().to_string(),
            source: std::io::Error::last_os_error(),
        });
    }
    let directory = unsafe {
        // SAFETY: a successful `open` returns one owned descriptor.
        OwnedFd::from_raw_fd(directory)
    };
    let raw = unsafe {
        // SAFETY: `directory` is an owned directory capability and `leaf` is one basename.
        libc::openat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if raw < 0 {
        return Err(DaemonError::Io {
            path: path.display().to_string(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(unsafe {
        // SAFETY: a successful `openat` returns one owned descriptor.
        std::fs::File::from_raw_fd(raw)
    })
}

#[cfg(not(unix))]
fn open_private_buffer_file(path: &Path) -> Result<std::fs::File, DaemonError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    opts.open(path).map_err(|source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// The run key as an `age` passphrase: 64 hex characters, normalised to lower case.
///
/// Normalised because the passphrase must be a function of the key **bytes** and not of
/// their spelling — a buffer written under `AB…` and opened under `ab…` is the same run,
/// and a daemon that treated those as two keys would fail closed on nothing at all.
///
/// The message describes the shape and never the value, the way `RunKeys::bad_seed` does.
///
/// The decoded bytes are **wiped before this returns**, on the error paths as well as the
/// success one. `SecretString` zeroises what it holds when it drops; the `Vec` this function
/// decoded into does not, and a freed heap allocation still holding 32 bytes of run key is
/// exactly the thing the buffer's encryption is meant to keep off the machine. It is a
/// narrow window — one function — but it is a free one to close.
fn passphrase(run_key_hex: &str) -> Result<SecretString, DaemonError> {
    let mut raw = hex::decode(run_key_hex).map_err(|_| bad_key())?;
    let out = match raw.len() {
        32 => Ok(SecretString::from(hex::encode(&raw))),
        _ => Err(bad_key()),
    };
    wipe(&mut raw);
    out
}

/// Overwrite a buffer of key material with zeroes, and make the compiler keep the writes.
///
/// Hand-written rather than pulled from `zeroize`: the crate is in the tree under `age`, but
/// reaching through another crate's dependency for four lines is a dependency this module
/// would then owe an explanation for. `black_box` is what does the work — the plain loop is
/// a dead store to a value about to be dropped, which an optimiser is entitled to delete,
/// and `black_box` makes it something the compiler must assume is read.
fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        *b = 0;
    }
    std::hint::black_box(bytes);
}

fn bad_key() -> DaemonError {
    DaemonError::Crypto("the buffer key is not 64 hex characters decoding to 32 bytes".to_string())
}

/// One `statvfs` field, widened to `u64`.
///
/// Written generically because the widths are the platform's, not this crate's:
/// `fsblkcnt_t` is 32 bits on Darwin and 64 on Linux, and `f_frsize` is a `c_ulong` that
/// follows the pointer width. A concrete `u64::from` would be a no-op conversion on one
/// target and the necessary one on another, and an `as` cast would silently truncate the
/// day a field grows. `Into` makes the compiler pick, and it will not compile at all if a
/// field ever stops fitting.
#[cfg(unix)]
fn widen(n: impl Into<u64>) -> u64 {
    n.into()
}

/// The acknowledgement watermark inside an open read transaction.
///
/// A row that is there but is not eight bytes is a `Db` error and not a `None`: "nothing was
/// ever acknowledged" and "the record of what was acknowledged is unreadable" lead to
/// opposite behaviour, and guessing the first one would re-offer an acknowledged range to a
/// server that refuses it permanently.
fn floor_in(txn: &ReadTransaction, op: &str) -> Result<Option<u64>, DaemonError> {
    let table = txn.open_table(META).map_err(|e| plain(e, op))?;
    let Some(row) = table.get(LAST_ACKED).map_err(|e| plain(e, op))? else {
        return Ok(None);
    };
    let bytes: [u8; 8] = row.value().try_into().map_err(|_| {
        DaemonError::Db(format!(
            "buffer {op}: the acknowledgement watermark is {} bytes and should be eight",
            row.value().len()
        ))
    })?;
    Ok(Some(u64::from_be_bytes(bytes)))
}

fn encode_out_of_window(bytes: &[u8]) -> Result<Vec<u8>, DaemonError> {
    let len = u64::try_from(bytes.len()).map_err(|_| {
        DaemonError::Refused("out-of-window record length is not representable".to_string())
    })?;
    let capacity = OUT_OF_WINDOW_HEADER_BYTES
        .checked_add(bytes.len())
        .ok_or_else(|| DaemonError::Refused("out-of-window record is too large".to_string()))?;
    let mut encoded = Vec::with_capacity(capacity);
    encoded.push(OUT_OF_WINDOW_RECORD_VERSION);
    encoded.extend_from_slice(&len.to_be_bytes());
    encoded.extend_from_slice(bytes);
    Ok(encoded)
}

fn decode_out_of_window(encoded: &[u8]) -> Result<u64, DaemonError> {
    let Some((&version, rest)) = encoded.split_first() else {
        return Err(DaemonError::Refused(
            "out-of-window record is empty".to_string(),
        ));
    };
    if version != OUT_OF_WINDOW_RECORD_VERSION {
        return Err(DaemonError::Refused(format!(
            "unsupported out-of-window record version {version}"
        )));
    }
    let (len, bytes) = rest.split_at_checked(8).ok_or_else(|| {
        DaemonError::Refused("out-of-window record has a truncated length".to_string())
    })?;
    let declared = u64::from_be_bytes(len.try_into().expect("split produced eight bytes"));
    let actual = u64::try_from(bytes.len()).map_err(|_| {
        DaemonError::Refused("out-of-window record length is not representable".to_string())
    })?;
    if declared != actual {
        return Err(DaemonError::Refused(format!(
            "out-of-window record declares {declared} bytes but contains {actual}"
        )));
    }
    Ok(actual)
}

fn out_of_window_stats_from_meta(
    meta: &impl ReadableTable<&'static str, &'static [u8]>,
    op: &str,
) -> Result<OutOfWindowStatsV1, DaemonError> {
    Ok(OutOfWindowStatsV1 {
        records: counter(meta, OUT_OF_WINDOW_RECORDS, op)?,
        bytes: counter(meta, OUT_OF_WINDOW_BYTES, op)?,
        overflowed: counter(meta, OUT_OF_WINDOW_OVERFLOWED, op)?,
    })
}

fn counter(
    meta: &impl ReadableTable<&'static str, &'static [u8]>,
    key: &str,
    op: &str,
) -> Result<u64, DaemonError> {
    let row = meta
        .get(key)
        .map_err(|e| plain(e, op))?
        .ok_or_else(|| DaemonError::Db(format!("buffer {op}: missing {key} counter")))?;
    let bytes: [u8; 8] = row.value().try_into().map_err(|_| {
        DaemonError::Db(format!(
            "buffer {op}: {key} counter is {} bytes and should be eight",
            row.value().len()
        ))
    })?;
    Ok(u64::from_be_bytes(bytes))
}

fn put_counter(
    meta: &mut redb::Table<'_, &'static str, &'static [u8]>,
    key: &str,
    value: u64,
) -> Result<(), DaemonError> {
    meta.insert(key, value.to_be_bytes().as_slice())
        .map_err(|e| plain(e, "append_out_of_window"))?;
    Ok(())
}

fn validate_out_of_window_layout(
    table: &impl ReadableTable<u64, &'static [u8]>,
    records: u64,
    op: &str,
) -> Result<(), DaemonError> {
    let actual = table.len().map_err(|e| plain(e, op))?;
    if actual != records {
        return Err(DaemonError::Db(format!(
            "buffer {op}: out-of-window table has {actual} rows but records counter is {records}"
        )));
    }
    let expected_last = records.checked_sub(1);
    let actual_last = table
        .last()
        .map_err(|e| plain(e, op))?
        .map(|(key, _)| key.value());
    if actual_last != expected_last {
        return Err(DaemonError::Db(format!(
            "buffer {op}: out-of-window arrival ordinals are not contiguous"
        )));
    }
    Ok(())
}

fn validate_cursor_bytes(cursor: &[u8], which: &str) -> Result<(), DaemonError> {
    if cursor.is_empty() {
        return Err(DaemonError::Refused(format!(
            "out-of-window {which} cursor is empty"
        )));
    }
    if cursor.len() > OUT_OF_WINDOW_MAX_CURSOR_BYTES {
        return Err(DaemonError::Refused(format!(
            "out-of-window {which} cursor exceeds the bounded frame size"
        )));
    }
    Ok(())
}

fn encode_frame<T: Serialize>(frame: &T) -> Result<Vec<u8>, DaemonError> {
    serde_json::to_vec(frame).map_err(|error| {
        DaemonError::Crypto(format!(
            "out-of-window batch frame could not be encoded: {error}"
        ))
    })
}

fn decode_frame<T: for<'de> Deserialize<'de>>(bytes: &[u8], op: &str) -> Result<T, DaemonError> {
    serde_json::from_slice(bytes).map_err(|error| {
        DaemonError::Crypto(format!(
            "buffer {op}: invalid encrypted batch frame: {error}"
        ))
    })
}

fn decode_progress(
    buffer: &Buffer,
    sealed: &[u8],
    op: &str,
) -> Result<BatchProgressFrame, DaemonError> {
    let frame: BatchProgressFrame = decode_frame(&buffer.unseal_bytes(sealed)?, op)?;
    if frame.version != OUT_OF_WINDOW_BATCH_FRAME_VERSION
        || frame.cursor.is_empty()
        || frame.cursor.len() > OUT_OF_WINDOW_MAX_CURSOR_BYTES
    {
        return Err(DaemonError::Crypto(format!(
            "buffer {op}: unsupported or malformed encrypted batch progress frame"
        )));
    }
    Ok(frame)
}

fn decode_transition(
    buffer: &Buffer,
    sealed: &[u8],
    op: &str,
) -> Result<BatchTransitionFrame, DaemonError> {
    let frame: BatchTransitionFrame = decode_frame(&buffer.unseal_bytes(sealed)?, op)?;
    if frame.version != OUT_OF_WINDOW_BATCH_FRAME_VERSION
        || frame.before.is_empty()
        || frame.after.is_empty()
        || frame.before.len() > OUT_OF_WINDOW_MAX_CURSOR_BYTES
        || frame.after.len() > OUT_OF_WINDOW_MAX_CURSOR_BYTES
    {
        return Err(DaemonError::Crypto(format!(
            "buffer {op}: unsupported or malformed encrypted batch transition frame"
        )));
    }
    Ok(frame)
}

fn decode_admission(
    buffer: &Buffer,
    sealed: &[u8],
    op: &str,
) -> Result<BatchAdmissionFrame, DaemonError> {
    let frame: BatchAdmissionFrame = decode_frame(&buffer.unseal_bytes(sealed)?, op)?;
    if frame.version != OUT_OF_WINDOW_BATCH_FRAME_VERSION
        || frame.stores.is_empty()
        || frame.max_transitions_per_store == 0
        || frame.stores.windows(2).any(|window| {
            window[0].store_key >= window[1].store_key
                || validate_cursor_bytes(&window[0].cursor, "admission").is_err()
        })
        || validate_cursor_bytes(
            &frame.stores.last().expect("non-empty admission").cursor,
            "admission",
        )
        .is_err()
    {
        return Err(DaemonError::Crypto(format!(
            "buffer {op}: malformed encrypted batch admission"
        )));
    }
    Ok(frame)
}

fn batch_digest(records: &[Vec<u8>]) -> Result<BatchDigest, DaemonError> {
    let record_count = u64::try_from(records.len()).map_err(|_| {
        DaemonError::Refused("out-of-window batch record count is not representable".to_string())
    })?;
    let mut input_bytes = 0u64;
    let mut hasher = Sha256::new();
    hasher.update(b"goalcup/out-of-window-batch/v1\0");
    hasher.update(record_count.to_be_bytes());
    for record in records {
        let length = u64::try_from(record.len()).map_err(|_| {
            DaemonError::Refused("out-of-window record length is not representable".to_string())
        })?;
        input_bytes = input_bytes.checked_add(length).ok_or_else(|| {
            DaemonError::Refused("out-of-window batch byte count overflowed".to_string())
        })?;
        hasher.update(length.to_be_bytes());
        hasher.update(record);
    }
    Ok(BatchDigest {
        record_count,
        input_bytes,
        digest: hasher.finalize().into(),
    })
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block_key = [0u8; 64];
    if key.len() > block_key.len() {
        block_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; 64];
    let mut outer_pad = [0x5cu8; 64];
    for (index, byte) in block_key.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn combine_validation(
    previous: OutOfWindowBatchValidationV1,
    current: OutOfWindowBatchValidationV1,
) -> OutOfWindowBatchValidationV1 {
    let flags = previous.flags | current.flags;
    let reason_digest = if current.flags == 0 {
        previous.reason_digest
    } else if previous.flags == 0 {
        current.reason_digest
    } else {
        let mut hasher = Sha256::new();
        hasher.update(b"goalcup/out-of-window-validation/v1\0");
        hasher.update(previous.reason_digest);
        hasher.update(current.reason_digest);
        hasher.finalize().into()
    };
    OutOfWindowBatchValidationV1 {
        flags,
        reason_digest,
    }
}

enum BatchJournalState {
    Absent,
    FullyPresent(BatchAdmissionFrame),
    Corrupt,
}

fn classify_batch_journal(
    buffer: &Buffer,
    schema: Option<&[u8]>,
    admission: Option<&[u8]>,
    has_progress: bool,
    has_transitions: bool,
    op: &str,
) -> BatchJournalState {
    if schema.is_none() && admission.is_none() && !has_progress && !has_transitions {
        return BatchJournalState::Absent;
    }
    if !has_progress
        || !has_transitions
        || !matches!(schema, Some([OUT_OF_WINDOW_BATCH_SCHEMA_VERSION]))
    {
        return BatchJournalState::Corrupt;
    }
    match admission {
        Some(sealed) => match decode_admission(buffer, sealed, op) {
            Ok(admission) => BatchJournalState::FullyPresent(admission),
            Err(_) => BatchJournalState::Corrupt,
        },
        None => BatchJournalState::Corrupt,
    }
}

fn validate_batch_layout(
    buffer: &Buffer,
    progress: &impl ReadableTable<[u8; 32], &'static [u8]>,
    transitions: &impl ReadableTable<[u8; 32], &'static [u8]>,
    admission: &BatchAdmissionFrame,
    op: &str,
) -> Result<(), DaemonError> {
    let expected: std::collections::BTreeSet<[u8; 32]> = admission
        .stores
        .iter()
        .map(|store| store.store_key)
        .collect();
    let mut progress_frames = std::collections::BTreeMap::new();
    for row in progress.iter().map_err(|e| plain(e, op))? {
        let (key, sealed) = row.map_err(|e| plain(e, op))?;
        let frame = decode_progress(buffer, sealed.value(), op)?;
        if key.value() != frame.store_key {
            return Err(DaemonError::Db(format!(
                "buffer {op}: batch progress key binding failed"
            )));
        }
        if !expected.contains(&frame.store_key)
            || (frame.transition_count == 0) != frame.head.is_none()
        {
            return Err(DaemonError::Db(format!(
                "buffer {op}: batch progress admission/count binding failed"
            )));
        }
        if progress_frames.insert(frame.store_key, frame).is_some() {
            return Err(DaemonError::Db(format!(
                "buffer {op}: duplicate batch progress key"
            )));
        }
    }

    if progress_frames
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        != expected
    {
        return Err(DaemonError::Db(format!(
            "buffer {op}: admitted source progress rows are incomplete"
        )));
    }

    let mut transition_frames = std::collections::BTreeMap::new();
    for row in transitions.iter().map_err(|e| plain(e, op))? {
        let (key, sealed) = row.map_err(|e| plain(e, op))?;
        let frame = decode_transition(buffer, sealed.value(), op)?;
        let derived =
            buffer.batch_transition_key(&OutOfWindowStoreKeyV1(frame.store_key), &frame.before);
        if key.value() != derived {
            return Err(DaemonError::Db(format!(
                "buffer {op}: batch transition key binding failed"
            )));
        }
        if !expected.contains(&frame.store_key) {
            return Err(DaemonError::Db(format!(
                "buffer {op}: transition references an unadmitted source"
            )));
        }
        if frame.record_count == 0 && frame.input_bytes != 0 {
            return Err(DaemonError::Db(format!(
                "buffer {op}: empty batch byte binding failed"
            )));
        }
        if transition_frames.insert(key.value(), frame).is_some() {
            return Err(DaemonError::Db(format!(
                "buffer {op}: duplicate batch transition key"
            )));
        }
    }

    let mut visited = std::collections::BTreeSet::new();
    for (store_key, progress) in &progress_frames {
        let admission_cursor = admission
            .stores
            .iter()
            .find(|store| store.store_key == *store_key)
            .expect("progress keys are admitted")
            .cursor
            .as_slice();
        if progress.transition_count == 0 {
            if progress.cursor != admission_cursor {
                return Err(DaemonError::Db(format!(
                    "buffer {op}: initial progress cursor binding failed"
                )));
            }
            if progress.validation != OutOfWindowBatchValidationV1::CLEAN {
                return Err(DaemonError::Db(format!(
                    "buffer {op}: initial progress validation is not clean"
                )));
            }
            continue;
        }
        if progress.transition_count > admission.max_transitions_per_store {
            return Err(DaemonError::Db(format!(
                "buffer {op}: batch transition count exceeds admission cap"
            )));
        }
        let mut head = progress.head;
        let mut cursor = progress.cursor.clone();
        let mut chain = Vec::new();
        while let Some(key) = head {
            if !visited.insert(key) {
                return Err(DaemonError::Db(format!(
                    "buffer {op}: batch transition chain contains a cycle"
                )));
            }
            let frame = transition_frames.get(&key).ok_or_else(|| {
                DaemonError::Db(format!("buffer {op}: progress head has no transition"))
            })?;
            if frame.store_key != *store_key || frame.after != cursor {
                return Err(DaemonError::Db(format!(
                    "buffer {op}: batch transition link binding failed"
                )));
            }
            chain.push(frame);
            cursor = frame.before.clone();
            head = frame.previous_head;
            if u64::try_from(chain.len()).unwrap_or(u64::MAX) > progress.transition_count {
                return Err(DaemonError::Db(format!(
                    "buffer {op}: batch transition count is inconsistent"
                )));
            }
        }
        if u64::try_from(chain.len()).unwrap_or(u64::MAX) != progress.transition_count {
            return Err(DaemonError::Db(format!(
                "buffer {op}: batch transition chain is incomplete"
            )));
        }
        if cursor != admission_cursor {
            return Err(DaemonError::Db(format!(
                "buffer {op}: oldest transition does not start at admission cursor"
            )));
        }
        let mut aggregate = OutOfWindowBatchValidationV1::CLEAN;
        for frame in chain.into_iter().rev() {
            aggregate = combine_validation(aggregate, frame.validation);
        }
        if aggregate != progress.validation {
            return Err(DaemonError::Db(format!(
                "buffer {op}: monotone validation state is inconsistent"
            )));
        }
    }
    if visited.len() != transition_frames.len() {
        return Err(DaemonError::Db(format!(
            "buffer {op}: batch transition journal contains unreachable rows"
        )));
    }
    Ok(())
}

/// A redb failure that happened while touching a known file, so the message carries the
/// path the way every `std::fs` error in this crate does.
fn db_err(path: &Path, what: &str, e: impl std::fmt::Display) -> DaemonError {
    DaemonError::Db(format!("buffer {}: {what}: {e}", path.display()))
}

/// A redb failure inside an already-open buffer, named by the operation that hit it. The
/// path is not repeated: an entry runs one buffer, and the operation is the part the
/// reader does not already know.
fn plain(e: impl std::fmt::Display, op: &str) -> DaemonError {
    DaemonError::Db(format!("buffer {op}: {e}"))
}

#[cfg(test)]
mod out_of_window_tests {
    use super::*;

    fn limits(records: u64, record_bytes: u64, total_bytes: u64) -> OutOfWindowLimitsV1 {
        OutOfWindowLimitsV1 {
            max_records: records,
            max_record_bytes: record_bytes,
            max_total_bytes: total_bytes,
        }
    }

    fn open() -> (tempfile::TempDir, PathBuf, String, Buffer) {
        let dir = tempfile::tempdir().expect("temporary artifact directory");
        let path = dir.path().join("buffer.redb");
        let key = "71".repeat(32);
        let buffer = Buffer::open(&path, &key).expect("buffer opens");
        (dir, path, key, buffer)
    }

    #[test]
    fn out_of_window_plaintext_is_encrypted_at_rest() {
        let (_dir, path, _key, buffer) = open();
        let marker = br#"{"redacted":"late-marker-4fddaf"}"#;
        retain(&buffer, marker, limits(2, 128, 256));

        let raw = std::fs::read(path).unwrap();
        let raw = String::from_utf8_lossy(&raw);
        assert!(raw.contains("age-encryption.org/v1"));
        assert!(!raw.contains("late-marker-4fddaf"));
    }

    #[test]
    fn out_of_window_rows_never_enter_normal_reads_or_ack_deletion() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(2, 128, 256);
        retain(&buffer, br#"{"late":true}"#, cap);

        assert!(buffer.read_from(None, 16).unwrap().envelopes.is_empty());
        assert_eq!(buffer.pending(None).unwrap(), 0);
        assert_eq!(buffer.ack(9).unwrap(), 0, "ACK touches ENVELOPES only");
        assert_eq!(
            buffer.verify_out_of_window(cap).unwrap(),
            OutOfWindowStatsV1 {
                records: 1,
                bytes: 13,
                overflowed: 0,
            },
            "late evidence survives ACK"
        );
    }

    #[test]
    fn total_byte_cap_is_independent_of_record_and_per_record_caps() {
        let (_dir, path, key, buffer) = open();
        let cap = limits(4, 4, 6);
        let records = batch(&[b"1234", b"5678"]);
        assert_eq!(
            apply_clean(&buffer, store_key(1), b"A", b"B", &records, cap, true).unwrap(),
            OutOfWindowBatchApplyV1::Applied {
                stats: OutOfWindowStatsV1 {
                    records: 1,
                    bytes: 4,
                    overflowed: 1,
                },
                overflowed: 1,
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );
        drop(buffer);
        assert_eq!(
            Buffer::open(&path, &key)
                .unwrap()
                .verify_out_of_window(cap)
                .unwrap()
                .overflowed,
            1,
            "total-byte overflow survives reopen"
        );
    }

    #[test]
    fn record_count_cap_is_independent_of_record_and_total_byte_caps() {
        let (_dir, path, key, buffer) = open();
        let cap = limits(2, 64, 1_024);
        let records = batch(&[b"a", b"b", b"c"]);
        assert_eq!(
            apply_clean(&buffer, store_key(1), b"A", b"B", &records, cap, true).unwrap(),
            OutOfWindowBatchApplyV1::Applied {
                stats: OutOfWindowStatsV1 {
                    records: 2,
                    bytes: 2,
                    overflowed: 1,
                },
                overflowed: 1,
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );
        drop(buffer);
        assert_eq!(
            Buffer::open(&path, &key)
                .unwrap()
                .verify_out_of_window(cap)
                .unwrap(),
            OutOfWindowStatsV1 {
                records: 2,
                bytes: 2,
                overflowed: 1,
            },
            "record-count overflow survives reopen"
        );
    }

    #[test]
    fn out_of_window_corruption_fails_closed_without_exposing_bytes() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(2, 128, 256);
        retain(&buffer, b"redacted", cap);

        let txn = buffer.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(OUT_OF_WINDOW).unwrap();
            table.insert(0, b"not-an-age-record".as_slice()).unwrap();
        }
        txn.commit().unwrap();

        let error = buffer
            .verify_out_of_window(cap)
            .expect_err("corrupt retained evidence cannot be treated as clean");
        assert!(matches!(error, DaemonError::Crypto(_)), "got {error:?}");
    }

    #[test]
    fn corrupt_out_of_window_accounting_refuses_a_later_batch() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(2, 128, 256);
        retain(&buffer, b"redacted", cap);

        let txn = buffer.db.begin_write().unwrap();
        {
            let mut meta = txn.open_table(META).unwrap();
            meta.insert(OUT_OF_WINDOW_BYTES, [1u8, 2, 3].as_slice())
                .unwrap();
        }
        txn.commit().unwrap();

        let next = batch(&[b"next"]);
        let error = apply_clean(&buffer, store_key(1), b"B", b"C", &next, cap, false)
            .expect_err("broken durable accounting cannot silently accept bytes");
        assert!(matches!(error, DaemonError::Db(_)), "got {error:?}");
    }

    #[test]
    fn reopening_never_recreates_missing_out_of_window_accounting() {
        let (_dir, path, key, buffer) = open();
        retain(&buffer, b"1234", limits(1, 4, 4));
        let txn = buffer.db.begin_write().unwrap();
        {
            let mut meta = txn.open_table(META).unwrap();
            assert!(meta.remove(OUT_OF_WINDOW_OVERFLOWED).unwrap().is_some());
        }
        txn.commit().unwrap();
        drop(buffer);

        let error = Buffer::open(&path, &key)
            .expect_err("a schema-marked store cannot forget overflow accounting");
        assert!(matches!(error, DaemonError::Db(_)), "got {error:?}");
    }

    #[test]
    fn reopening_never_recreates_a_missing_schema_marked_table() {
        let (_dir, path, key, buffer) = open();
        let txn = buffer.db.begin_write().unwrap();
        assert!(txn.delete_table(OUT_OF_WINDOW).unwrap());
        txn.commit().unwrap();
        drop(buffer);

        let error = Buffer::open(&path, &key)
            .expect_err("a schema marker is durable evidence that the table existed");
        assert!(matches!(error, DaemonError::Db(_)), "got {error:?}");
    }

    #[test]
    fn a_demonstrably_legacy_buffer_migrates_once_to_the_private_table() {
        let (_dir, path, key, buffer) = open();
        let txn = buffer.db.begin_write().unwrap();
        assert!(txn.delete_table(OUT_OF_WINDOW).unwrap());
        {
            let mut meta = txn.open_table(META).unwrap();
            for key in [
                OUT_OF_WINDOW_SCHEMA,
                OUT_OF_WINDOW_RECORDS,
                OUT_OF_WINDOW_BYTES,
                OUT_OF_WINDOW_OVERFLOWED,
            ] {
                assert!(meta.remove(key).unwrap().is_some());
            }
        }
        txn.commit().unwrap();
        drop(buffer);

        let migrated = Buffer::open(&path, &key).expect("no table and no state is legacy");
        assert_eq!(
            migrated.verify_out_of_window(limits(1, 4, 4)).unwrap(),
            OutOfWindowStatsV1 {
                records: 0,
                bytes: 0,
                overflowed: 0,
            }
        );
    }

    fn batch(records: &[&[u8]]) -> Vec<Vec<u8>> {
        records.iter().map(|record| record.to_vec()).collect()
    }

    /// Retains one record through the batch path, the only writer of out-of-window rows.
    fn retain(buffer: &Buffer, record: &[u8], cap: OutOfWindowLimitsV1) {
        let records = batch(&[record]);
        apply_clean(buffer, store_key(1), b"A", b"B", &records, cap, true).unwrap();
    }

    fn store_key(byte: u8) -> OutOfWindowStoreKeyV1 {
        OutOfWindowStoreKeyV1([byte; 32])
    }

    fn apply_clean(
        buffer: &Buffer,
        store: OutOfWindowStoreKeyV1,
        before: &[u8],
        after: &[u8],
        records: &[Vec<u8>],
        limits: OutOfWindowLimitsV1,
        initialize: bool,
    ) -> Result<OutOfWindowBatchApplyV1, DaemonError> {
        if initialize {
            buffer.initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store, before.to_vec())],
                max_transitions_per_store: limits.max_records.max(1),
            })?;
        }
        buffer.apply_out_of_window_batch_with_validation(
            store,
            before,
            after,
            records,
            OutOfWindowBatchValidationV1::CLEAN,
            limits,
        )
    }

    #[test]
    fn store_key_derivation_is_bound_to_the_buffer_run_key() {
        let (_dir_a, _path_a, _key_a, buffer_a) = open();
        let dir_b = tempfile::tempdir().unwrap();
        let path_b = dir_b.path().join("buffer.redb");
        let buffer_b = Buffer::open(&path_b, &"72".repeat(32)).unwrap();

        assert_ne!(
            buffer_a
                .derive_out_of_window_store_key(b"same-source-coordinate")
                .unwrap(),
            buffer_b
                .derive_out_of_window_store_key(b"same-source-coordinate")
                .unwrap(),
            "the same coordinate must not share a clear store id across runs"
        );
    }

    #[test]
    fn current_cursor_returns_the_admitted_root_for_empty_history() {
        let (_dir, _path, _key, buffer) = open();
        let coordinate = b"empty-history-source";
        let store = buffer.derive_out_of_window_store_key(coordinate).unwrap();
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store, b"A".to_vec())],
                max_transitions_per_store: 2,
            })
            .unwrap();

        let mut cursor = buffer.out_of_window_current_cursor(coordinate).unwrap();
        assert_eq!(cursor, b"A");
        cursor[0] = b"B"[0];
        assert_eq!(
            buffer.out_of_window_current_cursor(coordinate).unwrap(),
            b"A"
        );
    }

    #[test]
    fn current_cursor_restores_a_committed_head_before_a_later_batch() {
        let (_dir, path, key, buffer) = open();
        let coordinate = b"restartable-source";
        let store = buffer.derive_out_of_window_store_key(coordinate).unwrap();
        let limits = limits(4, 64, 128);
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store, b"A".to_vec())],
                max_transitions_per_store: 3,
            })
            .unwrap();
        buffer
            .apply_out_of_window_batch_with_validation(
                store,
                b"A",
                b"C",
                &[],
                OutOfWindowBatchValidationV1::CLEAN,
                limits,
            )
            .unwrap();
        drop(buffer);

        let reopened = Buffer::open(&path, &key).unwrap();
        assert_eq!(
            reopened.out_of_window_current_cursor(coordinate).unwrap(),
            b"C"
        );
        let store = reopened.derive_out_of_window_store_key(coordinate).unwrap();
        let error = reopened
            .apply_out_of_window_batch_with_validation(
                store,
                b"A",
                b"D",
                &[],
                OutOfWindowBatchValidationV1::CLEAN,
                limits,
            )
            .expect_err("recovery must not skip the persisted C head");
        assert!(matches!(error, DaemonError::Refused(_)), "got {error:?}");
        reopened
            .apply_out_of_window_batch_with_validation(
                store,
                b"C",
                b"D",
                &[],
                OutOfWindowBatchValidationV1::CLEAN,
                limits,
            )
            .unwrap();
        assert_eq!(
            reopened.out_of_window_current_cursor(coordinate).unwrap(),
            b"D"
        );
    }

    #[test]
    fn current_cursor_rejects_unadmitted_coordinates_and_corrupt_journals() {
        let (_dir, _path, _key, buffer) = open();
        let coordinate = b"admitted-source";
        let store = buffer.derive_out_of_window_store_key(coordinate).unwrap();
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store, b"A".to_vec())],
                max_transitions_per_store: 2,
            })
            .unwrap();
        let error = buffer
            .out_of_window_current_cursor(b"other-source")
            .expect_err("an unadmitted source cannot read another source's cursor");
        assert!(matches!(error, DaemonError::Refused(_)), "got {error:?}");

        let txn = buffer.db.begin_write().unwrap();
        txn.open_table(OUT_OF_WINDOW_PROGRESS)
            .unwrap()
            .insert(*store.as_bytes(), b"corrupt".as_slice())
            .unwrap();
        txn.commit().unwrap();
        let error = buffer
            .out_of_window_current_cursor(coordinate)
            .expect_err("a corrupt journal cannot provide a recovery cursor");
        assert!(
            matches!(error, DaemonError::Db(_) | DaemonError::Crypto(_)),
            "got {error:?}"
        );
    }

    #[test]
    fn current_cursor_isolated_by_run_key() {
        let (_dir_a, _path_a, _key_a, buffer_a) = open();
        let dir_b = tempfile::tempdir().unwrap();
        let path_b = dir_b.path().join("buffer.redb");
        let buffer_b = Buffer::open(&path_b, &"72".repeat(32)).unwrap();
        let coordinate = b"same-source";
        let store_a = buffer_a.derive_out_of_window_store_key(coordinate).unwrap();
        let store_b = buffer_b.derive_out_of_window_store_key(coordinate).unwrap();
        buffer_a
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store_a, b"A".to_vec())],
                max_transitions_per_store: 1,
            })
            .unwrap();
        buffer_b
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store_b, b"B".to_vec())],
                max_transitions_per_store: 1,
            })
            .unwrap();

        assert_eq!(
            buffer_a.out_of_window_current_cursor(coordinate).unwrap(),
            b"A"
        );
        assert_eq!(
            buffer_b.out_of_window_current_cursor(coordinate).unwrap(),
            b"B"
        );
    }

    #[test]
    fn store_key_derivation_length_frames_source_coordinates() {
        let (_dir, _path, _key, buffer) = open();
        let first = buffer.derive_out_of_window_store_key(b"ab").unwrap();
        let second = buffer.derive_out_of_window_store_key(b"a\0b").unwrap();
        assert_ne!(
            first, second,
            "different length-framed coordinates must have distinct store ids"
        );
    }

    #[test]
    fn progress_keys_are_keyed_and_do_not_expose_source_or_unkeyed_hash() {
        let (_dir, path, _key, buffer) = open();
        let coordinate = b"/workspace/participant/source.jsonl";
        let store = buffer.derive_out_of_window_store_key(coordinate).unwrap();
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store, b"cursor:0".to_vec())],
                max_transitions_per_store: 1,
            })
            .unwrap();

        let txn = buffer.db.begin_read().unwrap();
        let progress = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
        let keys = progress
            .iter()
            .unwrap()
            .map(|row| row.unwrap().0.value())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![*store.as_bytes()]);
        let unkeyed_hash: [u8; 32] = Sha256::digest(coordinate).into();
        assert_ne!(keys[0], unkeyed_hash);
        assert!(!keys.iter().any(|key| key.starts_with(coordinate)));
        assert!(
            !std::fs::read(path)
                .unwrap()
                .windows(coordinate.len())
                .any(|window| window == coordinate)
        );
    }

    #[test]
    fn raw_store_id_injection_is_unavailable_to_callers() {
        let (_dir, _path, _key, buffer) = open();
        let derived = buffer
            .derive_out_of_window_store_key(b"caller-coordinate")
            .unwrap();
        assert_ne!(derived.0, [0xabu8; 32]);
    }

    #[test]
    fn batch_progress_replay_returns_already_applied_before_tight_caps() {
        let (_dir, path, key, buffer) = open();
        let cap = limits(4, 64, 128);
        let records = batch(&[b"same"]);
        assert_eq!(
            apply_clean(
                &buffer,
                store_key(0x11),
                b"cursor:0",
                b"cursor:1",
                &records,
                cap,
                true,
            )
            .unwrap(),
            OutOfWindowBatchApplyV1::Applied {
                stats: OutOfWindowStatsV1 {
                    records: 1,
                    bytes: 4,
                    overflowed: 0,
                },
                overflowed: 0,
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );

        drop(buffer);
        let reopened = Buffer::open(&path, &key).unwrap();
        for _ in 0..20 {
            assert_eq!(
                apply_clean(
                    &reopened,
                    store_key(0x11),
                    b"cursor:0",
                    b"cursor:1",
                    &records,
                    limits(0, 0, 0),
                    false,
                )
                .unwrap(),
                OutOfWindowBatchApplyV1::AlreadyApplied {
                    stats: OutOfWindowStatsV1 {
                        records: 1,
                        bytes: 4,
                        overflowed: 0,
                    },
                    validation: OutOfWindowBatchValidationV1::CLEAN,
                }
            );
        }
        assert_eq!(reopened.verify_out_of_window(cap).unwrap().overflowed, 0);
    }

    #[test]
    fn earlier_batch_replay_survives_later_store_progress() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(4, 64, 128);
        let first = batch(&[b"first"]);
        let second = batch(&[b"second"]);
        apply_clean(&buffer, store_key(0x21), b"0", b"1", &first, cap, true).unwrap();
        apply_clean(&buffer, store_key(0x21), b"1", b"2", &second, cap, false).unwrap();

        assert_eq!(
            apply_clean(
                &buffer,
                store_key(0x21),
                b"0",
                b"1",
                &first,
                limits(0, 0, 0),
                false,
            )
            .unwrap(),
            OutOfWindowBatchApplyV1::AlreadyApplied {
                stats: OutOfWindowStatsV1 {
                    records: 2,
                    bytes: 11,
                    overflowed: 0,
                },
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );
    }

    #[test]
    fn empty_batch_advances_progress_and_replays_without_rows() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(4, 64, 128);
        let empty = Vec::new();
        assert_eq!(
            apply_clean(
                &buffer,
                store_key(0x31),
                b"before",
                b"after",
                &empty,
                cap,
                true
            )
            .unwrap(),
            OutOfWindowBatchApplyV1::Applied {
                stats: OutOfWindowStatsV1 {
                    records: 0,
                    bytes: 0,
                    overflowed: 0,
                },
                overflowed: 0,
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );
        assert_eq!(
            apply_clean(
                &buffer,
                store_key(0x31),
                b"before",
                b"after",
                &empty,
                limits(0, 0, 0),
                false,
            )
            .unwrap(),
            OutOfWindowBatchApplyV1::AlreadyApplied {
                stats: OutOfWindowStatsV1 {
                    records: 0,
                    bytes: 0,
                    overflowed: 0,
                },
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );
    }

    #[test]
    fn four_admitted_stores_can_advance_empty_polls_under_two_record_cap() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(2, 64, 128);
        let stores = [0xa4, 0xa5, 0xa6, 0xa7];
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: stores
                    .iter()
                    .map(|byte| (store_key(*byte), b"0".to_vec()))
                    .collect(),
                max_transitions_per_store: 2,
            })
            .unwrap();
        for byte in stores {
            buffer
                .apply_out_of_window_batch_with_validation(
                    store_key(byte),
                    b"0",
                    b"1",
                    &[],
                    OutOfWindowBatchValidationV1::CLEAN,
                    cap,
                )
                .unwrap();
        }
        assert_eq!(buffer.verify_out_of_window(cap).unwrap().records, 0);
    }

    #[test]
    fn identical_payloads_in_distinct_batches_are_retained_distinctly() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(4, 64, 128);
        let same = batch(&[b"same"]);
        apply_clean(&buffer, store_key(0x41), b"0", b"1", &same, cap, true).unwrap();
        apply_clean(&buffer, store_key(0x41), b"1", b"2", &same, cap, false).unwrap();
        assert_eq!(
            buffer.verify_out_of_window(cap).unwrap(),
            OutOfWindowStatsV1 {
                records: 2,
                bytes: 8,
                overflowed: 0,
            }
        );
    }

    #[test]
    fn batch_caps_retain_safe_records_advance_cursor_and_replay_without_recharge() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(4, 4, 4);
        let records = batch(&[b"safe", b"too-long"]);
        assert_eq!(
            apply_clean(&buffer, store_key(0x51), b"0", b"1", &records, cap, true).unwrap(),
            OutOfWindowBatchApplyV1::Applied {
                stats: OutOfWindowStatsV1 {
                    records: 1,
                    bytes: 4,
                    overflowed: 1,
                },
                overflowed: 1,
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );
        assert_eq!(
            apply_clean(
                &buffer,
                store_key(0x51),
                b"0",
                b"1",
                &records,
                limits(0, 0, 0),
                false,
            )
            .unwrap(),
            OutOfWindowBatchApplyV1::AlreadyApplied {
                stats: OutOfWindowStatsV1 {
                    records: 1,
                    bytes: 4,
                    overflowed: 1,
                },
                validation: OutOfWindowBatchValidationV1::CLEAN,
            }
        );
    }

    #[test]
    fn batch_progress_rejects_cursor_or_batch_divergence() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(4, 64, 128);
        let records = batch(&[b"same"]);
        apply_clean(&buffer, store_key(0x61), b"0", b"1", &records, cap, true).unwrap();

        let error = buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0x61),
                b"0",
                b"different",
                &records,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .expect_err("same before cursor cannot be rebound to another after cursor");
        assert!(matches!(error, DaemonError::Refused(_)), "got {error:?}");

        let different = batch(&[b"different"]);
        let error = buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0x61),
                b"0",
                b"1",
                &different,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .expect_err("same transition cannot be rebound to another batch");
        assert!(matches!(error, DaemonError::Refused(_)), "got {error:?}");

        let error = buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0x61),
                b"wrong",
                b"2",
                &records,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .expect_err("a new transition must start at the durable current cursor");
        assert!(matches!(error, DaemonError::Refused(_)), "got {error:?}");
    }

    #[test]
    fn transition_count_is_bounded_by_independent_admission_cap() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(4, 64, 128);
        let empty = Vec::new();
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store_key(0x81), b"0".to_vec())],
                max_transitions_per_store: 1,
            })
            .unwrap();
        buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0x81),
                b"0",
                b"1",
                &empty,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .unwrap();
        let error = buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0x81),
                b"1",
                b"2",
                &empty,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .expect_err("a distinct transition cannot exceed the bounded journal");
        assert!(matches!(error, DaemonError::Refused(_)), "got {error:?}");
    }

    #[test]
    fn nonclean_validation_is_monotone_and_survives_replay() {
        let (_dir, path, key, buffer) = open();
        let cap = limits(4, 64, 128);
        let validation = OutOfWindowBatchValidationV1 {
            flags: 0b101,
            reason_digest: [0xabu8; 32],
        };
        let records = batch(&[b"nonclean"]);
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![(store_key(0xa1), b"0".to_vec())],
                max_transitions_per_store: 2,
            })
            .unwrap();
        let applied = buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0xa1),
                b"0",
                b"1",
                &records,
                validation,
                cap,
            )
            .unwrap();
        assert!(matches!(
            applied,
            OutOfWindowBatchApplyV1::Applied {
                validation: state,
                ..
            } if state == validation
        ));

        let clean = buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0xa1),
                b"1",
                b"2",
                &[],
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .unwrap();
        assert!(matches!(
            clean,
            OutOfWindowBatchApplyV1::Applied {
                validation: state,
                ..
            } if state == validation
        ));

        drop(buffer);
        let reopened = Buffer::open(&path, &key).unwrap();
        assert_eq!(
            reopened.verify_out_of_window_batch_state().unwrap(),
            validation
        );
        let replay = reopened
            .apply_out_of_window_batch_with_validation(
                store_key(0xa1),
                b"0",
                b"1",
                &records,
                validation,
                limits(0, 0, 0),
            )
            .unwrap();
        assert!(matches!(
            replay,
            OutOfWindowBatchApplyV1::AlreadyApplied {
                validation: state,
                ..
            } if state == validation
        ));
    }

    #[test]
    fn encrypted_transition_and_progress_swaps_fail_closed() {
        let (_dir, path, key, buffer) = open();
        let cap = limits(4, 64, 128);
        let records = batch(&[b"same"]);
        buffer
            .initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
                stores: vec![
                    (store_key(0xa2), b"0".to_vec()),
                    (store_key(0xa3), b"0".to_vec()),
                ],
                max_transitions_per_store: 2,
            })
            .unwrap();
        buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0xa2),
                b"0",
                b"1",
                &records,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .unwrap();
        buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0xa3),
                b"0",
                b"1",
                &records,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .unwrap();
        let k2 = buffer.batch_transition_key(&store_key(0xa2), b"0");
        let k3 = buffer.batch_transition_key(&store_key(0xa3), b"0");
        let txn = buffer.db.begin_write().unwrap();
        {
            let mut transitions = txn.open_table(OUT_OF_WINDOW_TRANSITIONS).unwrap();
            let v2 = transitions.get(k2).unwrap().unwrap().value().to_vec();
            let v3 = transitions.get(k3).unwrap().unwrap().value().to_vec();
            transitions.insert(k2, v3.as_slice()).unwrap();
            transitions.insert(k3, v2.as_slice()).unwrap();
            let mut progress = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
            let v2 = progress
                .get(*store_key(0xa2).as_bytes())
                .unwrap()
                .unwrap()
                .value()
                .to_vec();
            let v3 = progress
                .get(*store_key(0xa3).as_bytes())
                .unwrap()
                .unwrap()
                .value()
                .to_vec();
            progress
                .insert(*store_key(0xa2).as_bytes(), v3.as_slice())
                .unwrap();
            progress
                .insert(*store_key(0xa3).as_bytes(), v2.as_slice())
                .unwrap();
        }
        txn.commit().unwrap();
        drop(buffer);
        let error = Buffer::open(&path, &key).expect_err("encrypted key swaps must be rejected");
        assert!(
            matches!(error, DaemonError::Db(_) | DaemonError::Crypto(_)),
            "got {error:?}"
        );
    }

    #[test]
    fn deleting_a_linked_transition_fails_closed_on_reopen() {
        let (_dir, path, key, buffer) = open();
        let cap = limits(4, 64, 128);
        apply_clean(&buffer, store_key(0xb1), b"0", b"1", &[], cap, true).unwrap();
        let head = buffer.batch_transition_key(&store_key(0xb1), b"0");
        let txn = buffer.db.begin_write().unwrap();
        {
            let mut transitions = txn.open_table(OUT_OF_WINDOW_TRANSITIONS).unwrap();
            assert!(transitions.remove(head).unwrap().is_some());
        }
        txn.commit().unwrap();
        drop(buffer);
        let error = Buffer::open(&path, &key).expect_err("a missing linked row must be rejected");
        assert!(
            matches!(error, DaemonError::Db(_) | DaemonError::Crypto(_)),
            "got {error:?}"
        );
    }

    #[test]
    fn stale_same_key_snapshot_cannot_replace_the_admitted_start() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let path_a = dir_a.path().join("buffer.redb");
        let path_b = dir_b.path().join("buffer.redb");
        let key = "71".repeat(32);
        let store = store_key(0xc1);
        let buffer_a = Buffer::open(&path_a, &key).unwrap();
        let buffer_b = Buffer::open(&path_b, &key).unwrap();
        let admission = |cursor: &[u8]| OutOfWindowAdmissionV1 {
            stores: vec![(store, cursor.to_vec())],
            max_transitions_per_store: 2,
        };
        buffer_a
            .initialize_out_of_window_batch_progress(admission(b"admitted"))
            .unwrap();
        buffer_b
            .initialize_out_of_window_batch_progress(admission(b"stale"))
            .unwrap();
        apply_clean(
            &buffer_b,
            store,
            b"stale",
            b"after",
            &[],
            limits(4, 64, 128),
            false,
        )
        .unwrap();

        let transition_key = buffer_b.batch_transition_key(&store, b"stale");
        let transition = {
            let txn = buffer_b.db.begin_read().unwrap();
            let table = txn.open_table(OUT_OF_WINDOW_TRANSITIONS).unwrap();
            table.get(transition_key).unwrap().unwrap().value().to_vec()
        };
        let progress = {
            let txn = buffer_b.db.begin_read().unwrap();
            let table = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
            table
                .get(*store.as_bytes())
                .unwrap()
                .unwrap()
                .value()
                .to_vec()
        };
        let txn = buffer_a.db.begin_write().unwrap();
        {
            let mut transitions = txn.open_table(OUT_OF_WINDOW_TRANSITIONS).unwrap();
            transitions
                .insert(transition_key, transition.as_slice())
                .unwrap();
            let mut progress_table = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
            progress_table
                .insert(*store.as_bytes(), progress.as_slice())
                .unwrap();
        }
        txn.commit().unwrap();
        drop(buffer_a);
        drop(buffer_b);
        let error = Buffer::open(&path_a, &key)
            .expect_err("stale chain must not replace the admitted start");
        assert!(
            matches!(error, DaemonError::Db(_) | DaemonError::Crypto(_)),
            "got {error:?}"
        );
    }

    #[test]
    fn admission_without_journal_is_corrupt_for_verify_reopen_and_reinit() {
        let (_dir, path, key, buffer) = open();
        let store = store_key(0xc2);
        let admission = OutOfWindowAdmissionV1 {
            stores: vec![(store, b"0".to_vec())],
            max_transitions_per_store: 2,
        };
        buffer
            .initialize_out_of_window_batch_progress(admission.clone())
            .unwrap();
        let txn = buffer.db.begin_write().unwrap();
        assert!(txn.delete_table(OUT_OF_WINDOW_PROGRESS).unwrap());
        assert!(txn.delete_table(OUT_OF_WINDOW_TRANSITIONS).unwrap());
        {
            let mut meta = txn.open_table(META).unwrap();
            assert!(meta.remove(OUT_OF_WINDOW_BATCH_SCHEMA).unwrap().is_some());
        }
        txn.commit().unwrap();

        let error = buffer
            .verify_out_of_window_batch_state()
            .expect_err("admission without journal must not verify clean");
        assert!(matches!(error, DaemonError::Db(_)), "got {error:?}");
        let error = buffer
            .initialize_out_of_window_batch_progress(admission)
            .expect_err("reinitialization must not recreate a deleted journal");
        assert!(matches!(error, DaemonError::Db(_)), "got {error:?}");
        drop(buffer);
        let error = Buffer::open(&path, &key).expect_err("reopen must reject orphan admission");
        assert!(matches!(error, DaemonError::Db(_)), "got {error:?}");
    }

    #[derive(Clone, Copy, Debug)]
    enum BatchJournalCorruption {
        SchemaOnly,
        TablesAndAdmissionWithoutSchema,
        SchemaAndEmptyTablesWithoutAdmission,
        MissingProgress,
        MissingTransitions,
        CorruptProgress,
        CorruptTransitions,
    }

    fn assert_batch_journal_corruption_is_rejected(case: BatchJournalCorruption) {
        let (_dir, path, key, buffer) = open();
        let store = store_key(0xd1);
        let admission = OutOfWindowAdmissionV1 {
            stores: vec![(store, b"start".to_vec())],
            max_transitions_per_store: 2,
        };
        if !matches!(case, BatchJournalCorruption::SchemaOnly) {
            buffer
                .initialize_out_of_window_batch_progress(admission.clone())
                .unwrap();
        }
        let txn = buffer.db.begin_write().unwrap();
        match case {
            BatchJournalCorruption::SchemaOnly => {
                let mut meta = txn.open_table(META).unwrap();
                meta.insert(
                    OUT_OF_WINDOW_BATCH_SCHEMA,
                    [OUT_OF_WINDOW_BATCH_SCHEMA_VERSION].as_slice(),
                )
                .unwrap();
            }
            BatchJournalCorruption::TablesAndAdmissionWithoutSchema => {
                let mut meta = txn.open_table(META).unwrap();
                assert!(meta.remove(OUT_OF_WINDOW_BATCH_SCHEMA).unwrap().is_some());
            }
            BatchJournalCorruption::SchemaAndEmptyTablesWithoutAdmission => {
                let mut meta = txn.open_table(META).unwrap();
                assert!(
                    meta.remove(OUT_OF_WINDOW_BATCH_ADMISSION)
                        .unwrap()
                        .is_some()
                );
                drop(meta);
                let progress_keys = {
                    let progress = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
                    progress
                        .iter()
                        .unwrap()
                        .map(|row| row.unwrap().0.value())
                        .collect::<Vec<_>>()
                };
                let mut progress = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
                for key in progress_keys {
                    progress.remove(key).unwrap();
                }
            }
            BatchJournalCorruption::MissingProgress => {
                assert!(txn.delete_table(OUT_OF_WINDOW_PROGRESS).unwrap());
            }
            BatchJournalCorruption::MissingTransitions => {
                assert!(txn.delete_table(OUT_OF_WINDOW_TRANSITIONS).unwrap());
            }
            BatchJournalCorruption::CorruptProgress => {
                let mut progress = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
                progress
                    .insert(*store.as_bytes(), b"corrupt".as_slice())
                    .unwrap();
            }
            BatchJournalCorruption::CorruptTransitions => {
                let mut transitions = txn.open_table(OUT_OF_WINDOW_TRANSITIONS).unwrap();
                transitions
                    .insert([0xd2; 32], b"corrupt".as_slice())
                    .unwrap();
            }
        }
        txn.commit().unwrap();

        let verify = buffer.verify_out_of_window_batch_state();
        assert!(
            matches!(
                verify,
                Err(DaemonError::Db(_)) | Err(DaemonError::Crypto(_))
            ),
            "{case:?} must fail verify, got {verify:?}"
        );
        let reinit = buffer.initialize_out_of_window_batch_progress(admission);
        assert!(
            matches!(
                reinit,
                Err(DaemonError::Db(_)) | Err(DaemonError::Crypto(_))
            ),
            "{case:?} must fail reinitialization, got {reinit:?}"
        );
        drop(buffer);
        let reopened = Buffer::open(&path, &key);
        assert!(
            matches!(
                reopened,
                Err(DaemonError::Db(_)) | Err(DaemonError::Crypto(_))
            ),
            "{case:?} must fail reopen"
        );
    }

    #[test]
    fn every_partial_batch_journal_state_fails_closed() {
        for case in [
            BatchJournalCorruption::SchemaOnly,
            BatchJournalCorruption::TablesAndAdmissionWithoutSchema,
            BatchJournalCorruption::SchemaAndEmptyTablesWithoutAdmission,
            BatchJournalCorruption::MissingProgress,
            BatchJournalCorruption::MissingTransitions,
            BatchJournalCorruption::CorruptProgress,
            BatchJournalCorruption::CorruptTransitions,
        ] {
            assert_batch_journal_corruption_is_rejected(case);
        }
    }

    #[test]
    fn corrupt_or_swapped_batch_progress_rows_fail_closed() {
        let (_dir, _path, _key, buffer) = open();
        let cap = limits(4, 64, 128);
        let records = batch(&[b"same"]);
        apply_clean(&buffer, store_key(0x91), b"0", b"1", &records, cap, true).unwrap();

        let txn = buffer.db.begin_write().unwrap();
        {
            let mut progress = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
            progress
                .insert(*store_key(0x91).as_bytes(), b"corrupt".as_slice())
                .unwrap();
        }
        txn.commit().unwrap();
        let error = buffer
            .apply_out_of_window_batch_with_validation(
                store_key(0x91),
                b"0",
                b"1",
                &records,
                OutOfWindowBatchValidationV1::CLEAN,
                cap,
            )
            .expect_err("corrupt encrypted progress cannot be replayed");
        assert!(matches!(error, DaemonError::Crypto(_)), "got {error:?}");

        let txn = buffer.db.begin_write().unwrap();
        {
            let mut progress = txn.open_table(OUT_OF_WINDOW_PROGRESS).unwrap();
            progress
                .insert(*store_key(0x91).as_bytes(), b"corrupt".as_slice())
                .unwrap();
            let mut transitions = txn.open_table(OUT_OF_WINDOW_TRANSITIONS).unwrap();
            transitions
                .insert([0xa1; 32], b"corrupt".as_slice())
                .unwrap();
        }
        txn.commit().unwrap();
        let error = Buffer::open(&_path, &_key)
            .expect_err("a swapped/corrupt transition journal cannot reopen");
        assert!(
            matches!(error, DaemonError::Crypto(_) | DaemonError::Db(_)),
            "got {error:?}"
        );
    }
}
