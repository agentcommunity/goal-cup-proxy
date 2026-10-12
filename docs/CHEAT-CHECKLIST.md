# Cheat checklist — what a Pass does not cover

For anyone about to trust a Pass or claim a new check closes something. It lists the blind spots
this build accepts, each with what bounds it and the ticket that tracks it. Anything that behaves
worse than stated here is a bug. Add a row whenever a review accepts a gap instead of fixing it.

Updated: 2026-10-13. A verifier exit 0 alone is not a capture claim: a claim needs native and wire
records per real harness ([ARCHITECTURE](ARCHITECTURE.md)).

## On m2/wire (current)

The hook leg and OpenCode are gone on `m2/wire`, so the v1/trunk rows below that lean on hooks do not
apply. A v5 capture trace is judged by the integrity reader (exit 0 complete and valid, 1 incomplete
or unsealed, 2 invalid, 3 no usable report); the V1 checks are not in this tree. LocalOnly (no server,
receipt or ACK) is a valid local outcome. Nothing on m2 is qualified for long runs ([TODO](../TODO.md)).

- **A local-only record can be rewritten and re-sealed.** The chain is plain SHA-256 with no
  secret key: anyone who controls the machine can edit a record, recompute the hashes after it
  and update `seal.json`, or truncate at a clean point (no producer left open) and finish again
  with the public code, and the verifier still exits 0 with anchor LocalOnly. What the chain
  catches is an edit that leaves the later hashes alone, a reorder, a splice, a truncation left
  unsealed and accidental corruption. Every trace this build ships is local only. Bounded by:
  nothing yet. The fix is a server-signed acknowledgement of the chain head, which needs the
  hourly upload (TODO step 2) and a production server key; until then even a signed receipt can
  be forged ([SECURITY](../SECURITY.md)).

