//! Run events the daemon is allowed to emit (DECISIONS D3 exception list).

use serde::{Deserialize, Serialize};

/// Whether one trusted foreign-session declaration was inspected completely.
///
/// This is raw coverage evidence. It carries no path, session value, database value or
/// error string; checks decide what an unavailable observation means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum ForeignScanOutcome {
    Complete,
    Absent,
    Unavailable(ForeignScanUnavailableReason),
}

/// Closed reasons a bounded foreign-session declaration could not be inspected fully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForeignScanUnavailableReason {
    Discovery,
    Race,
    Open,
    Schema,
    Identity,
    Row,
    PathLimit,
    RowLimit,
    RecordLimit,
    SightingLimit,
}

impl ForeignScanUnavailableReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Race => "race",
            Self::Open => "open",
            Self::Schema => "schema",
            Self::Identity => "identity",
            Self::Row => "row",
            Self::PathLimit => "path_limit",
            Self::RowLimit => "row_limit",
            Self::RecordLimit => "record_limit",
            Self::SightingLimit => "sighting_limit",
        }
    }
}

/// The health states. The daemon reports them; no rule reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Online,
    UploadLagging,
    ConnectionLost,
    AgentExited,
    TrackerInterrupted,
    CaptureStalled,
    HooksSilent,
    ConfigRestored,
    DeadlineReached,
    ArmingPending,
    ArmingLost,
    VersionChanged,
    ConfigMismatch,
    /// A tailed store was truncated or its inode changed: the file on disk is no longer
    /// the file the cursor was reading. Reported alongside the chained
    /// `EventBody::TailNotice`, never instead of it.
    StoreReplaced,
    /// Free space on the artifact-root volume fell below the daemon's reserve, so the
    /// buffer degrades before `append` starts failing.
    DiskPressure,
    /// The entry's chain is closed. Additive, and it is a state of the
    /// DOCUMENT rather than a rung of the V1 health ladder (removed by D48): nothing chains it, because a
    /// health event chained after the seal would be a link past the last link. The status
    /// document carries it so that a sealed run stops reading `online — Recording…` with a
    /// countdown running.
    Sealed,
    /// A durable seal intent could not be reconciled. Capture is closed until an operator
    /// repairs the entry; this is deliberately distinct from `Sealed`, because no terminal
    /// claim has been promoted.
    SealingRecoveryNeeded,
}

/// What the tailer saw on disk that the chain could not otherwise explain (D9).
///
/// These are observations, not judgments: the daemon records what it measured (a shrink,
/// an inode change, a re-read from zero) and the verifier decides what it means. V9
/// reconciles a notice against the traced tool calls; a truncation with no traced cause,
/// not declared as an expected resume re-read, is `store_rewritten`.
///
/// Two of the five are the daemon's own capacity bounds rather than signatures of a
/// rewritten transcript. They have their own names so a line the tailer declined to carry
/// is never chained as `Truncated`, which V9 would read as a participant editing the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TailNoticeKind {
    /// The file shrank below the saved offset.
    Truncated,
    /// The inode at the path changed.
    FileReplaced,
    /// The tailer re-read from zero after a restart or a notice above.
    Reread,
    /// A single line exceeded the tailer's per-line cap; the shortfall is
    /// `offset_after - offset_before` bytes the chain does not carry.
    OverlongLine,
    /// A single WHOLE-FILE capture exceeded the tailer's per-file byte cap
    /// (`MAX_FILE_BYTES`), so only part of the file reached the chain; the shortfall is
    /// `offset_after - offset_before` bytes.
    ///
    /// Not a per-poll cap: a poll that hits the per-poll cap sets `TailBatch.more_available`
    /// and emits no notice at all, because nothing went wrong. The directory tailer's
    /// whole-file reading is the one that writes this.
    OversizeFile,
}

/// One store's cursor at a heartbeat (D9).
///
/// The type lives in `crate::cursor`, where it carries the unit its number is
/// measured in beside the human string. The name is re-exported here
/// because `EventBody::Heartbeat.stores_seen` is an events shape and every caller spells
/// it `events::StoreCursor`: one type at two paths, never two types.
pub use crate::cursor::StoreCursor;

