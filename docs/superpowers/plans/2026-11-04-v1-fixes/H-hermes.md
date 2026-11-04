# Lane H: Hermes on several versions, for participants
**Wave:** 1 (H.1, H.2) and 2 (H.3 to H.7)   **Depends on:** none for wave 1; wave 2 on lane G `here_path` (Mac rows per version, requested below) and G.20/G.23 (versions list)   **Branch:** fix/hermes
**Covers findings:** versions-x2, versions-5, MULTI-VERSION-PLAN tasks 1 and 13 (and versions.md V6, the same finding)
**Owns files:** connectors/hermes-cli.toml, docs/harnesses/hermes-cli.md, scripts/pad/harnesses/hermes-cli.toml, scripts/pad/harnesses/{direct,gateway}/hermes-config.yaml, crates/goalcup/tests/connector_hermes_rows.rs; new run files scripts/pad/runs/hermes-spike-linux.toml, hermes-spike-mac.toml, core-hermes-mac.toml (lane G owns the folder; it edits none of these three)
**Shared files (only these parts):** crates/goalcup/src/connector/schema.rs (one field after line 46; the loop at 314-321); crates/goalcup/src/connector/mod.rs (load(): new lines between 207 and 208; plan(): two lines after 301); crates/goalcup/tests/connector_no_model_address.rs (two arrays, 18-28 and a new test at the end); crates/goalcup/tests/connector_launch.rs (one test appended after 266); docs/CONNECTOR-SCHEMA.md (one table row after 40, one clause in 60; lane L owns the file)
**Produces (for other lanes):** connector key `ca_settings: Vec<SettingRow>` (`{file, key, value}`, written at every start and continue, never by `goalcup setup`). Lane I: `ca_settings` is an ordinary override key (allowed in a `[[versions]]` row, like `update_off`); Hermes uses no `[[versions]]` row. Lane G: G.24 is superseded by H.5; G.5 keeps the `pad_only` mechanism but adds no Hermes data (H.1 deletes the block for every version).
**Consumes:** lane G: harness-file key `here_path = "<absolute dir, may hold {version}>"`, put first on `PATH` for `version_cmd` (pad-run:314) and in `gc_env(clean=True)` (pad-run:307) when `where = "here"`; G.20 `versions` list; G.23 `scripts/pad/runs/matrix-hermes.toml`. Lane F: tier line for an unread or other build (F-version-core.md:96-99), unchanged by this lane.

## Dropped
- "0.22.6 drops SSL_CERT_FILE; 0.22.5 is unaffected" (MAC-LEARNINGS.md:19, docs/harnesses/hermes-cli.md:52, connectors/hermes-cli.toml:42-44): wrong cause. `agent/ssl_verify.py` is byte-identical from v0.21.6 (commit 1c8fae6180) through v0.22.5, v0.22.6 and v0.22.7. `resolve_httpx_verify` returns `_shared_context(None)` when `SSL_CERT_FILE` is set (ssl_verify.py:341-342), and on macOS truststore verifies with Security.framework plus only the certificates loaded into the context (truststore `_macos.py:440-450`), so the run CA is never trusted. On Linux truststore calls `set_default_verify_paths()`, which reads `SSL_CERT_FILE` (`_openssl.py:25-38`), so every version works there. It is an OS split, not a version split. D72 (DECISIONS.md:1049-1055) measured 0.22.5 on macOS needing the trust entry on 26 Oct, and PRD-13:107 cut that entry. The plan below fixes the real cause.
- MULTI-VERSION task 1, "measure REQUESTS_CA_BUNDLE, HTTPX_CA_BUNDLE, SSL_CERT_DIR on 0.22.6": settled by source, not measured. Model clients get an explicit `verify` context (agent_runtime_helpers.py:1979-1983 at v0.22.6), so httpx reads no CA variable; `REQUESTS_CA_BUNDLE` reaches only `requests` calls (ssl_verify.py:246); `SSL_CERT_DIR` takes the same branch as `SSL_CERT_FILE`. No Hermes variable or launch argument sets provider trust; only a `providers:`/`custom_providers:` entry whose `base_url` equals the route (config_providers.py:368-392, 446-472).
- MULTI-VERSION task 13, "a 0.22.6 `say`+`gap` row": not needed; every version gets the same rows.
- versions-x2 "table refuses verified for versions in `version_notes`": the note is deleted (H.5), so there is nothing to refuse.