Ticket IDs below (H-xx, F-xx) name tickets of the retired `LEDGER.md`; read it in git history at `6494030`. Open issues that still matter are in [TODO](../TODO.md#known-issues).

### Wire leg, finish and storage

- **Proxy loss is chained, but not counted per loss.** The proxy chains each loss when it begins
  (fail-open tunnel, table full, opaque fallback, unsettled cut) as a stream-zero `Loss` frame
  written as a stream-less gap, so a lossy run reads incomplete (closed `76e88bad`). A
  well-formed request that meets the upstream's clean close of an idle keep-alive is not a
  fallback: it is not forwarded, the connection closes `Interrupted`, and the client retries on a
  new connection. Residuals: an
  old CLI on a new daemon keeps the old behaviour (counters only on the seal request); gap counts
  count records, not individual losses; a handled finish shows doubled daemon-clocked gaps.
  Ticket: H-16.
- **A tunnel whose ClientHello names an approved host is a loss; one that names none is not.**
  A tunnel to any authority (an IP literal, a sibling name, an authentication host) whose
  ClientHello names an approved host is chained as a fail-open loss, and so is a request inside
  an intercepted connection whose Host names another host (the connection goes opaque, so a
  later legitimate request on it is not captured either). A tunnel to a host that is not
  approved stays opaque by design, and is not a loss, when its ClientHello names a host that is
  not approved (domain fronting on a provider's edge, an ECH outer name), names none, or is not
  TLS: an edge that answered the API that way would let it through uncaptured. Every tunnel now
  waits for the client's first bytes, inside the 30-second admission deadline, so a protocol
  whose server speaks first, or a CONNECT held idle past the deadline, fails admission. A decided
  tunnel still open at stop (a dev server or MCP server that outlived the harness) is cut without
  a loss: a fail-open one was already chained at its decision. Only a connection cut before its
  decision is chained as an unsettled cut.
- **The seal walk lists what it cannot read, and has no time bound.** A symlink, FIFO, socket or
  device in the workspace is listed with the absent digest and never followed or read, so the
  agent cannot wedge the seal with one. A directory the agent makes unreadable (`chmod 000`) is
  listed the same way and its contents are not: an absent digest on a directory means "contents
  unseen", a way to hide files from the seal manifest that stays visible as the marker. A
  regular file is hashed whole with no size cap or deadline inside the walk, so a huge file or
  one on a stalled network mount still holds the run lock while it is read.
- **Two faults can leave a harness running on a sealed entry.** The spawn gate (4b) closes the
  spawn-to-registration window and the finish budget (4a) keeps the SIGTERM, grace, SIGKILL bound.
  A CLI crash after the gate released the child, combined with a raced manual `goalcup seal`, can
  still leave a harness running. Needs both faults. Ticket: H-10; record
  m2-h10-finish-and-spawn-gate.
- **The final executable check is neither a coherent snapshot nor inode-bound execution on macOS.**
  After durable registration the CLI re-opens the exact canonical path with
  `O_NOFOLLOW|O_NONBLOCK`, requires a regular file and streams it into SHA-256. The residual window
  starts at the first read: a same-UID writer can mutate the open file in place so the read sequence
  hashes to the pin while the file's final bytes differ. After the read, `exec` resolves the path,
  so a same-UID pathname replacement can change what runs. This does not bind exact bytes or inode.
- **One run per shared home, by a lock with two gaps.** `goalcup start` holds an exclusive flock
  on `capture-homes/<id>.lock` until its finish returns, and refuses a second start of the same
  harness (D39). The lock dies with the launcher: if `goalcup start` is SIGKILLed while a
  registered harness keeps running, a new start can share the home with it. A by-hand
  `codex login` into that home takes no lock. A Ctrl-C that lands during the release itself
  (milliseconds) still reaches only the parked child, and the harness runs.
- **Shared-home validation has same-UID and ACL residuals.** The CLI preserves D39's exact leaf
  alias after checking and rechecking leaf/target identities, ownership and exact target mode
  `0700`; the daemon separately refuses every loaded connector's `real_dirs`. The CLI does not
  inspect ACLs, and a process under the same UID can replace the alias or target after the final
  pre-create check. Those are residuals, not permissions or inode-binding claims.
- **Open assumptions.** One proxy per entry (CP-A4); the 8 MiB queue holds about 2 MiB of payload, so
  one stall can lose up to about 2.1 MiB; `seal.json` sits outside the reservation, so a failed seal
  write reads `incomplete` with no further policy; the network-filter rule is bound to the E01
  binary hash. Ticket: H-04.
- **A base64url run of 16 KiB or more can still redact the rest of its JSON line** (the redactor
  holds 16 KiB and fails closed), but only when a rule prefix inside it (`sk-`, `eyJ`, `AIza`,
  `glpat-`) lies more than 16 KiB before the run's end. A bare run holds nothing: the JSON key and
  the email local part are bounded (128 and 64 units). The run is common: the native Codex rollout
  persists `encrypted_content` on every reasoning item (9 in
  `spikes/codex-cli/fixtures/rollout-main-01a06614.jsonl`, largest 2,020 B; 8 in
  `spikes/codex-app/fixtures/rollout-main-01a06657.jsonl`, largest 2,276 B). The earlier
  "unreachable" counted wire client messages. Ticket: F-07.
- **Redaction residuals.** An invalid UTF-8 byte inside a key body leaves the whole key raw (fix
  before any raw-byte capture surface). A lone `BEGIN` mention redacts to the end of that record and
  breaks that record's JSON. Ticket: H-20.

### Hermes reference row (D41): what the key scan cannot see

The key-fragment scan (`crates/goalcup-cli/src/key_scan.rs`) and the `masked_key` redaction rule fail
closed where they can. These are the shapes they cannot see (independent secret review, 2026-10-02).

- **Encodings read:** the key; its first and last eight bytes; masks of 4, 6 and 8 key bytes with
  `...`, `…`, `***` or `…` plus the last four; any mask with a gap of at most 16 bytes between the
  first four and last four; the first 16 hex digits of its SHA-256; each of those inside base64
  (standard and URL alphabets, three alignments). **Not caught:** UTF-16, a JSON `\u00XX`-escaped key,
  double base64, the key in hex, a 5/4 mask with more than 12 stars and no eight-byte prefix, gzip
  beyond what is stored literally.
- **Mask shapes not named:** a suffix-only mask (`****abcd`), the 3/4 mask `sk-...abcd`, an upper-case
  prefix, a window gap over 16 bytes. DeepSeek's own 401 is believed to echo a suffix-only mask; four
  characters is not a fragment (D41), so that residue (16 bits) can reach the trace and a terminal.
- **Split masks.** A mask split across two `data_b64` segment records is invisible to a text scan of
  the chain file. The run scan (`key_scan::scan_run`, called by `goalcup start` after the finish
  report) also decodes each stream's segments through the verifier's reader and scans each stream's
  bytes in order. A trace it cannot read, a segment that does not decode, or a trace with no segment
  is `incomplete`, never `clean`. Residual: only the redactor's retained bytes are decoded, so a
  fragment the redactor already removed is invisible. Ticket: H-18.
- **The verdict file is bound, not locked** (H-18). `<run>/key-scan.txt` ends with `trace`,
  `trace-chain bytes=<n> sha256=<hex>` and `scanned-files count=<n> sha256=<hex>`, a digest of the
  sorted listing (relative path, kind, size, modification time, inode) of the run and trace
  directories. `check_verdict` returns Current, Stale, Unbound or Missing, and only Current is clean.
  **No retention step calls it yet.** An in-place rewrite that restores the old modification time
  is not detected, run-directory file bytes are not hashed (the trace is, by content), and the
  verdict word can be edited. A run ended before its finish writes `incomplete`; a Ctrl-C that
  kills `goalcup` writes nothing, which a retention step must read as not clean.
- **The tee withholds what it sees, not what it cannot.** A matcher holds a stream's last `overlap`
  bytes; a named fragment never reaches the terminal, and on a hit the bytes read with it are
  dropped for a fixed line. A shape the scan does not name is forwarded. Output held when a
  descendant keeps a stream open past the finish deadline is never sent.
- **A JSON `\u`-escaped key reaches the terminal.** The tee, `scan_tree` and `scan_trace` match
  raw bytes, so a key spelled as `\uXXXX` escapes is forwarded to the participant's terminal and
  is no hit in a native file or the trace. Only a bound identity is checked decoded (D50): such a
  key in the stdout identity's id binds nothing and fails the row (`credential_stdout_identity.rs`,
  `escaped-id`); anywhere else it passes. Ticket: H-18.
- **"Stays local" is checked by local signal only.** A goal file under Git, `Library/Mobile
  Documents`, `Library/CloudStorage` or a folder named Dropbox, OneDrive, Google Drive or iCloud Drive
  is refused. `~/Desktop` and `~/Documents` are refused only when local state says iCloud owns them
  (the File Provider attribute, or the same device and inode as the CloudDocs folder). No network
  call. A folder synced in a way those signals do not show is not caught. **Unverified positive:** no
  machine here has the setting on. Ticket: H-18.
- **Hermes' tool-child environment is source-only evidence.** The blocklist covering
  `DEEPSEEK_API_KEY` is read from Hermes source; no exercised tool child has shown it. Helpers
  outside that sanitiser inherit everything, which is why the `tirith` download is off. `models.dev`
  is still contacted for model metadata (no switch; it carries neither key nor session content).
  Marker timing is tested only against a fake late marker. Ticket: H-18.
- **`masked_key` is soft and has no boundary.** It never wins an overlap, so a mask glued to a longer
  secret is redacted by the longer rule and what is left is redacted as `masked_key`. A key that
  follows a complete mask with no separator keeps its four-character tail; neither is a D41
  fragment. `spikes/redact.py` (fixtures only) is held to be no weaker than the daemon; its one
  known difference is that Python's `IGNORECASE` also folds `İ` and `ı`, so it can redact more.

## On v1/trunk

These rows are V1 history: the hook-era build on `v1/trunk` and `v0.1.0`. D48 deleted that code
from this tree; the files they name are at the `v0.5.0` tag. They are kept short; the earlier long
wording is in git (`git show f8621f99:docs/CHEAT-CHECKLIST.md`).

- **SQLite in-place UPDATE or DELETE is unwitnessed** (OpenCode tailer pages by rowid;
  `crates/goalcup-daemon/src/tailer/sqlite.rs` at `v0.5.0`). OpenCode has no connector on m2.
- **App-wide config changed while no daemon runs is not reported.** The baseline is re-hashed at
  daemon start; the chain shows a `DaemonRestart` marking the window.
- **No server key is pinned** until V1.1, so no trace is `anchor: signed` on its own; a key
  supplied with `--dev-key` is reported as `supplied` with `server_key_unpinned`. V1
  qualification used the development signer.
- **More than 20,000 transcript files** hit the directory tailer's seen-set cap (`MAX_SEEN_FILES`);
  further files are reported as `StoreUnavailable`, never tailed.
- **The connector-directory digest walks by name, then opens by handle**, so a symlink swap between
  listing and open would be followed (needs `O_NOFOLLOW`); the window is the daemon's own.
- **The CLI trusts its own daemon's unsigned answers** (`WorkspaceInfo`, `ResumeInfo`): the
  participant attacking their own run. The trace is judged against signed documents regardless.
- **A redacted record re-read after a hard kill may chain twice**, witnessed by a `Reread` notice.
- **`$CODEX_HOME/config.toml` is not watched** (Codex rewrites its tip counter there each launch). A
  participant who silences hooks through it loses witnesses, not evidence: prompts are still tailed
  off the rollout, so an unwitnessed message is a V2 judgement and a V3 flag. Hook files are watched.
- **Foreign scans use the trusted installed-manifest union.** Legacy `pre_arm_scan` completeness,
  SQLite pathname reopen and native timing remain qualification gaps; an incomplete or unavailable
  scan is reported, never treated as clean.
- **`home_seeded` is the CLI's word about the seed and nothing more.** A per-run home edited after the
  seed is caught by the config watch and the hooks, not by it. A copied credential is a name and a
  length, with no digest, so no stable identifier of it appears in public traces.
- **A leg-2 post suppressed just before a witnessed resume is invisible to `leg2_receiver_seq_gap`.**
  The rule forgives a counter that restarts at 1 across a `source: "resume"` post; V2 and V3 still
  catch the dropped post as an unwitnessed leg-1 record. The rule's test was deleted on m2 with the
  receiver.
- **Termination is bounded to the owned set.** A daemonized child that left the group and the walked
  lineage before the first sweep is not claimed; a launch that leads a process group keeps the
  fail-closed answer, and no shipped connector registers a pgid; a process forked by a dying member
  after the root exits cannot be adopted. F0 (2026-09-17) is closed; the residual windows are in
  [TERMINATION](TERMINATION.md) and [D30](DECISIONS.md#d30-confirmed-termination-is-bounded-to-the-owned-process-set-and-the-limitation-is-stated).
  Read the Linux failure of `a_child_started_before_the_next_authenticated_snapshot_is_chased`
  (the 2026-09-16 Linux ARM qualification record, in git history at `6494030`) before citing that test.
- **Seal attribution is a Flag, not the gate** (D35). A shell call that merely quotes a whole
  `apply_patch` envelope attributes the file it names without writing it
  (`patch_target_paths`); `BULK_ATTRIBUTION_THRESHOLD` caps the mass-import version, a single planted
  file sits under it.

### Things to try on v1/trunk (should be caught)

- Edit a chained transcript line: `store_rewritten` or `Reread` evidence.
- Start a second harness session in the workspace: refused or chained as foreign, never as yours.
- Copy a prepared tree into the empty workspace: caught by the baseline (chained at seq 0, bound by
  V13), by heartbeats with no unobserved interval, and by the foreign scan (D5, V4).
- Stop the daemon mid-run, edit, restart: restart witnessed; the edit is still evidence.
- Change an allowlisted app-wide file mid-run: `ConfigDeviation` chained and the file restored.
- Kill the tracker with SIGKILL and keep working: upload gap refilled, health says what happened.
- Send a goal over the length gate: blocked, and the leg-1 copy is still captured.
