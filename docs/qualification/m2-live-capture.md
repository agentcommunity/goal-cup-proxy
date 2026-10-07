# T24 installed capture — 2026-09-30

> **Historical (2026-10-02).** Both T24 rows on `d8d5b1cf` failed and are preserved as measured. Current: [3c rows](m2-real-agent-progress.md#current-paired-default-tiny-real-qualification--3c-pass).

**Both real tasks failed. T24/T24N qualification is incomplete.** Local packaging, native capture and integrity-valid seals are measured separately; neither task produced the requested file/tool completion or model-wire observations. No pushed or published release.

## Candidate and installed bytes

Source `d8d5b1cf3936426ffa8be787f50a99044901c14f`; fresh default build `CARGO_BUILD_JOBS=3 cargo build --release --offline -p goalcup-cli -p goalcup-daemon -p goalcup-verifier` exited 0. Three arm64 binaries report version 0.1.0; test-support marker absent. The private `m2-pilot` bundle is target `aarch64-apple-darwin`, version `0.0.0-m2-pilot-candidate`, unsigned, unnotarized and unqualified. Installation exited 0 and seven staged/installed hashes matched:

| Payload | SHA-256 |
|---|---|
| `bin/goalcup` | `79dd7ebe17fecf5ab6abc15894da060b2eb2e7e34da916259f9e160396211945` |
| `bin/goalcup-daemon` | `01cee03588c4157c3ea32cb0492abdb3f6f70845038688ec6072031918d080d8` |
| `bin/goalcup-verify` | `d78b54b865d36a3d3505f6cc08140af1e39ca2beaac9c096576c123dd3a8e871` |
| `connectors/claude-code-cli/files/settings-launch.json` | `34fd01e7577c7260b7cb8ddce08a626429e207494d3058fcae1f24c3b7972d9d` |
| `connectors/claude-code-cli/manifest.toml` | `6cec1071e12e234113a48fcc778d529fd75e46da23b3009a3c2b147b5086e299` |
| `connectors/codex-cli/files/config.toml` | `776f5a90326020276259241d576bebc89ad6366f57d96b4f1ecfd7b354367ba9` |
| `connectors/codex-cli/manifest.toml` | `90213a073d161bc7cd6e0d5df5d890f30c771cbac568878431a569dd4a7ea9ba` |

Bundle/prefix: `/Users/Shared/goalcup-m2-t24/candidate-d8d5b1cf3936426ffa8be787f50a99044901c14f/{bundle,prefix}`; label `com.agentcommunity.goalcup.m2pilot.t24.d8d5b1`. Prefix was scoped-uninstalled after both final reviews; bundle and evidence remain. Actual aliases and local records bind the canonical existing isolated homes. Source review and focused tests support request/launch binding consistency; a live Codex child-environment snapshot was not retained. Tester read-only observation confirmed Claude actual HOME, canonical CLAUDE_CONFIG_DIR and keychain link. Both scoped subscription auth probes succeed; no credential copy/new login/global config/base URL/global CA/profile change.

## Gate and real rows

Gate accepted with documented retry: original build/fmt/clippy/workspace exits **0/0/0/101**; **134 result rows, 2,406 passed / 2 failed / 21 ignored**. Known finish_matrix Busy(mid-upload) cases were signed_deadline_finishes_without_cli and manual_finish_after_restart_resume_is_idempotent. One permitted isolated serial retry passed **3/3, exit 0**; original failure retained, no full rerun, original start-load effect unmeasured. Inherited proc-macro-error2 future-Rust warning remains. Historical gate/build `9529e388` and corrected synthetic smoke `0d421533` are distinct.

| Row | Task | Captured research observations | Integrity/local finish |
| --- | --- | --- | --- |
| [Corrected Codex](m2-installed-codex.md), e4cd17147d161ce46 | FAILED, TLS EOF/reconnects; no tool/result | 8 native, 0 OpenAI Responses | verifier 0, valid/sealed/zero gaps; record 12, LocalOnly |
| [Claude](m2-installed-claude.md), e500e225fb8747257 | FAILED, ECONNRESET/zero usage; no tool/result | 30 native, 0 Anthropic Messages/0 OpenAI Responses | verifier 0, valid/sealed/zero gaps; record 22, LocalOnly |

Goal: 100-byte file, SHA-256 `71e93a0247b0ff22a8048cd580b6316404e8910d62753260fc96588263f94d84`, requesting result.txt exactly m2-capture-ok and a real tool read. Both used identical installed bytes, reservation 67,108,864 bytes and --no-open. Observer saw 131,072 filesystem blocks (64 MiB) on device 16777234 while starting; its timestamp alone does not prove pre-spawn allocation. Reviewed reservation sequence 4 precedes Launch sequence 5. No receipt clock, verified acknowledgement or upload exists; delivery/anchor are local_only.

## Retained failures and remaining scope

Original Codex e2b55d691d6d8c896 used an empty installer-created home after alias setup exited; separate green auth probes had not proven actual binding. Incorrect READY provenance and failed row retained. TERM at 14:05:29Z was about 33 seconds beyond its 600-second horizon. Original bootstrap-code-5 installed failure and corrected synthetic smoke with deliberate verifier exit 1 remain separate. Config test-call failure, native restart count failure, detached-tee finding/bounded 10-second drain residual and superseded package findings remain in historical reports.

Transport stage, source defect and remediation are unproven. Narrow follow-up is non-secret admission classes for upstream/client TLS, ALPN and timeout; no additional implementation/probe/test this round. G-2 remains incomplete: full 40-minute Part B awaits separate human quiet-window GO, not waived/completed. Recovery/counter exit-1 coverage, scanner expansion, finer frontier/RSS and volume-suite scope review remain backlog; T27/T28 sustained qualification deferred. Raw rows remain owner-only/ignored. Final source report is live `t24/reports/T24-live-results.md`; two small aggregate fixtures were passed through spikes/redact.py (zero substitutions), then read by eye. They contain measurement identifiers/counts/hashes/outcome only, no native/auth bytes or redaction marker. Both final scoped reviewers approved the failed-result prose and aggregate fixtures (no C/I); this is not approval of successful qualification.

## Teardown

Scoped teardown completed after both final evidence/fixture reviews approved the failed-result report (not successful qualification): exact owned label bootout exit 0, existing assisted uninstall exit 0, subsequent owned-label lookup exit 113 (absent). Installed bin/share, daemon config and install manifest removed. Artifact/workspace/row trees, preserved failed-home bytes and home aliases remain; authenticated target paths/metadata unchanged, credentials not read and uninstall did not follow aliases. Of 107 scoped retained records compared, 106 matched; registry/registry.redb digest changed during shutdown, cause uninspected. Do not claim all artifact bytes identical. Logs/snapshots remain in candidate/evidence/08-owned-bootout.log, 09-owned-uninstall.log, 10-owned-label-absence.log and teardown-preservation-{before,after}.json. Bundle, raw rows and earlier failures retained. No worktree/branch cleanup or further test/model/probe.
