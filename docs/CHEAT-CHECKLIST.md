# Cheat checklist: what a Pass does not cover

Read this before you trust a verifier Pass or claim that a new check closes something. It lists
the blind spots this build accepts, and what bounds each. Anything that behaves worse than stated
here is a bug. Add a row when a review accepts a gap instead of fixing it. Open work on these rows
is in [TODO](../TODO.md#known-issues) under the ticket IDs (H-xx, F-xx) used here.

A verifier exit 0 alone is not a capture claim. A capture claim needs native and wire records for
the real harness ([ARCHITECTURE](ARCHITECTURE.md)). Exit 0 means the record is complete and
consistent, not that the participant followed the rules.

## Anchoring and the agent's reach

- **A trace that never reached the server can be rewritten and re-sealed.** The chain is plain
  SHA-256 with no secret. Whoever controls the machine can edit a record, recompute the hashes
  after it, update `seal.json` (or truncate at a clean point and finish again), and the verifier
  still exits 0 with anchor `LocalOnly`. The chain catches an edit that leaves later hashes alone,
  a reorder, a splice, a truncation left unsealed and accidental corruption. Bounded by: the
  server. A run uploaded during the week carries the server's signed receipt and a signed
  acknowledgement per stored chunk, under a key pinned in the verifier. A rewrite of acknowledged
  records no longer matches them. The window is the part of the chain not yet acknowledged.
- **The agent can read the run credential.** The per-run browser approval returns a credential
  for that run alone. It is stored at mode 0600 under the tracer's configuration directory
  (`runs/<run id>/credential`) and never in the harness's environment or argv. The harness runs
  as the participant and can read any file they can. With it an agent could upload to that run
  only. Bounded by: the server binds every chunk to the run's chain position, and a verify job
  re-runs `goalcup-verify` over the stored chain every hour and at seal.
- **An unlimited agent can reach the tracer's own state (D52).** The shipped connectors do not
  sandbox the agent. A tool command runs with the participant's full access and network. It can
  read `control.json` and call the daemon's loopback control API, rewrite its own session file,
  write `config.toml` or `settings.json` into the shared Codex or Claude Code config home (later
  runs load it, a base URL included), read and write the real `~/.codex` and `~/.claude`, and reach
  the network with a tool that ignores `HTTPS_PROXY`, so a second model called that way is not
  recorded. Bounded by: the wire leg records every model call that goes through the proxy, the
  chain and seal are the daemon's, and a tampered session file or home shows as a disagreement
  between the legs.
- **A shared CA row does not fail closed (WP12).** A connector that trusts the proxy through
  `SSL_CERT_FILE` or `REQUESTS_CA_BUNDLE` (`mode = "extend"`) hands that variable the system's
  roots plus the proxy CA, because every tool child reads it too. So if the harness itself skips
  `HTTPS_PROXY` for a connection, that connection verifies against the real roots and nothing
  records it. A harness's own variable gets the proxy CA alone, and a replace-mode one (Hermes,
  D41) makes such a connection fail. No shipped connector names a shared row. Bounded by: a
  connector names a shared row only when the harness has no CA variable of its own
  (CONNECTOR-SCHEMA, reviewer checklist); the wire leg records every model call that goes
  through the proxy; and the research report's `analysis.cross_leg` notes
  `native_activity_without_wire_streams` when the session file shows model work the proxy did not
  record.
- **An unpinned build may be anything on `PATH`** (a wrapper that unsets `HTTPS_PROXY`, a fake that
  writes a session file and calls no model). Bounded by: the recorded build digests (D53) and the
  judges comparing the native session's model turns with the wire leg's model calls. The research
  report's `analysis.cross_leg` notes `native_activity_without_wire_streams` when the session file
  shows model work and the proxy recorded no wire stream. `Launch.harness_version` is what the
  harness printed for `version_args`: reported, not proven.
