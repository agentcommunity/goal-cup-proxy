# Fix list: PRD-02-recorder-core (g3)

Cross-file fixes raised by the g3 reviews. Ids starting G3-X are shared with other fix lists. Format: id |
severity | section and lines | change | ledger item or rule.

- **G3-X18** | major | Owner decisions (36) | Replace "Disk full: no special handling. The harness stops and the participant cleans up." with "Disk full: no special handling. Recording goes off (null mode), goalcup never stops the harness, and the gap covers the time (section 5)." | R2, R3 (locked); point 3 (15-8)
- **G3-X20** | major | Section 6 `status.json` (around 300-311) | Apply PRD-00 R3-2: `written` carries every COUNT key as entry totals (including `req_holes`, `tunnel_hosts`, `mask_failclosed_bytes` in the example); notes keep the first note of each `what` beside the last 20 (line 307 "last 20" becomes "the first note of each `what` and the last 20"); the sealed `status.json` carries `seal {reason, workspace_head}`, `cut_files` and `outlived`. | R6 (locked); PRD-00 R3-2 (P10-04)
- **G3-X32** | major | Interfaces (PRD-17 and PRD-14 rows); test constructors | Add a test constructor for the real pipeline (masker and writer) in a temp entry folder, used by PRD-14's `common::recorder::real(dir)`; PRD-17 H10 and K15 run through it because `for_tests` shows bytes before masking. | M1, point 4 (17-4)
- **G3-X56** | minor | R1 kill test (693) | Add: "The first kill comes after the first heartbeat row." | Provable acceptance for PRD-07 acceptance 2 (P07-7)