/// A tracker-written file deviating from the bytes the daemon wrote (D6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDeviation {
    pub path: String,
    pub expected_sha256: String,
    pub observed_sha256: String,
    pub detected_at: String,
    pub diff: String,
    /// The daemon restored the expected bytes.
    pub restored: bool,
    /// Restoring invalidated a hook-trust hash, so capture may be down until re-arm (D6, Codex).
    pub arming_lost: bool,
}

/// A harness version change observed mid-run (D12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionChange {
    pub component: String,
    pub from: String,
    pub to: String,
    pub detected_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthEvent {
    pub state: HealthState,
    pub at: String,
    pub detail: String,
}

/// The largest string value a [`SeededKey`] carries verbatim, in bytes.
///
/// The line between a decision and a document. `claudeInChromeDefaultEnabled` is a boolean,
/// `numStartups` is a number, `theme` is a short string — those are the answers the
/// participant gave their harness, and they are what the record exists to show. `oauthAccount`
/// is an object holding an account; a long string is a document, a token or a path. The rule
/// is a shape rule and never a name list, because a name list is a place to forget an entry:
/// a key the manifest adds tomorrow is bounded by this on the day it is added.
///
/// **32 and not 64.** At 64 the rule would carry `userID` verbatim — Claude Code's stable
/// identifier for the account, 64 hex characters, constant across every entry that
/// participant ever runs. Chain events are not redacted (`ChainWriter::event` appends
/// directly, unlike `leg1`/`leg2`), so it would reach the chain, the exported trace and the
/// published verdict. That is the harm `SeededFile` avoids by carrying no digest: "two
/// public traces carrying the same one prove the same account ran both — across entries and
/// across participants". A published `userID` gives that linkage without needing a digest.
/// Every key whose value is a DECISION — `theme`,
/// `installMethod`, `autoUpdates`, `lastOnboardingVersion`, `firstStartVersion`,
/// `firstStartTime` and every boolean and number — is well under 32; a 64-hex account
/// identifier is not. Still a shape rule, still never a name list.
pub const HOME_SEED_VALUE_MAX_BYTES: usize = 32;

/// One key a `[home_seed]` merge copied, with its value where the shape rule allows one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeededKey {
    /// The key, as the connector manifest names it.
    pub name: String,
    /// The raw JSON value that was copied — recorded for `true`/`false`, for a number, and
    /// for a string of at most [`HOME_SEED_VALUE_MAX_BYTES`] bytes, and **absent for every
    /// other shape**: an object, an array, `null`, or a longer string is named and never
    /// quoted. Absent means "not the kind of value this record carries", never "empty".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

impl SeededKey {
    /// `name` with `value` where [`HOME_SEED_VALUE_MAX_BYTES`] and the shape rule allow it.
    /// The one place that rule is applied, so the recorder cannot apply a second one.
    pub fn new(name: &str, value: &serde_json::Value) -> Self {
        let carried = match value {
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some(value.clone()),
            serde_json::Value::String(s) if s.len() <= HOME_SEED_VALUE_MAX_BYTES => {
                Some(value.clone())
            }
            _ => None,
        };
        SeededKey {
            name: name.to_string(),
            value: carried,
        }
    }
}

/// One whole file a `[home_seed]` copied, by name and length.
///
/// **Never its contents, and never a digest of them.** Codex's `auth.json` is a credential,
/// and a digest is a stable identifier for that credential: two public traces carrying the
/// same one prove the same account ran both — across entries and across participants — and
/// anyone holding a leaked copy of the file could confirm whose runs it was used on. Which
/// credential a run was given is not a decision the participant made, so the record loses
/// nothing a reader is owed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeededFile {
    /// The file's name under the manifest's `from_dir`, e.g. `auth.json`.
    pub name: String,
    pub bytes: u64,
}

/// The legacy representation of a copied home-seed file.
///
/// It is retained only while a reader recomputes an already-written historical link.  New
/// evidence never serializes this shape: its digest would be a stable credential identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LegacySeededFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// The exact `HomeSeeded` representation that an evidence line carried when it was read.
///
/// This is link-preservation metadata, not new evidence: current writers use `Later`, while
/// a historical line keeps `Legacy` only so its original hash preimage can be
/// recomputed.  The legacy constructor is private: a caller can construct only the default
/// current layout, while deserialization is the one path that can retain the old one.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSeedWireLayout(HomeSeedWireLayoutKind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum HomeSeedWireLayoutKind {
    Later,
    Legacy { files: Vec<LegacySeededFile> },
}

