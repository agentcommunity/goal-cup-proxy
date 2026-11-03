# Fix list for PRD-11-connectors-harnesses.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 2 major, 4 minor, 6 in all.

## G2-06 (major)
- Sources: X3-6, P00-11
- Section and lines here: Builder notes step 1 (867-868)
- Same id also in: PRD-00-index, PRD-02-recorder-core, PRD-05-masking
- Ledger item or rule: PRD-00 merge rule 1 and file ownership
- Change:
  PRD-00 phase 0 step 5: "PRD-02: the scaffold: `Cargo.toml` with every dependency the PRDs named, `lib.rs` with the full module list, S1 (with `for_tests` and `reserve`), S4 and S8 as compiling stubs, and an empty placeholder `mod.rs` in every other module folder of `crates/goalcup/src/`. This is the one ownership exception; each owner replaces its own placeholder in its first merge." Add: "PRD-05: the S2 stub in `src/mask/` by H+1 (it stars every byte, G2-01). PRD-11: the S3 stub in `src/connector/` by H+1, before the host list."
  PRD-02 builder note 1: add "placeholder module roots for every other folder (PRD-00 phase 0 step 5)".
  PRD-05 note 1 and PRD-11 note 1: "replace the scaffold's placeholder with the real S2 (S3) stub by H+1".

## G2-08 (major)
- Sources: X3-8, P02-23
- Section and lines here: S3 types (315-346)
- Same id also in: PRD-00-index, PRD-02-recorder-core
- Ledger item or rule: Seam named but not defined; point 6 (notes reach the record)
- Change:
  PRD-00 Seams, add row "S17 `StartFacts` and the first control line: run, entry, deadline, server_time, local wall, workspace, connector ref, keep, build, `HostList`, `NativeSources`, mask rules, values, CA PEM, previous end, harness pid and start, `null_since`, `LaunchPlan.notes`; every type serializes with serde. Owner PRD-02; used by PRD-10, PRD-11, PRD-05; stable by H+1.5".
  PRD-11 S3: "`HostList`, `NativeSources`, `MaskRule` derive `Serialize` and `Deserialize`."
  PRD-02 control-line `start` row: list the S17 fields and add "`notes` from `LaunchPlan` (written with `Recorder::event` after `start` on boot 0 only)"; PRD-11 interfaces row: add `notes`.

## P04-15 (minor)
- Sources: P04-15
- Section and lines here: Connector test
- Same id also in: PRD-04-native-capture
- Ledger item or rule: Point 4; ledger L6 (no deny lists in code)
- Change:
  PRD-04 asks of PRD-11: "the connector test refuses a native row whose pattern matches any of the connector's own `secret_files`". No check in `src/native/`. PRD-11 adds that connector test.

## P04-16 (minor)
- Sources: P04-16
- Section and lines here: Connector load checks
- Same id also in: PRD-04-native-capture
- Ledger item or rule: Ledger H8 (the file path is the ID)
- Change:
  PRD-04 line 124: "Row names are unique within a connector (refused at load, PRD-11). Rows may overlap; a file is read once, by the first row (in file order) that matches it, compared by the file's absolute path under the resolved roots." PRD-11: refuse duplicate row names at load.

## X2-09 (minor)
- Sources: X2-09
- Section and lines here: 'Files it owns' `docs/harnesses/<id>.md` (715-719) and README table (720)
- Same id also in: PRD-15-docs-decisions
- Ledger item or rule: Ledger L2; PRD-15 rule 2 (one place per fact); point 6
- Change:
  One harness table: `docs/harnesses/README.md`, tier columns generated from `connectors/tiers.toml`. PRD-15 line 97: "the harness table with tiers" becomes "a link to `docs/harnesses/README.md`"; delete the table text at 390-391. PRD-11 line 715: delete "plus the pad recipe (appendix O section .7 of each harness)"; fact pages hold facts and limits only; the pad recipe lives only in `scripts/pad/harnesses/<id>.toml`.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
