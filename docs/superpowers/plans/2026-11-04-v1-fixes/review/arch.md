# arch lane notes (in progress)

## Map
- goalcup-format: GCR1 framing/records/links/parts/reader/dump (sync, no TLS)
- goalcup (32k LOC src): cli/ (8 cmds), watchdog/ (start proc: binds port, CA, harness spawn, recorder restart/null mode), pipeline/ (writer, mask_worker, counts, seal, dead), proxy/ (CONNECT, tls, http1, h2, ws, decode, tunnel), native/ (tail/snapshot session files), mask/ (stream masker 2.7k), upload/ (3 threads, ureq), connector/ (toml loader, launch, hints, hosts)
- goalcup-verify: check/dump/secrets/usage (usage/native.rs reads harness session formats by connector id)
- server/worker: CF Worker TS, chunks+heartbeats, local db + supabase db
- connectors/*.toml + model-hosts.toml + tiers.toml compiled into binary
- scripts/pad: python test pad (check.py, pad, pad-run, sbx.mjs)
Flow: harness -HTTPS_PROXY-> proxy (relay first, copy) -> held-bytes guard -> masker -> writer -> part files -> uploader -> Worker; verify fetches later.

## Findings (top first)
- arch-1 per-harness knowledge in 5 places: connectors/<id>.toml, goalcup-verify/src/usage/hints.rs:30-150 (reader arms, WIRE_IDS, join_of, thread_db keyed by connector id), scripts/pad/harnesses/<id>.toml (version/install), connectors/tiers.toml, docs/harnesses/<id>.md. New harness w/ wire thread headers needs Rust in verifier. Fix: move WIRE_IDS/join/thread_db/row->reader into connector `[usage]` table; verify include_str's connectors.
- arch-2 module graph is a full mesh: pipeline->watchdog (16 refs), upload->cli (7), watchdog->cli (10), pipeline->cli; cli/ is the de-facto util lib (log, read_json, status_json, proc_start_id, alive, ServerJson). Fix: extract `run` (paths/json/log/proc identity) + `ctl` (StartFacts, SealReason, Exit, ToRecorder/ToWatchdog) modules.
- arch-3 cross-crate coupling hidden in include_str: goalcup-verify/src/secrets.rs:22 includes ../../goalcup/src/mask/shapes.tsv; secrets.rs:23-32 copies masker's ends_an_escape regex + GUARD=24 ("SEAM NOTE ... this copy"). Breaks the stated 'share only goalcup-format' rule silently. Fix: shapes.tsv + guard consts into goalcup-format (data only) or connectors/.
- arch-4 dead: scripts/check-links.py (1349) + scripts/test_check_links.py (1672) referenced by nothing (gate.sh, ci.yml, release.yml, README) - 3k lines. Cut.
- arch-5 server dual DB impl: server/worker/src/db-local.ts (654, tests/dev) vs db-supabase.ts (412) -> SQL bodies living in website repo, mirrored as 4468-line sql/goal-cup-tracer-upload.sql. integration/lifecycle tests run on LocalDb only; supabase.test.ts stubs fetch. Prod atomicity semantics never tested here.
- arch-6 tiers.toml compiled into binary (cli/start.rs:26) while written by pad after release; ships 16x 'not verified' version ''. Tier/version facts need a release to change. (agree with versions lane)
- arch-7 recovered watchdog runs as hidden `goalcup recorder` subcommand (pipeline/mod.rs:799-803 -> watchdog::run_recovered): two processes named `goalcup recorder`, one being a watchdog; recorder entry point owns watchdog dispatch. Fix: hidden `watchdog` subcommand.
- arch-8 version is encoded 5 ways: connector `version` prefix (refuse), `version_notes` (exact), tiers row version (exact), pad harnesses version pin, connector header comment; opencode vs opencode2 is a whole duplicated connector differing by 6 keys.
- arch-9 failure domain: uploader + proxy + masker share the recorder process; a recorder restart (crash/write error/D78) drops every in-flight harness TCP stream (watchdog keeps only the listener). Point 3 cost; consider relay surviving in watchdog later (post-event).


## Re other lanes
- arch-10 null-mode retry kills harness streams: watchdog/mod.rs:793-806 sends Restart on each null wait (30s,1,2,5,5.. min); pipeline/mod.rs:995 null recorder returns at once -> every relayed SSE/WS connection closes each retry while normal boots keep failing. ARCHITECTURE.md:95 "The harness keeps its connections" is false. Fix: null recorder stops accepting, drains open conns (or hands listener back) before exit.
- docs-7 agree, same root as arch-1: builtin.rs list (connector/builtin.rs:8-25) makes a "data-only" connector PR a code PR. Fix: build.rs globbing connectors/*.toml.
- cr-watchdog respawn-forever: agree; same failure-domain theme as arch-10 (watchdog loop has no exit while restart path stuck).
- versions: agree on all four; arch-8 adds that version facts live in 5 files; recommend one `[[versions]]` table in the connector as single source (pad pin, tier, notes, refusal derived).
- oss-3 agree: docs/plans 7.7M dwarfs code (crates/goalcup/src 32k LOC).
- usability-11 agree: root cause is arch-8 (version split = duplicated connector id). One connector with per-version rows lets goalcup pick the row from `--version` and kills the wrong-pick refusal.
- usability-2 agree; connector `program` is already data - accepting program name as alias is a 3-line lookup in connector/mod.rs.
## Inbox: none received by finish.

## Answers
- inbox-arch.md: no questions received (file absent at round 2).

## Round 2 (cross-talk)
- arch-x1 [important, critical on Linux] Recorder respawns re-exec the binary by path (watchdog/mod.rs:269 current_exe -> Boot::start Command::new(exe), mod.rs:569 loop). install.sh rename-replace mid-run (cr-scripts-2): Linux current_exe = "/…/goalcup (deleted)" -> every respawn ENOENT -> cr-watchdog-1 loop spins forever, listener never accepted, harness model calls hang (point 3). macOS: path now holds the NEW binary -> mixed-version watchdog/recorder, StartFacts JSON across versions, no version handshake (boot.rs:248 records recorder's own CARGO_PKG_VERSION only). Fix: at watchdog start hardlink/copy own exe into entry dir (or keep an O_RDONLY fd and spawn via /proc/self/fd|fexecve), put CARGO_PKG_VERSION in the first control line and refuse a mismatch; install.sh then can't break a live run.
- arch-x2 [important] No egress layer: cr-proxy-1 (CA roots dropped, ca.rs:352), cr-proxy-2 (HTTPS_PROXY removed, launch.rs:19-31, proxy upstream dials direct), cr-upload-server-3 (client.rs:221 try_proxy_from_env(false), webpki roots only) are one design gap: goalcup assumes a direct internet path on 3 legs. Corporate/TLS-inspected networks fail on all three. Fix: one `net` module (upstream proxy from the participant's saved env + system/extra roots) used by both proxy upstream and ServerClient; `goalcup setup` probes it once and says so.

## Re other lanes (round 2)
- cr-watchdog-1 agree, severity stays important alone, but chained with cr-scripts-2 it blocks the harness (see arch-x1).
- cr-scripts-2 severity-up: on Linux it leads to arch-x1/harness hang, not just a swapped binary.
- cr-proxy-1/cr-proxy-2/cr-upload-server-3 agree; root cause arch-x2.
- cr-format-verify-1 agree + evidence: goalcup-verify include_str's only shapes.tsv (secrets.rs:22), never model-hosts.toml, so it cannot recompute host labels even if it wanted to. Fix together with arch-2/arch-8: a data-only crate (connectors/*.toml, model-hosts.toml, shapes.tsv) both binaries depend on.
- cr-format-verify-5, versions-9 duplicate of arch-2 (hints.rs:30 reader by connector id).
- cr-upload-server-4 duplicate of arch-3.
- cr-watchdog-4 / usability-12 duplicate of arch-7 (root: tiers compiled in).
- versions-8 duplicate of docs-2 (RULES.md:208 version sampling).
- versions-4, usability-11, docs-1, versions-7 share root arch-6 (opencode2 cloned connector); one connector with [[versions]] rows fixes all four.
- versions-5 agree: hermes-config.yaml:11-16 gives the pad a CA route participants lack while hermes-cli.toml:42-44 tells participants 0.22.6 cannot work - pad green != participant green. Fix: move the providers.ssl_ca_cert block into the connector's settings rows for 0.22.6 (needs per-version rows, arch-6) or drop it from the pad.
- cr-proxy-3 agree on design: refusal table is per boot, in memory (hosts.rs:1-19), so every recorder restart and every 600 s expiry re-probes with a failed call; persisting refusals per entry dir would cut it.
