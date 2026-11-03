# Fix list: PRD-06-uploader-compression (g3)

Cross-file fixes raised by the g3 reviews. Ids starting G3-X are shared with other fix lists. Format: id |
severity | section and lines | change | ledger item or rule.

- **G3-X22** | major | Section 6 (258, 262-264); Builder notes (642); Interfaces row chunk answer (476) | A bad `counts` is dropped by the Worker with a `counts-dropped` log line and the heartbeat is kept (`200`), never `400` (PRD-07 section 4). Replace "PRD-07 refuses anything else with `400`" and "or every heartbeat is a `400`" accordingly. The chunk answer is `{offset, next_seq, head, received_at, sealed}`. Section 6 names the `counts` keys; `docs/UPLOAD-API.md` copies them (PRD-07 fixes no key list). | Point 3, D64, seam S5 (P07-2)
- **G3-X23** | major | Acceptance automated tests | Add: "`failed` end to end: one automated test in a build box runs the `test-hooks` binary with forced null mode (PRD-02 `watchdog_null.rs` setup) against `wrangler dev` (`DB_BACKEND=local`) and asserts a stored heartbeat with `recording` `failed` and `vitals.counts.mode` `"null"`, then `gaps` after the next good boot." | R7 (locked), B3 (P07-3)
- **G3-X26** | major | U6 pass line | U6 reads the heartbeat rows from `fetch.json` `heartbeats` (PRD-07 section 7), not its own database query. | Points 5, 6; R7 (P07-5)
- **G3-X29** | major | Acceptance automated tests; Interfaces PRD-14 row (482) | Add: "`upload_worker.rs`, `#[ignore]` (full tier): the real client against `wrangler dev` with its local database, then `scripts/fetch-run.sh --local`; the fetched `rec.gcr.zst` equals the local parts byte for byte." List it in the PRD-14 interface row. | Point 5, S1 (locked) (T14-7)