impl Default for HomeSeedWireLayout {
    fn default() -> Self {
        Self(HomeSeedWireLayoutKind::Later)
    }
}

impl HomeSeedWireLayout {
    fn legacy(files: Vec<LegacySeededFile>) -> Self {
        Self(HomeSeedWireLayoutKind::Legacy { files })
    }

    fn is_legacy(&self) -> bool {
        matches!(self.0, HomeSeedWireLayoutKind::Legacy { .. })
    }
}

/// What one launch's `[home_seed]` carried out of the participant's own configuration and
/// into the per-run harness home.
///
/// # Why this is recorded at all
///
/// Harness settings are the participant's decision, not the
/// tracker's. The tracker seeds their own answers into the per-run home unchanged, so the
/// trace has to say WHICH answers it carried — `claudeInChromeDefaultEnabled` is the
/// browser-tools feature switch, and a run whose agent had browser tools is a different run
/// from one whose agent did not. This is a raw record and nothing else: no check reads it, no
/// finding rests on it, and the verdict carries it as a fact list.
///
/// It is the CLI's word. A participant who edits the per-run home by hand AFTER the seed is
/// not caught by this record — the config watch and the hooks are what catch that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSeedRecord {
    /// The daemon-issued process-handoff generation this seeding belongs to.  It is an
    /// opaque ordering marker, not a secret and not a digest of configuration bytes; a
    /// restart uses it with `handoff_request_digest` to reconcile exactly one append.
    pub handoff_generation: Option<u64>,
    /// The process registration's canonical termination identity digest.  It is bound to
    /// entry+generation and intentionally excludes every `home_seed` field, so the chain
    /// gets a replay marker without turning copied configuration into public evidence.
    pub handoff_request_digest: Option<String>,
    /// The connector whose `[home_seed]` this followed, e.g. `claude-code-cli`.
    pub connector: String,
    /// The source **as the manifest spells it** — `{home}/.claude.json`, `{home}/.codex` —
    /// and never the path it resolved to. The resolved path names the participant's account
    /// on their own machine; the manifest's spelling is the rule that was followed, which is
    /// what a reader of somebody else's trace can check against the pinned connector.
    pub from: String,
    /// The keys copied into the destination document, in manifest order.
    pub keys: Vec<SeededKey>,
    /// The subset of `keys` whose destination value already equalled the value written, so
    /// the merge changed nothing. On a resume — which seeds again — that is normally all of
    /// them, and this is how the record says "the relaunch changed nothing" about a key.
    ///
    /// **Unchanged, not skipped.** The key merge has no skipped case: a key on this list was
    /// still written, identically. `files_kept` below means the opposite — that file was not
    /// written at all — which is why the two lists are not spelled alike.
    pub keys_unchanged: Vec<String>,
    /// The keys the TRACKER decided, as opposed to the ones it copied above.
    ///
    /// There is one, and it is the workspace trust entry: the connector's `trust_keys` —
    /// `hasTrustDialogAccepted` for Claude Code — set to `true` for this run's own workspace,
    /// so the harness does not stop at its per-project safety question over a directory the
    /// tracker itself created, empty, for this entry. No copied key can answer that question,
    /// because the directory did not exist when the participant's file was written.
    ///
    /// A reader of somebody else's trace has to be able to tell what the participant chose
    /// from what the tracker did to their harness, so this is a list of its own rather than
    /// another entry in `keys`. The workspace PATH is deliberately not here: it names a
    /// directory on the participant's machine, and this record holds no resolved path.
    /// Empty for a connector that declares no `trust_field`.
    pub written: Vec<SeededKey>,
    /// Whole files copied into the per-run home by this launch.
    pub files: Vec<SeededFile>,
    /// Declared files the per-run home ALREADY held, left exactly as they were — nothing was
    /// written. Names only, which is all the launch that did copy them recorded either.
    pub files_kept: Vec<String>,
    /// Declared files this machine does not have, so nothing was copied. Reported, never
    /// refused.
    pub files_missing: Vec<String>,
    /// The JSON field layout the line used when it entered this process.  It is skipped by
    /// neither the chain nor ordinary serialization: [`Serialize`] selects the matching
    /// historical or current wire shape below.
    #[doc(hidden)]
    pub wire_layout: HomeSeedWireLayout,
}

