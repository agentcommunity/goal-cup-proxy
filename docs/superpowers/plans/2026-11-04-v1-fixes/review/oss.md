# oss lane
- oss-1 repo name: all public links say agentcommunity/goal-cup-proxy but checkout is goal-cup-tracer; NOTICE cites nonexistent spikes/
- oss-2 /Users/team paths in tracked crates test corpora + scripts/pad/runs/*signin.toml
- oss-3 docs/plans (7.7M, 275 files), docs/superpowers, docs/research, docs/archive tracked
- oss-4 no CHANGELOG, no SUPPORT, no deny.toml/cargo-audit; CODEOWNERS single person
- oss-5 staging workers.dev subdomain b-9a5 in docs/script
- oss-6 README/AGENTS event-centric; stranger story for non-participants thin

## Answers
- inbox-oss.md does not exist: no questions.

## Re other lanes
- cr-scripts-4 agree, severity ok minor; release.yml:59 only compares tag to Cargo.toml, never that the commit is on main; adds to oss-7 (tag/version coupling manual).
- cr-scripts-9 agree: gate.sh:55 runs sudo -n dnf install; a stranger running the gate hits a surprise sudo; matches oss-5 (contributor path).
- docs-7 agree and raises oss-5: CONTRIBUTING.md:64 says "no .rs file" yet the gate needs a Rust line; first contributor PR fails.
- usability-5 / docs-3 agree: broken README example and anchors are the first things a stranger sees.
- arch-9 agree (dead link checker 3k lines): fold into oss-3 cleanup.
- cr-upload-server-5 agree and raise to important for OSS: absolute workspace path with OS username uploaded contradicts the privacy promise in point 4/6.
- cr-upload-server-11/docs-6 agree: removed --dev flag still documented.
