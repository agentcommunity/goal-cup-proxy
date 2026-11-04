# docs lane
- docs-1 CONNECTOR-SCHEMA.md:7 "Nothing is pinned" / harnesses/README.md:77 "not pinned" vs connectors/opencode*.toml `version` hard refusal (text.rs:180, guide Appendix A)
- docs-2 RULES.md:208 "samples the version, records a change as an event": no sampler in code; RULES.md:345 machine facts (memory, disk, WSL2, container, power) vs start.rs:271 sends os+cpus only
- docs-3 3 broken anchors harnesses/{codex,claude-code,amp}-cli.md -> HARNESS-SETUP.md#codex etc; TODO.md:51 says HARNESS-SETUP describes removed MCP: it does not
- docs-4 VISION.md:20 two videos (one by hour 24) vs RULES/README/guide one demo video
- docs-5 DECISIONS.md 1416 lines, ~45 superseded entries kept, contradicts its own header policy; RULES.md:294 cites D74 for label (D74 is model host rule)
- docs-6 UPLOAD-API.md:37,174-177 + Worker dev field: --dev removed (D84) but docs/worker still say "Start without --dev"
- docs-7 CONTRIBUTING/skill: contributor PR must be data-only but connector_builtin test fails without builtin.rs entry! line
- docs-8 stale history docs LOCAL-RESEARCH-HANDOFF.md, RESEARCH-TARGET.md (GOALCUP_CONFIG_DIR etc, not in code) in docs/ top level; OPERATING.md:184 usage R2 job "not in this build"
- docs-9 RULES platforms (Intel mac, arm64 Linux) vs README table; README check flags omit --harness-files
## Re other lanes
- versions: agree, doc side is docs-1.

## Answers
- inbox-docs.md: empty/absent, no questions to answer.

## Re other lanes (round 2)
- arch-7 / cr-watchdog-4 / usability-12: agree, severity-up on doc side. connectors/tiers.toml is the seed (every tier "not verified", reason "not yet run on the pad", git log: only the phase-0 seed commit). PARTICIPANT-GUIDE.md:93 says the tier column "is written from the test pad's results" and ARCHITECTURE.md:132 says tiers are set by the pad, yet the shipped binary includes the seed (start.rs:26). The guide should say plainly that 1.0 ships everything "not verified" and that this changes nothing about recording.
- arch-1: agree the doc side conflicts: ARCHITECTURE.md:95 says "The harness keeps its connections" (respawn); null-mode retry (watchdog/mod.rs:793) closes them. Docs should separate respawn from null retry.
- versions-8 = docs-2 (duplicate, same RULES.md:208 claim). versions-3/4/11: agree; doc fix needed too: CONNECTOR-SCHEMA.md:6 points to tiers.toml but no doc says tiers.toml holds one version only (pad table:31).
- oss-1: agree and extend: every public link (README:24 install curl, :220, CONTRIBUTING:11,17) uses agentcommunity/goal-cup-proxy and git remote is goal-cup-proxy, while the checkout/repo identity elsewhere is goal-cup-tracer; NOTICE:1 title "Goal Cup Proxy" and NOTICE:9 cites spikes/*/fixtures which do not exist. The install one-liner in README:24 is the most user-visible. Pick one name; sweep README, CONTRIBUTING, NOTICE, SECURITY, packaging/install.sh.
- oss-3 / docs-8: agree; docs/plans plus top-level history docs (LOCAL-RESEARCH-HANDOFF, RESEARCH-TARGET) are one cleanup: move to an archive branch or docs/archive with a README line.
- cr-upload-server-11 / docs-6: same area as mine; docs-6 adds the Worker side (--dev still mentioned in UPLOAD-API.md:37,174-177). Agree with both, merge into one fix.
- cr-format-verify-2 / -3 (default check runs no secret scan, warns hidden at exit 0): agree and add docs gap: README check flags omit --harness-files (docs-9) and no doc states that a plain `check` is not the full verification; participant text should not imply a pass means secrets were scanned.
- usability-5: checked: --prompt exists (main.rs:63) and is documented consistently in README:44,80 and guide:218,238; I cannot confirm the failure from docs alone, defer to usability's code evidence; if it cannot work the README example and guide:218 both need the same fix.
- cr-scripts-3 / versions-5: agree; docs lack any statement of how pad-config versions map to participant-recordable versions (harnesses/README.md:77 "not pinned").
- cr-proxy-1/-2 (corporate CA, upstream HTTPS_PROXY): docs gap, no troubleshooting section names corporate TLS inspection or proxies; add to PARTICIPANT-GUIDE troubleshooting once behaviour is decided. cr-proxy note on crush macOS: docs/harnesses/crush-cli.md:44 covers Linux only; add a macOS "not recordable" line.

## New findings
- docs-x1 [important] Plan input for multi-version: no single doc describes how a harness version is chosen/recorded/verified. Version facts live in connector comments, tiers.toml, harnesses/*.md, pad config, guide Appendix A (see versions-11). Fix: one "Harness versions" section in CONNECTOR-SCHEMA.md as the contract (field list, meaning of version, range policy, what a mismatch does), and have harnesses/*.md headers be generated or link to it.
- docs-x2 [minor] Public name mismatch is also a docs-correctness bug for the install path (README:24) not only branding; see oss-1 above. Verify the URL resolves before 1.0 announcement.
