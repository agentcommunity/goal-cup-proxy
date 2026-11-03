# Fix list for PRD-08-verifier-checks.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 2 major, 2 minor, 4 in all.

## G2-11 (major)
- Sources: X3-14, P03-3
- Section and lines here: R6
- Same id also in: PRD-03-proxy
- Ledger item or rule: PRD-16 acceptance plan; T6 (10 USD per run); acceptance a watcher cannot run
- Change:
  PRD-03 acceptance 10: "**Load** (phase 6, PRD-16 A1 and A2, as PRD-16 runs them under the 10 USD cap). Pass on each: zero proxy GAPs of any kind on listed hosts, `memory_guard` included (PRD-16 X3); no `503` or refused connection; PRD-16 X1 holds per 10-minute bucket; recorder RSS reported. Connections cut by A2's planned kill are PRD-02's `recorder_down`, not a proxy GAP." Lines 586 and 645: "The ALPN default stays `PreferH1` in 1.0; no mirrored load run is scheduled (G2-25)." Make the risks row at 627 say the same.
  PRD-08 R6: point to the same runs (PRD-16 A1 and A2), not "A2 Codex".

## P03-4 (major)
- Sources: P03-4
- Section and lines here: `listed_hosts_decrypted`
- Same id also in: PRD-03-proxy, PRD-01-record-format
- Ledger item or rule: Ledger R4 (named loss), point 6
- Change:
  PRD-03 section 2, after "Guard 64 KiB (`client_hello`); past it, a tunnel" add: "The CONNECT host decides. The `conn` gets the new tunnel reason `hello_too_big`; on a listed host its close GAP is `memory_guard` with `{"guard":"client_hello","limit":65536}`, `off` 0, `lost` = `up` + `down` + 1."
  PRD-01: add `hello_too_big` to the tunnel reasons and GAP 1 to the listed-tunnel GAP list. PRD-08: count it as `named`.

## P03-22 (minor)
- Sources: P03-22
- Section and lines here: `refusal_after_trust`
- Same id also in: PRD-03-proxy
- Ledger item or rule: Provable acceptance; point 6
- Change:
  PRD-03: "one `ca_refused` note each time a pair is learned"; acceptance 7's third call adds "and a second `ca_refused` note". The no-alert close gets GAP tail `{"alert": false}`. PRD-08: skip `refusal_after_trust` for a GAP with `alert: false`.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
