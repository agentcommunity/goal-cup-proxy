# Fix list: PRD-01-record-format (g3)

Cross-file fixes raised by the g3 reviews. Ids starting G3-X are shared with other fix lists. Format: id |
severity | section and lines | change | ledger item or rule.

- **G3-X20** | major | Section 9 COUNT example (375) and key list (384); Section 11 note `what` list (465) | Apply PRD-00 R3-4: each `tunnel_hosts` value gains `listed` (the GAP reason name) for a listed host, shown in the example; notes `outlived` (`count`, `programs`), `native_missing` and `parse_stopped`, and the `long_tunnel` facts, join the `what` list. PRD-10 prints only what these keys hold. | R6, R8 (locked); PRD-00 R3-4 (P10-04)
- **G3-X33** | minor | Interfaces row for PRD-17 lessons (786) | "Lesson cases H10 and K15 of PRD-17 write a part file through the real pipeline and read it with `read`; their test also runs `goalcup-verify check` and asserts exit 0. No other lessons test writes a part file." | Interface agreement with PRD-08, PRD-17 (17-11)
