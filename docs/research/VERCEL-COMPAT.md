# Goal Cup Proxy on Vercel Sandbox and Vercel AI Gateway

> **Superseded by [D74](../DECISIONS.md#d74-the-model-host-is-the-connectors-default-or-the-one-address-the-harnesss-own-config-names) for Claude Code and Codex** (28 Oct 2026). Both connectors now read
> the gateway address from the harness's own config, and fall back to their default hosts when it
> names none. No `vercel` row is added to `providers.toml`. The §2 rows below are corrected;
> the measured runs are in [the 28 Oct results](results/2026-10-vercel-gateway.md).

22 Oct 2026. Desk research only: nothing below has been run on Vercel. Vercel facts come from
vercel.com/docs, read today, with the page named on each row. Goal Cup facts come from this repository,
with the file named. **Unverified** means no one has measured it.

## Verdict

1. **Sandbox: probably yes, for practice runs. Not yet for a 168-hour run.** A Vercel Sandbox is
   a Firecracker microVM on x86_64 Linux with root, 64 GB of disk and open egress. That fits Goal Cup
   Proxy on paper. But one session lasts at most 24 hours, and a stop keeps the files only, not the
   processes. So a 168-hour run needs at least six forced restarts.
2. **AI Gateway: only Hermes and Amp are recorded today.** Hermes records whatever host its own
   config names (D62). Amp still talks only to `ampcode.com`. Claude Code and Codex pointed at
   `ai-gateway.vercel.sh` would send their model traffic through an opaque tunnel, with no wire
   record. Fixing that is a reviewed connector or core change, not a setting.
3. **The two blockers to check first:** the filesystem type of the sandbox disk (goalcup needs
   ext4 or XFS), and what a 24-hour session stop does to a running trace. Both are unknown. The
   30-minute recipe below tests both.

## 1. Sandbox fit

Sources: [Sandbox](https://vercel.com/docs/sandbox), [Concepts](https://vercel.com/docs/sandbox/concepts),
[Pricing and limits](https://vercel.com/docs/sandbox/pricing),
[Persistence](https://vercel.com/docs/sandbox/concepts/persistent-sandboxes),
[Duration guide](https://vercel.com/kb/guide/vercel-sandbox-duration-and-persistence),
[Images](https://vercel.com/docs/sandbox/concepts/images), [Drives](https://vercel.com/docs/sandbox/concepts/drives),
[Firewall](https://vercel.com/docs/sandbox/concepts/firewall),
[universal image](https://github.com/vercel/sandbox/tree/main/images/universal).

| Goal Cup requirement (where) | Vercel Sandbox fact (source) | Fit | What to test |
|---|---|---|---|
| macOS or Linux, arm64 or x86_64 (`README.md` Platforms) | Firecracker microVM with its own kernel. Default image is Ubuntu 26.04. Images must be `linux/amd64` (Concepts, Images). | Yes, but on the weakest platform | Linux x86_64 is the least measured Goal Cup platform: a no-login smoke on a hosted runner, no logged-in run, no run over 2 hours (`README.md` Platforms, `TODO.md` Linux). Run a logged-in practice first. |
| Trace folders on APFS, ext4 or XFS. `goalcup setup` fails on overlayfs, btrfs, ZFS, NFS, SMB, tmpfs, FUSE; `start` warns (`crates/goalcup-cli/src/folder_gate.rs`, `README.md`) | Each sandbox gets "64 GB of ephemeral NVMe storage". The filesystem type is not documented. Drives are a separate mount, type not documented (Pricing, Drives). | **Unknown** | `findmnt -no FSTYPE -T "$HOME"` and `df -T`. If it is overlay, try a loop-mounted ext4 image with `sudo` (root is available). Drives: same check. |
| A 3 GiB reservation by `fallocate`; warns under 5 GiB free; pauses the agent under 1 GiB (`crates/goalcup-platform/src/disk.rs`, `docs/PARTICIPANT-GUIDE.md`) | 64 GB disk on every plan (Pricing). | Yes, if the filesystem is ext4/XFS | `goalcup setup` step 4 (`machine`) says `[ok]`. |
| 168 hours on the server's clock; the harness keeps running (`docs/RULES.md`) | Max session: **45 minutes on Hobby, 24 hours on Pro and Enterprise**. Default timeout 5 minutes; `extendTimeout()` up to the plan maximum (Pricing). A stop snapshots the **filesystem**; nothing says memory or processes survive (Persistence, Duration guide). | **No, as is** | A 168-hour run needs at least seven sessions. Each boundary kills goalcup, the daemon and the harness. Test a stop and resume mid-run (recipe step 7). Ask Vercel for longer sessions. |
| After a crash or reboot, `goalcup continue` closes the dead segment with one `IngestLost` gap; that segment verifies exit 1, the next one is clean; a torn tail is exit 2 (`docs/OPERATING.md` drill 5) | Unknown whether a session stop is a clean shutdown (signals, `sync`) before the snapshot (Concepts says only "the session shuts down"). | **Unknown** | Stop the sandbox during a practice run, resume, run `goalcup continue`, verify both parts. A cleaner path: `goalcup seal <part>` from a second terminal before the limit, then `continue` after resume. |
| A terminal the participant types into; it stays open (`README.md` During the run) | `sandbox exec --interactive --tty <name> -- bash`. tmux is preinstalled in the universal image (Concepts, universal image). Detached commands exist; nothing says whether a dropped `exec` connection kills its processes. | Probably, with tmux | Run goalcup inside tmux, drop the `exec` connection, reattach. |
| Sleep lock via `systemd-inhibit` on Linux; without one the step still passes with a warning (`crates/goalcup-platform/src/awake.rs`, `docs/PARTICIPANT-GUIDE.md`) | No sleep concept in a microVM. systemd presence not documented. | Yes | Expect a `PROBLEM:` line about sleep; harmless. |
| Egress to the model host, GitHub and the Goal Cup server (`docs/RULES.md`) | Default network policy is `allow-all`. Domain filtering is by TLS SNI, without decryption (Firewall). | Yes | None for the default policy. |
| The recording proxy checks upstream TLS against the Mozilla roots compiled into `webpki-roots` only (`crates/goalcup-cli/src/proxy/tls.rs`) | The firewall **terminates TLS** for any domain with a `transform` (credential brokering) or `forwardURL` rule, using a per-sandbox CA added to the system store (Firewall, Concepts). | **No, if brokering is used** | If Vercel injects the gateway key at the firewall, goalcup will refuse the firewall's certificate and every model call fails. Keep brokering off for the model host, or change goalcup (core change, needs a DECISIONS entry). |
| Root not required; runs as the user (`README.md`, install into `~/.local/bin`, no sudo) | Default user `ubuntu` (uid 1000) with passwordless sudo (universal image README). Full root via `sudo` (Concepts). | Yes | None. |
| Codex's own sandbox (`bwrap`) needs user namespaces; in Docker it failed: `bwrap: No permissions to create a new namespace` (`docs/research/results/2026-10-22-local-test-day.md`) | Dedicated kernel; Docker and FUSE run with `sudo` (Concepts). Unprivileged user namespaces not documented. Ubuntu 26.04 may restrict them through AppArmor (**unverified**). | **Unknown** | `unshare -Ur true` as `ubuntu`. Either way, D67's `sandbox_mode = "danger-full-access"` avoids it (`docs/HARNESS-SETUP.md`). |
| Harness installed as a native binary first on `PATH`, not an npm wrapper (`README.md`) | Universal image preinstalls Claude Code, Codex, OpenCode and pi, updated nightly (universal image, Images). Not Amp or Hermes. | Partly | Install the measured native builds yourself; pin the image by digest so the harness does not change between sessions. |
| CPU and memory (no stated minimum) | Pro: up to 8 vCPU and 16 GB; 2 GB per vCPU; default 2 vCPU (Pricing). | Yes | 4 vCPU / 8 GB is a fair default. |

**Cost, rough.** Pro rates in `iad1`: Active CPU $0.128/hour, memory $0.0212/GB-hour (Pricing).
One 168-hour run at 4 vCPU and 8 GB: memory 1,344 GB-hours = $28.49. CPU at 10% busy: 67.2
hours = $8.60; at 30%: 201.6 hours = $25.80. So about **$37 to $54 per participant**, plus snapshot
storage at $0.08/GB-month. An estimate from list prices, not a quote.

## 2. AI Gateway fit, per harness

Gateway facts. Sources: [AI Gateway](https://vercel.com/docs/ai-gateway),
[Coding agents](https://vercel.com/docs/ai-gateway/coding-agents),
[Anthropic Messages](https://vercel.com/docs/ai-gateway/sdks-and-apis/anthropic-messages-api),
[Claude Code](https://vercel.com/docs/ai-gateway/coding-agents/claude-code),
[Codex](https://vercel.com/docs/ai-gateway/coding-agents/openai-codex),
[Hermes](https://vercel.com/docs/ai-gateway/coding-agents/hermes),
[Amp](https://vercel.com/docs/ai-gateway/coding-agents/amp).

- One host: `ai-gateway.vercel.sh`. OpenAI Chat Completions at `/v1/chat/completions`, Responses at
  `/v1/responses`, Anthropic Messages at `/v1/messages`.
- Per-agent bases: Claude Code `https://ai-gateway.vercel.sh/claude-code`; Codex
  `https://ai-gateway.vercel.sh/codex/v1`; others `https://ai-gateway.vercel.sh/coding-agent/v1`.
- Auth: an AI Gateway API key, as `Authorization: Bearer` or `x-api-key`. Budgets per key are soft
  caps.
- Streaming is supported. Response compression is not documented. Goal Cup now decodes gzip,
  brotli and zstd replies (fix plan v2, `docs/research/results/2026-10-22-local-test-day.md`), so
  compression should not matter (**unverified** against the gateway).

The Goal Cup rule. The only wire path is the loopback recording proxy, reached through the
harness's `HTTPS_PROXY` (D1, D36). The proxy decrypts only the hosts a connector declares; every
other host is an opaque tunnel, counted and never recorded (D1, D62). Goal Cup never sets a base
URL. The harness's environment is built from nothing, so a base URL or key exported in the shell
never reaches it (D61; `crates/goalcup-cli/tests/it/prep.rs`). A base URL must live in the
harness's own config in its Goal Cup home.

| Harness | How it points at the gateway | Recorded today? | Change needed |
|---|---|---|---|
| **Hermes** (`hermes-cli`) | Built-in `ai-gateway` provider, key `AI_GATEWAY_API_KEY` in `.env`; or a custom endpoint `https://ai-gateway.vercel.sh/v1` (Hermes page). | **Yes, if `model.base_url` names the gateway.** The connector reads `config.yaml` `model.base_url` at every start (D62, `connectors/hermes-cli/harness.toml`). Paths under `<base>/chat/completions` get the `openai-chat-completions` hint. | None for a custom endpoint. **Unverified** for the built-in `ai-gateway` provider: Hermes may take its runtime URL from `AI_GATEWAY_BASE_URL` or its registry, not from `config.yaml`. If `model.base_url` is empty, `goalcup start` refuses (safe). If it names one host and Hermes uses another, calls fail TLS under the replace-mode CA (known gap, `docs/harnesses/hermes-cli.md` section 9). Test both. |
| **Amp** (`amp-cli`) | Set in Amp's web settings (Model Routing). Early access for Megawatt and Gigawatt members only (Amp page). | **Probably yes, unchanged.** The Amp client talks to `ampcode.com`, which the connector records (`connectors/amp-cli/harness.toml`). The gateway call would be made by Amp's servers (**inference**, not stated by Vercel). | None, if the client still talks only to `ampcode.com`. Confirm in a practice run: `proxy admission:` shows no new tunnelled host. |
| **Claude Code** (`claude-code-cli`) | `ANTHROPIC_BASE_URL=https://ai-gateway.vercel.sh/claude-code` plus `ANTHROPIC_AUTH_TOKEN` (or a Claude subscription plus `ANTHROPIC_CUSTOM_HEADERS`). `vercel ai-gateway setup` writes them into `settings.json` `env` (Claude Code page). | **Yes, measured 28 Oct ([Round 3](results/2026-10-vercel-gateway.md#round-3-28-oct-one-capture-system-sandbox)): `wire_and_native`, 15 requests, Claude Code 2.1.312.** Before D74: 0 wire requests, the gateway tunnelled. The connector now reads `env.ANTHROPIC_BASE_URL` in its `settings.json` (`absent = "defaults"`) and keeps `providers = ["anthropic"]`. The `/messages` paths get the `anthropic-messages` hint. | Done: option (b), `model_host_from` with `absent = "defaults"`; `env` is no longer a deny word. |
| **Codex** (`codex-cli`) | `model_provider = "vercel"` with `[model_providers.vercel] base_url = "https://ai-gateway.vercel.sh/codex/v1"`, `env_key = "AI_GATEWAY_API_KEY"`, `wire_api = "responses"` (Codex page). | **Yes, measured 28 Oct ([Round 3](results/2026-10-vercel-gateway.md#round-3-28-oct-one-capture-system-sandbox)): `wire_and_native`, 17 requests, Codex 0.165.0.** The connector reads `model_providers.*.base_url` via `model_provider` (`absent = "defaults"`) and keeps `chatgpt.com`. The `/responses` paths get the `openai-responses` hint. | Done: option (b), with the fallback. Key: in `$CODEX_HOME/.env` (Goal Cup's `codex-cli/.env`), which Codex reads itself (measured). Codex posted over plain HTTPS, no WebSocket. |

Rule questions for the owner, not technical blockers: is a Vercel-routed model service "the model
service the harness's own config names" (D62) for Claude Code and Codex too? (D74: yes.) And
would a run whose model traffic tunnelled unrecorded count as "routing work around Goal Cup Proxy" (`docs/RULES.md`)?

## 3. Open questions to ask Vercel

1. What filesystem is the sandbox's 64 GB disk, and what is a Drive (ext4, XFS, overlay, network,
   FUSE)? Does `fallocate(0)` reserve real blocks on it?
2. Can a sponsored team get sessions of 170 hours or more? If not, what exactly happens at the
   24-hour limit: a signal to processes, a grace period, `sync`, then snapshot? Can a running
   session be snapshotted with its memory?
3. When the `sandbox exec --interactive --tty` connection drops, do its processes keep running?
4. Are unprivileged user namespaces enabled for the default `ubuntu` user (Codex uses `bwrap`)?
5. Is there an arm64 option? Goal Cup is better measured on Linux arm64.
6. Does the default `allow-all` policy ever terminate TLS? Any transparent proxy on egress?
7. Can each participant get their own AI Gateway key with a hard budget, without firewall
   credential brokering? Brokering breaks Goal Cup's upstream TLS check (section 1).
8. Does the gateway send compressed responses (`br`, `zstd`) to these agents?
9. Amp through the gateway: does the Amp client ever call `ai-gateway.vercel.sh` itself, or only
   `ampcode.com`? Is early access open to Goal Cup participants?
10. Who owns each sandbox: the participant's own Vercel team, or one Vercel runs for the event?
    Goal Cup rules allow a cloud machine the participant controls or the organizers provide.

## 4. A 30-minute test recipe

For a Vercel engineer with a Pro team, a GitHub account and an AI Gateway key. It needs a Goal
Cup account (`goalcup login`; ask the organizers) and a new, empty, public GitHub repository with a
fine-grained token. Practice runs last about ten minutes, never push and never count.

1. **Create** (laptop, ~2 min):
   ```sh
   sandbox create --name gc-test
   sandbox config vcpus gc-test 4
   sandbox config timeout gc-test 2h
   sandbox exec --interactive --tty gc-test -- bash
   ```
2. **Facts** (in the sandbox, ~2 min). Send the output back:
   ```sh
   uname -a; nproc; free -g; df -T "$HOME" /tmp
   findmnt -no FSTYPE -T "$HOME"
   unshare -Ur true && echo userns-ok
   command -v systemd-inhibit || echo no-inhibit
   ```
3. **Install** (~2 min). Start tmux first so the run survives a dropped connection:
   ```sh
   tmux new -s gc
   curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
   export PATH="$HOME/.local/bin:$PATH"
   goalcup --version
   ```
4. **Log in and check** (~3 min):
   ```sh
   GOALCUP_NO_BROWSER=1 goalcup login   # open the printed address on the laptop, approve
   goalcup doctor --harness hermes-cli
   ```
   Hermes is the one harness the gateway is recorded for today. Install it as its docs say, native
   binary first on `PATH`.
5. **Setup** (~5 min):
   ```sh
   goalcup setup --harness hermes-cli
   ```
   Setup opens Hermes's setup. Choose a custom OpenAI-compatible endpoint with base URL
   `https://ai-gateway.vercel.sh/v1` and your AI Gateway key. Repeat `goalcup setup` until its four
   steps say `[ok]`. The last (`machine`) checks disk space and the folders' filesystem. If it fails
   on the filesystem, stop here and send the output: that is blocker 1. A real run's repository,
   token and project folder come later (the participant's guide, section 5); a practice run needs
   none.
6. **Practice** (~10 min):
   ```sh
   goalcup start --harness hermes-cli --practice
   ```
   Type a small prompt, for example "Write a tested fizzbuzz in Python and commit it." Note the
   trace folder that `start` prints. Check from a second tmux window with `goalcup status`.
7. **Session boundary** (~5 min, optional but important). Start a second practice. After about
   three minutes, on the laptop: `sandbox stop gc-test`. Then reattach with step 1's `exec`, run
   `tmux new -s gc`, `export PATH="$HOME/.local/bin:$PATH"` and `goalcup continue`. Note both trace
   folders.
8. **Verify** (~1 min) for each trace folder:
   ```sh
   goalcup-verify --capture-integrity <trace-dir>
   goalcup-verify --research-json <trace-dir> --output "$HOME/research.ndjson"
   ```
   Pass: step 6 exits 0 with `integrity: valid`. The `proxy admission:` line at the finish shows
   `intercepted=` above 0, and `ai-gateway.vercel.sh` is not among the tunnelled hosts. Step 7: the
   first part exits 1 with one `IngestLost` gap; exit 2 means a torn tail, which must be reported.
   Send the verifier output and `goalcup status`, never the raw trace.

To test Claude Code or Codex through the gateway, run the same practice with `--harness
claude-code-cli` or `codex-cli`. Today the expected result is no wire record for the gateway host:
that confirms the connector change in section 2 is needed.