- **The launch is not inode-bound execution.** The CLI resolves `program` on `PATH`, refuses a
  symlink or a non-regular file, and reads none of its bytes. A same-UID path replacement or an
  updater can change what `exec` runs. That is a different build, and the daemon's
  `Launch.harness_build` digests are the record of what ran. On macOS the image digest reads a
  path, so the same replacement is a residual there too.
- **One run per shared home, by a lock with two gaps.** `goalcup start` holds an exclusive lock on
  `capture-homes/<id>.lock` until its finish returns (D39). The lock dies with the launcher: if
  `goalcup start` is SIGKILLed while a registered harness keeps running, a new start can share the
  home with it. A by-hand login into that home takes no lock.
- **Shared-home validation has same-UID and ACL residuals.** The CLI checks leaf and target
  identity, owner and `0700` mode, twice. It does not inspect ACLs, and a same-UID process can
  replace the alias after the last check.

## Wire leg, finish and storage

- **Proxy loss is chained, but not counted per loss (H-16).** Each loss (fail-open tunnel, full
  table, opaque fallback, unsettled cut) is chained when it begins, so a lossy run reads
  incomplete. Gap counts count records, not individual losses. A tunnel whose ClientHello names an
  approved host is a loss. A tunnel to a host that is not approved stays opaque by design when its
  ClientHello names none, names an unapproved host, or is not TLS. Such a tunnel, and one to an
  authentication host, leaves no record in the chain; the `proxy admission:` line at the finish
  counts the completed ones (`tunneled_not_approved`, `tunneled_auth_host`). A tunnel still open
  at the stop is not counted, so `accepted` less `intercepted` can exceed the line. A connection
  the stop cut is counted as `cut`. A decided tunnel still open at
  stop (a dev server that outlived the harness) is cut without a loss.
- **The seal lists what it cannot read, and has no time bound inside a file walk.** A local run
  walks the workspace. A symlink, FIFO, socket or device is listed with the absent digest and never
  followed. A directory the agent made unreadable (`chmod 000`) is listed the same way and its
  contents are not: an absent digest on a directory means "contents unseen". A regular file is
  hashed whole, so a huge file or one on a stalled mount holds the run lock while it is read.
- **A clone is sealed by Git, and `.gitignore` hides files (D56).** A run on the server seals the
  commit and a digest of uncommitted and untracked work. Only `.gitignore` files hide a path.
  `.git/info/exclude` and `core.excludesFile` are outside the digest, so an agent could hide work
  with them. A clone whose checkout converts files (LFS, eol, smudge) does not digest to the empty
  value even when unchanged. If Git cannot be read the trace is incomplete, never invalid.
- **Two faults can leave a harness running on a sealed entry (H-10).** The spawn gate and the finish
  budget close the common windows. A CLI crash after the gate released the child, together with a
  raced manual `goalcup seal`, can still leave a harness running. It needs both faults.
- **A base64url run of 16 KiB or more can redact the rest of its JSON line (F-07).** The redactor
  holds 16 KiB and fails closed. This happens only when a rule prefix inside the run (`sk-`, `eyJ`,
  `AIza`, `glpat-`) lies more than 16 KiB before the run's end. Codex rollouts persist
  `encrypted_content` on reasoning items, so such runs are common. Re-measure on a long chain.
- **Redaction residuals (H-20).** A PEM block is at most 15 360 bytes after its `BEGIN`: an `END`
  must start within them. Without one, a lone `BEGIN` takes its bytes up to the first `"`, at most
  15 360, so in JSON it ends with its own string and the record still parses. Three edges remain.
  A key body longer than the bound (an RSA-16384 key printed by a tool is about 14.5 KB) leaks its
  tail. A key cut off before its `END` whose body goes on past a quote (SSE deltas that stop early)
  has only its first piece redacted. A lone `BEGIN` followed within the bound by another key's `END`
  is one block, across records. An invalid UTF-8 byte reads as `0` when the rules match, so it no
  longer ends a key.