## Review focus
1. A participant `config.yaml` with `providers: {}` (flow style) must skip the row and log `ca_settings not written`, never refuse or crash. Test in H.4.
2. A `ca_settings` value naming loopback, plain `http`, or a host outside the shared list, or any key not under a `goalcup-*` entry (for example `model.base_url`), must fail the load as a model address. Test in H.3.
3. A second start on the same home must not rewrite `config.yaml` (bytes equal). Test in H.4.
4. Every Hermes route row has its `ssl_ca_cert` twin and every route host is a shared-list host, so the trust entry never adds a decrypted host. Test in H.5.
5. The rows must not mark Hermes as configured: the first-run guard counts `model.provider/base_url/api_key` and keys, not `providers:` entries (hermes_cli/main.py:1094-1180 at v0.22.6). Checked in H.7 step 2 on a fresh home.

### Task H.1: Delete the pad-only CA block   (size S, PARALLEL-OK)
**Files:** Modify scripts/pad/harnesses/direct/hermes-config.yaml:11-16, scripts/pad/harnesses/gateway/hermes-config.yaml:11-16 (delete both blocks), scripts/pad/harnesses/hermes-cli.toml:6-9 (comment: "No CA setting: goalcup's own ca_settings rows (connectors/hermes-cli.toml) give Hermes the run bundle")
**Interfaces:** none
- [ ] Step 1: Verification (data only): `grep -n "ssl_ca_cert\|goalcup-ca\|providers:" scripts/pad/harnesses/*/hermes-config.yaml`
- [ ] Step 2: Before the edit expect 6 lines (3 per file).
- [ ] Step 3: Delete lines 11-16 in both files; edit the comment.
- [ ] Step 4: Run the same grep; expect no output. Run `python3.11 -m unittest discover -s scripts/pad -p 'test_*.py'`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad/harnesses; git commit -m "pad: Hermes config without the pad-only CA block"`

### Task H.2: Spike: the CA failure is macOS, every version   (size S, PAD)
**Files:** Create scripts/pad/runs/hermes-spike-linux.toml (`harness = "hermes-cli"`, `key = "deepseek"`, `case = "direct"`, `mode = "micro"`, `where = "sandbox"`), scripts/pad/runs/hermes-spike-mac.toml (same, `where = "here"`, `build = "target/release"`)
**Interfaces:** Consumes H.1 (block gone). Mac 0.22.5 and 0.22.7 rows also consume lane G `here_path`; without it, step 3 is skipped and the source basis stands.
- [ ] Step 1: Build on the lane branch (H.1 only, no Rust change): `PATH="$HOME/.cargo/bin:$PATH" cargo build --release -p goalcup -p goalcup-verify`.
- [ ] Step 2: Run `scripts/pad/pad scripts/pad/runs/hermes-spike-linux.toml` (image pin 0.22.5, linux-x86_64). Pass: `C1` PASS, `facts.wire_requests >= 1`.
- [ ] Step 3: Owner step, once per version, never touching `~/.hermes` or `~/.local/bin`: `mkdir -p /Users/team/dev/harness-installs/hermes-$V && curl -fsSL https://raw.githubusercontent.com/NousResearch/hermes-agent/v$V/scripts/install.sh | HOME=/Users/team/dev/harness-installs/hermes-$V bash -s -- --skip-setup --skip-browser --skip-computer-use --commit $C` with (0.22.5, bf0203a794a07aac3ff764972ab9087baa4e4f4e), (0.22.7, b200f5bc60e31588ba2cb60e288029c4a54ba5fa). Then, with `here_path = "/Users/team/dev/harness-installs/hermes-{version}/.local/bin"` in the pad harness file and `version = "<V>"` in a copy of the Mac spike file, run `scripts/pad/pad <copy>`.
- [ ] Step 4: Run `scripts/pad/pad scripts/pad/runs/hermes-spike-mac.toml` (the owner's installed 0.22.6). Each Mac row is expected to FAIL: `facts.threads == 0`, `facts.wire_requests` in (None, 0), and `grep -rEil "certificate.verify.failed" pad-out/hermes-cli/direct/<run>/watch` non-empty.
- [ ] Step 5: Decision. Linux PASS and every Mac row FAIL as above: wave 2 as written (base rows, no version row). A Mac 0.22.5 row that reaches the model: the cause is a version split after all; H.5 then puts `ca_settings` in `[[versions]] match = ">=0.22.6"` (lane I) and the rest is unchanged. Commit the run files: `git add scripts/pad/runs/hermes-spike-*.toml; git commit -m "pad: Hermes CA spike runs"`; the rows go to lane G's rows file only through `table`.

### Task H.3: `ca_settings` key and its loader rules   (size M)
**Files:** Modify crates/goalcup/src/connector/schema.rs (field after line 46; loop 314-321), crates/goalcup/src/connector/mod.rs (between 207 and 208); Test crates/goalcup/tests/connector_no_model_address.rs
**Interfaces:** Produces `pub ca_settings: Vec<SettingRow>` in `struct Connector` (doc: "Trust rows written into the isolated home at every start and continue, never by setup. A value may be an `https://` address only on a `base_url` key under a `goalcup-*` entry, and its host must be a shared-list host.")
- [ ] Step 1: Write the failing tests. Add to `refused` in `every_refused_shape_fails_load`:
  ```rust
  "ca_settings = [{ file = \"{home}/c.yaml\", key = \"providers.goalcup-trust-x.base_url\", value = \"http://127.0.0.1:9\" }]",
  "ca_settings = [{ file = \"{home}/c.yaml\", key = \"providers.goalcup-trust-x.base_url\", value = \"https://evil.example.com/v1\" }]",
  "ca_settings = [{ file = \"{home}/c.yaml\", key = \"model.base_url\", value = \"https://api.deepseek.com/v1\" }]",
  ```
  New test:
  ```rust
  #[test]
  fn a_trust_entry_for_a_listed_host_loads() {
      let body = "ca_settings = [\
          { file = \"{home}/c.yaml\", key = \"providers.goalcup-trust-x.base_url\", value = \"https://api.deepseek.com/v1\" },\
          { file = \"{home}/c.yaml\", key = \"providers.goalcup-trust-x.ssl_ca_cert\", value = \"${env:SSL_CERT_FILE}\" }]";
      let c = load_text(body).unwrap();
      assert_eq!(c.ca_settings.len(), 2);
  }
  ```
- [ ] Step 2: Run `cargo nextest run -p goalcup --test connector_no_model_address`; expect FAIL (`unknown field ca_settings`).
- [ ] Step 3: Implement. schema.rs: add the field; extend the 315 loop to `("ca_settings", &c.ca_settings)` for the path rule (1 root), and for its `://` rule refuse unless the value starts with `https://`, the key's last part is `base_url` and the part before it starts with `goalcup-`. mod.rs `load()`: for each such value, the host (between `https://` and the next `/` or `:`) must give `Decision::Listed { source: Source::Shared, .. }` from `connector.hosts.decide`, else `ConnectorError::ModelAddress { file, key: format!("ca_settings[{i}].value") }`.
- [ ] Step 4: Run `cargo nextest run -p goalcup --test connector_no_model_address`; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup/src/connector/schema.rs crates/goalcup/src/connector/mod.rs crates/goalcup/tests/connector_no_model_address.rs; git commit -m "connector: ca_settings trust rows, https to a shared-list host only"`

### Task H.4: Write `ca_settings` at every launch   (size S)
**Files:** Modify crates/goalcup/src/connector/mod.rs (after line 301, inside the same `if free`); Test crates/goalcup/tests/connector_launch.rs (append)
**Interfaces:** Consumes `ca_settings` (H.3). `cli/setup.rs` is not touched: setup runs Hermes outside the proxy, where `${env:SSL_CERT_FILE}` is unset.
- [ ] Step 1: Write the failing tests (helper: a connector file in the temp dir, `HEAD` as in connector_no_model_address.rs, plus the two rows of H.3's loading test with `file = "{home}/config.yaml"`):
  ```rust
  #[test]
  fn trust_rows_are_written_at_launch_once() {
      // load the file connector, plan twice on d.join("home")
      let text = fs::read_to_string(home.join("config.yaml")).unwrap();
      assert!(text.contains("  goalcup-trust-x:\n    base_url: \"https://api.deepseek.com/v1\"\n    ssl_ca_cert: \"${env:SSL_CERT_FILE}\"\n"), "{text}");
      // second plan: bytes equal
  }
  #[test]
  fn trust_rows_skip_a_flow_map_and_log() {
      // home/config.yaml holds "providers: {}\n" before the plan
      assert!(plan.log.iter().any(|l| l.starts_with("ca_settings not written: ")), "{:?}", plan.log);
      assert_eq!(fs::read_to_string(home.join("config.yaml")).unwrap(), "providers: {}\n");
      assert!(plan.refusal.is_none());
  }
  ```
- [ ] Step 2: Run `cargo nextest run -p goalcup --test connector_launch trust_rows`; expect FAIL (file absent / no log line).
- [ ] Step 3: Implement: `let skipped = write_settings(home, &connector.ca_settings).into_iter(); log.extend(skipped.map(|s| format!("ca_settings not written: {}", s.reason)));`
- [ ] Step 4: Run `cargo nextest run -p goalcup --test connector_launch`; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup/src/connector/mod.rs crates/goalcup/tests/connector_launch.rs; git commit -m "connector: write ca_settings at every start and continue"`

### Task H.5: Hermes trust rows, no version note   (size S)
**Files:** Modify connectors/hermes-cli.toml (add `ca_settings` after line 40 with a 4-line comment citing ssl_verify.py:300-343 and config_providers.py:446-472; delete 42-44); Test crates/goalcup/tests/connector_hermes_rows.rs
**Interfaces:** Consumes H.3, H.4. One entry per route, named `goalcup-trust-<hermes provider id>`, two rows each (`base_url`, then `ssl_ca_cert = "${env:SSL_CERT_FILE}"`). Routes (plugins/model-providers/*/__init__.py, same at v0.22.5 and v0.22.6; recheck v0.22.7 in Step 3; every host is in connectors/model-hosts.toml):
  deepseek `https://api.deepseek.com/v1`, ai-gateway `https://ai-gateway.vercel.sh/v1`, openrouter `https://openrouter.ai/api/v1`, nous `https://inference-api.nousresearch.com/v1`, xai `https://api.x.ai/v1`, zai `https://api.z.ai/api/paas/v4`, fireworks `https://api.fireworks.ai/inference/v1`, alibaba `https://dashscope.aliyuncs.com/compatible-mode/v1` and `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` (entry ids `goalcup-trust-alibaba`, `goalcup-trust-alibaba-intl`).
- [ ] Step 1: Write the failing tests:
  ```rust
  const ROUTES: [&str; 9] = ["https://api.deepseek.com/v1", "https://ai-gateway.vercel.sh/v1", "https://openrouter.ai/api/v1",
      "https://inference-api.nousresearch.com/v1", "https://api.x.ai/v1", "https://api.z.ai/api/paas/v4",
      "https://api.fireworks.ai/inference/v1", "https://dashscope.aliyuncs.com/compatible-mode/v1",
      "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"];
  #[test]
  fn hermes_trust_rows_pair_each_route_with_the_run_bundle() {
      let c = connector::load("hermes-cli", None).unwrap();
      let urls: Vec<&str> = c.ca_settings.iter().filter(|r| r.key.ends_with(".base_url")).map(|r| r.value.as_str()).collect();
      assert_eq!(urls, ROUTES);
      for r in c.ca_settings.iter().filter(|r| r.key.ends_with(".base_url")) {
          let twin = r.key.replace(".base_url", ".ssl_ca_cert");
          assert!(c.ca_settings.iter().any(|t| t.key == twin && t.value == "${env:SSL_CERT_FILE}"), "{twin}");
          assert_eq!(r.file, "{home}/config.yaml");
      }
      assert_eq!(c.ca_settings.len(), 18);
  }
  #[test]
  fn hermes_has_no_version_note() {
      assert!(!include_str!("../../../connectors/hermes-cli.toml").contains("version_notes"));
  }
  ```
- [ ] Step 2: Run `cargo nextest run -p goalcup --test connector_hermes_rows`; expect FAIL (`ca_settings` empty; `version_notes` present).
- [ ] Step 3: Implement the rows. Before committing, check each route at v0.22.7: `curl -fsS https://raw.githubusercontent.com/NousResearch/hermes-agent/v0.22.7/plugins/model-providers/<id>/__init__.py | grep -o 'base_url="https://[^"]*"'`; a route that differs is dropped from ROUTES and the rows, never guessed.
- [ ] Step 4: Run `cargo nextest run -p goalcup --test connector_hermes_rows --test connector_builtin`; expect PASS.
- [ ] Step 5: Commit `git add connectors/hermes-cli.toml crates/goalcup/tests/connector_hermes_rows.rs; git commit -m "hermes: trust rows give every version the run bundle; drop the 0.22.6 note"`

### Task H.6: Docs say what goalcup writes and why   (size S, PARALLEL-OK after H.5)
**Files:** Modify docs/harnesses/hermes-cli.md:3 (`Facts for version: 0.22.5 to 0.22.7`), :52 (CA paragraph), :13 (one clause); docs/CONNECTOR-SCHEMA.md one row after :40, one clause in :60 (lane L's file, these lines only)
**Interfaces:** none
- [ ] Step 1: Verification: `grep -n "148241\|0.22.5 is unaffected\|drops .SSL_CERT_FILE" docs/harnesses/hermes-cli.md; grep -c "ca_settings" docs/CONNECTOR-SCHEMA.md`
- [ ] Step 2: Before: 1 match in hermes-cli.md; 0 in CONNECTOR-SCHEMA.md.
- [ ] Step 3: Write. hermes-cli.md:52 says: since 0.21.6 model calls trust the OS store plus a `providers` entry's `ssl_ca_cert` matched by exact `base_url`; on macOS `SSL_CERT_FILE` never reaches them, on Linux it does; goalcup writes one `goalcup-trust-<provider>` entry per listed route into its own Hermes home with `ssl_ca_cert: ${env:SSL_CERT_FILE}`, names only the real address, and Hermes still calls the model itself through `HTTPS_PROXY`; the entries show in Hermes's provider list and are not for picking; a custom endpoint or a route not listed is not trusted on macOS and the run says it has no model requests. :13 adds: the trust entries do not count as setup. CONNECTOR-SCHEMA row: `ca_settings` | list of `{file, key, value}` | `[]` | Trust rows written at every start and continue, never by setup; an `https://` value only on a `base_url` key under a `goalcup-*` entry, host on the shared list. :60 adds `ca_settings` to the `://` rule with that exception.
- [ ] Step 4: Run the Step 1 greps; expect 0 matches in hermes-cli.md and >= 2 in CONNECTOR-SCHEMA.md; run `python3 scripts/check-links.py` if lane K has landed it.
- [ ] Step 5: Commit `git add docs/harnesses/hermes-cli.md docs/CONNECTOR-SCHEMA.md; git commit -m "docs: Hermes trust rows, the macOS cause, no version note"`

### Task H.7: Rows per version and OS   (size L, PAD)
**Files:** Create scripts/pad/runs/core-hermes-mac.toml (`core-hermes.toml` plus `where = "here"` and `build = "target/release"`); Modify scripts/pad/harnesses/hermes-cli.toml (add `here_path = "/Users/team/dev/harness-installs/hermes-{version}/.local/bin"` once lane G ships the key; `version` stays `0.22.5`, the sandbox image pin)
**Interfaces:** Consumes H.1 to H.5, lane G `here_path`, G.20 `versions`, G.23 `matrix-hermes.toml` (ask lane G for `versions = ["0.22.5", "0.22.6", "0.22.7"]`).
- [ ] Step 1: Install 0.22.6 the same way as H.2 Step 3 (commit f0d94882e89ab8d4098b5bb2f166adfb9ee0a0ab), so no row depends on the owner's own install.
- [ ] Step 2: Fresh-home check (review focus 5), in an empty folder with `PATH=/Users/team/dev/harness-installs/hermes-0.22.7/.local/bin:$PATH`, `GOALCUP_HOME="$(mktemp -d)"` and `GOALCUP_SERVER=off`: `target/release/goalcup setup --harness hermes-cli --yes`, then `target/release/goalcup start`; expect the "isn't configured yet" screen, then `grep -c goalcup-trust "$GOALCUP_HOME/homes/hermes-cli/config.yaml"` = 9. Quit at the screen.
- [ ] Step 3: Run `scripts/pad/pad scripts/pad/runs/core-hermes.toml` (Linux, 0.22.5) and `matrix-hermes.toml` with `where = "here"` (Mac, three versions); without `here_path`, `core-hermes-mac.toml` for the installed version only. 12 full rows (3 Linux, 9 Mac).
- [ ] Step 4: Pass, per row: `C1, C2, C4, C5, C6` PASS; `C3` PASS, or INCONCLUSIVE only for the masked-upstream rule awaiting the owner (ask 4); `facts.threads >= 1` and `facts.subagents >= 1`; `pad_only == []`; `grep -rEil "certificate.verify.failed" <run>/watch` empty; `grep -c "ca_settings not written" <run>/goalcup.log` = 0. Then `python3 scripts/pad/table`; expect Hermes builds for each passing (version, os).
- [ ] Step 5: Commit `git add scripts/pad/runs/core-hermes-mac.toml scripts/pad/harnesses/hermes-cli.toml; git commit -m "pad: Hermes rows per version on macOS and Linux"`; rows and tiers are committed by lane G's G.14/G.23.

## Unknown newer versions
No Hermes row. A participant on 0.22.8 gets lane F's line (F-version-core.md:97 shape): "Hermes 0.22.8: not verified on 0.22.8; verified with goalcup on 0.22.5, 0.22.6, 0.22.7 (macOS). goalcup records it the same way; goalcup status says if model requests are missing." The trust rows apply to it unchanged. If a later Hermes changes the trust rule, the fix is one `[[versions]]` row overriding `ca_settings`, which is why lane I must allow that key.

## Cross-lane risks
- Lane G G.5/G.24 assume 0.22.5 needs no block and plan a 0.22.6-only `pad_only` plus a lane I `ca_env` overlay. Both premises are wrong (Dropped, first item); `ca_env` cannot work because no variable reaches Hermes model calls on macOS. G.5 keeps its mechanism with no Hermes data; G.24 is dropped.
- schema.rs `struct Connector` gets one field from lane E and one from this lane; mod.rs `load()` is shared with lane A (UnknownId text, other lines) and lane I (overlay merge). Rebase order: H.3 after E's field lands.
- Lane I removes `version_notes` from the schema. H.5 deletes Hermes's use first; lane I must not edit connectors/hermes-cli.toml.
- Lane L owns HARNESS-SETUP.md:100 and TODO.md:73, which still say the 0.22.6 note exists: delete those sentences when H.5 lands.
- Pad time: 12 full rows plus 2 to 4 micro rows, DeepSeek and gateway keys, serial on one Mac.

## Owner questions
1. AGENTS.md:60 says goalcup's launch code never sets a model base URL. The trust entry writes a `base_url` key that names the real provider address, never the proxy, only under a `goalcup-*` entry and only for shared-list hosts; Hermes still sends calls itself through `HTTPS_PROXY`. Without it, Hermes on macOS records no model call on any version since 0.21.6. Allow it (recommended), with lane L adding that exception to AGENTS.md:60?
2. Routes: ship the 9 built-in routes above (recommended; only DeepSeek and Vercel AI Gateway are pad-proven) or only those 2?
3. The pad presets Hermes's telemetry answer (`telemetry.shared_metrics`, hermes-config.yaml:5-10), which a participant answers on screen as `first_screen` tells them. Count it as the participant's answer (recommended), or mark it `pad_only`?
4. Installing three Hermes builds under /Users/team/dev/harness-installs (H.2, H.7) on the owner's Mac: agreed?