impl HomeSeedRecord {
    /// Whether this record retained the old wire layout while parsing an historical line.
    /// This is crate-visible so envelope version validation can limit that layout to V1.
    pub(crate) fn has_legacy_wire_layout(&self) -> bool {
        self.wire_layout.is_legacy()
    }
}

impl Serialize for HomeSeedRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Later<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            handoff_generation: &'a Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            handoff_request_digest: &'a Option<String>,
            connector: &'a str,
            from: &'a str,
            keys: &'a [SeededKey],
            keys_unchanged: &'a [String],
            written: &'a [SeededKey],
            files: &'a [SeededFile],
            files_kept: &'a [String],
            files_missing: &'a [String],
        }

        #[derive(Serialize)]
        struct Legacy<'a> {
            connector: &'a str,
            from: &'a str,
            keys: &'a [SeededKey],
            keys_kept: &'a [String],
            files: &'a [LegacySeededFile],
            files_kept: &'a [String],
            files_missing: &'a [String],
        }

        match &self.wire_layout.0 {
            HomeSeedWireLayoutKind::Later => Later {
                handoff_generation: &self.handoff_generation,
                handoff_request_digest: &self.handoff_request_digest,
                connector: &self.connector,
                from: &self.from,
                keys: &self.keys,
                keys_unchanged: &self.keys_unchanged,
                written: &self.written,
                files: &self.files,
                files_kept: &self.files_kept,
                files_missing: &self.files_missing,
            }
            .serialize(serializer),
            HomeSeedWireLayoutKind::Legacy { files } => {
                if self.handoff_generation.is_some()
                    || self.handoff_request_digest.is_some()
                    || !self.written.is_empty()
                    || files.len() != self.files.len()
                    || files.iter().zip(&self.files).any(|(legacy, current)| {
                        legacy.name != current.name || legacy.bytes != current.bytes
                    })
                {
                    return Err(serde::ser::Error::custom(
                        "legacy HomeSeeded layout does not match its normalized record",
                    ));
                }
                Legacy {
                    connector: &self.connector,
                    from: &self.from,
                    keys: &self.keys,
                    keys_kept: &self.keys_unchanged,
                    files,
                    files_kept: &self.files_kept,
                    files_missing: &self.files_missing,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for HomeSeedRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireSeededKey {
            name: String,
            #[serde(default)]
            value: NullableHomeSeedField<serde_json::Value>,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireLegacySeededFile {
            name: String,
            bytes: u64,
            sha256: String,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireCurrentSeededFile {
            name: String,
            bytes: u64,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum WireFile {
            Legacy(WireLegacySeededFile),
            Later(WireCurrentSeededFile),
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            handoff_generation: NullableHomeSeedField<u64>,
            #[serde(default)]
            handoff_request_digest: NullableHomeSeedField<String>,
            connector: String,
            from: String,
            #[serde(default)]
            keys: Vec<WireSeededKey>,
            #[serde(default)]
            keys_kept: NullableHomeSeedField<Vec<String>>,
            #[serde(default)]
            keys_unchanged: NullableHomeSeedField<Vec<String>>,
            #[serde(default)]
            written: NullableHomeSeedField<Vec<WireSeededKey>>,
            #[serde(default)]
            files: Vec<WireFile>,
            #[serde(default)]
            files_kept: Vec<String>,
            #[serde(default)]
            files_missing: Vec<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let keys = wire
            .keys
            .into_iter()
            .map(|key| {
                Ok(SeededKey {
                    name: key.name,
                    value: key.value.into_option("HomeSeeded key value")?,
                })
            })
            .collect::<Result<Vec<_>, D::Error>>()?;
        let handoff_generation = wire.handoff_generation.into_option("handoff_generation")?;
        let handoff_request_digest = wire
            .handoff_request_digest
            .into_option("handoff_request_digest")?;
        let keys_unchanged = wire.keys_unchanged.into_option("keys_unchanged")?;
        let written = wire.written.into_option("written")?;
        match wire.keys_kept {
            NullableHomeSeedField::Present(keys_kept) => {
                if keys_unchanged.is_some()
                    || written.is_some()
                    || handoff_generation.is_some()
                    || handoff_request_digest.is_some()
                {
                    return Err(serde::de::Error::custom(
                        "legacy HomeSeeded layout cannot carry later fields",
                    ));
                }
                let mut legacy_files = Vec::with_capacity(wire.files.len());
                for file in wire.files {
                    match file {
                        WireFile::Legacy(file) => legacy_files.push(LegacySeededFile {
                            name: file.name,
                            bytes: file.bytes,
                            sha256: file.sha256,
                        }),
                        WireFile::Later(_) => {
                            return Err(serde::de::Error::custom(
                                "legacy HomeSeeded layout requires SeededFile.sha256",
                            ));
                        }
                    }
                }
                let files = legacy_files
                    .iter()
                    .map(|file| SeededFile {
                        name: file.name.clone(),
                        bytes: file.bytes,
                    })
                    .collect();
                Ok(HomeSeedRecord {
                    handoff_generation: None,
                    handoff_request_digest: None,
                    connector: wire.connector,
                    from: wire.from,
                    keys,
                    keys_unchanged: keys_kept,
                    written: Vec::new(),
                    files,
                    files_kept: wire.files_kept,
                    files_missing: wire.files_missing,
                    wire_layout: HomeSeedWireLayout::legacy(legacy_files),
                })
            }
            NullableHomeSeedField::Null => Err(serde::de::Error::custom(
                "legacy HomeSeeded keys_kept cannot be null",
            )),
            NullableHomeSeedField::Missing => {
                let keys_unchanged = keys_unchanged.ok_or_else(|| {
                    serde::de::Error::custom("later HomeSeeded layout requires keys_unchanged")
                })?;
                let written = written.ok_or_else(|| {
                    serde::de::Error::custom("later HomeSeeded layout requires written")
                })?;
                let written = written
                    .into_iter()
                    .map(|key| {
                        Ok(SeededKey {
                            name: key.name,
                            value: key.value.into_option("HomeSeeded written key value")?,
                        })
                    })
                    .collect::<Result<Vec<_>, D::Error>>()?;
                let mut files = Vec::with_capacity(wire.files.len());
                for file in wire.files {
                    match file {
                        WireFile::Later(file) => files.push(SeededFile {
                            name: file.name,
                            bytes: file.bytes,
                        }),
                        WireFile::Legacy(_) => {
                            return Err(serde::de::Error::custom(
                                "later HomeSeeded layout cannot carry SeededFile.sha256",
                            ));
                        }
                    }
                }
                Ok(HomeSeedRecord {
                    handoff_generation,
                    handoff_request_digest,
                    connector: wire.connector,
                    from: wire.from,
                    keys,
                    keys_unchanged,
                    written,
                    files,
                    files_kept: wire.files_kept,
                    files_missing: wire.files_missing,
                    wire_layout: HomeSeedWireLayout::default(),
                })
            }
        }
    }
}

/// Distinguishes an absent field from an explicit JSON `null` for every HomeSeeded member
/// whose writer can omit it.  `Option<T>` collapses those two inputs, which would let a
/// malformed line reuse the hash of a null-free canonical record before its link is checked.
#[derive(Debug, Default)]
enum NullableHomeSeedField<T> {
    #[default]
    Missing,
    Null,
    Present(T),
}

impl<T> NullableHomeSeedField<T> {
    fn into_option<E>(self, field: &str) -> Result<Option<T>, E>
    where
        E: serde::de::Error,
    {
        match self {
            Self::Missing => Ok(None),
            Self::Null => Err(E::custom(format!("HomeSeeded {field} cannot be null"))),
            Self::Present(value) => Ok(Some(value)),
        }
    }
}

impl<'de, T> Deserialize<'de> for NullableHomeSeedField<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor<T>(std::marker::PhantomData<T>);

        impl<'de, T> serde::de::Visitor<'de> for Visitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = NullableHomeSeedField<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a HomeSeeded value or null")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(NullableHomeSeedField::Null)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(NullableHomeSeedField::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                T::deserialize(deserializer).map(NullableHomeSeedField::Present)
            }
        }

        deserializer.deserialize_option(Visitor(std::marker::PhantomData))
    }
}