- **The proxy credential reaches the provider, not the trace (H8).** The harness's
  `HTTPS_PROXY=http://goalcup:<32 hex>@127.0.0.1:<port>` holds a per-run secret. It is the only
  place the tracer hands that secret to the harness. An agent that runs `env`, or a harness that
  uploads its environment, sends it to the provider: the tracer records traffic and never
  rewrites it. The daemon knows the secret's exact value (D60) and removes it from every recorded
  body and metadata string: raw, every byte percent-encoded or `\u`-escaped, and inside base64 at
  any alignment in both alphabets, each with one line break allowed between any two characters
  (LF or CR LF, raw, or escaped with one or two backslashes, as a JSON string or JSON inside a
  JSON string carries it). So an escaped colon, a glued earlier match, `env | base64` wrapped at
  76 columns, and the same inside a Hermes tool result are caught. The shared rule
  `proxy_credential` (`goalcup:<secret>`, percent-encoded once, the Basic token) stays for a
  reader without the secret. **Not caught:** a double percent-encoding or any other transform of
  the value; base64 with `/` written as `\/`; three levels of escaping; two breaks in one gap; a
  fragment of the secret. Bounded by: the proxy listens on loopback only, and the secret ends
  with the run.
- **Codex identifiers left in the trace (TR1 #15).** The Codex connector removes `credits`, the
  installation id, `safety_identifier` and the `x-codex-turn-state` token, as measured on 0.153.0
  WebSocket traffic. It keeps `client_metadata.session_id`, `thread_id` and
  `x-codex-turn-metadata` (a JSON string of turn ids). The rows match a field name anywhere in a
  Codex trace, so the participant's own JSON with a `credits` object or a `safety_identifier`
  field in a tool result is redacted too.
- **Open assumptions (H-04).** One proxy per entry. The 8 MiB capture queue holds about 2 MiB of
  payload, so one stall can lose up to about 2.1 MiB. `seal.json` sits outside the reservation, so
  a failed seal write reads `incomplete` with no further policy.

## Provider keys (D61): what the tracer does not hold

The tracer never sees, forwards or stores a participant's provider key. A harness keeps its key
in its own home, from its own login. So there is no key-fragment scan and no `key-scan.txt`: the
tracer has no key to scan for. What keeps a key out of a trace is the daemon's redaction of what
the harness records (the shared rules and `masked_key`), and these shapes it does not see.

- **Hermes reads the participant's key from its own `$HERMES_HOME/.env`**, which the tracer never
  reads. A key printed raw or JSON-escaped is removed when a shared rule names its shape
  (`openai_key` for `sk-…`, `generic_secret_assignment` for `NAME_KEY=…`; `hermes_offline_key.rs`
  pins it through the trace, the upload chunks and the baseline). **Not caught:** a key the agent
  encodes before printing it (`base64`, hex, any transform), and a key of a shape no shared rule
  names, because the tracer never knows the value to derive the forms from. The base64 of a `.env`
  printed into the session reaches the trace and the upload as base64 text.
- **The harness's home is the participant's.** A key the harness stores there is not read by
  the tracer. An agent that prints that file into its session is recorded through the same
  redaction, no more.
- **"Stays local" is checked by local signal only.** A goal file under Git, `Library/Mobile
  Documents`, `Library/CloudStorage` or a folder named Dropbox, OneDrive, Google Drive or iCloud
  Drive is refused. `~/Desktop` and `~/Documents` are refused only when local state says iCloud
  owns them. No network call. A folder synced in a way those signals do not show is not caught.
- **`masked_key` is soft and has no boundary.** It never wins an overlap, so a mask glued to a
  longer secret is redacted by the longer rule. `spikes/redact.py` (fixtures only) may redact more
  than the daemon, never less.

## Termination

`Confirmed` is bounded to the owned process set (D30). A process that left the walk before the
first snapshot is not claimed. On Linux, `Confirmed` can stand while an orphan the walk cannot see
is running. `goalcup start` sweeps its own children there before the seal, but the trace does not
record that, and the sweep does nothing if `goalcup start` itself dies. Read
[TERMINATION](TERMINATION.md) before you trust a termination outcome.
