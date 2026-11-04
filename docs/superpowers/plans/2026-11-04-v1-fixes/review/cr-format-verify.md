# cr-format-verify notes (crates/goalcup-format + goalcup-verify, docs/RECORD-FORMAT.md)
Verdict: With fixes. Codec/reader/writer/recovery are solid and well tested; the verifier is internally consistent but its honesty checks trust the record's own labels.

## Findings, top first
1. important: honesty "listed host" judgement never consults connectors/model-hosts.toml; it trusts the conn meta `listed` and the start event `hosts` (honesty.rs:164-199, 247-262). A tunnel to api.openai.com labelled not_listed passes. Fix: include_str model-hosts.toml (as shapes.tsv already is) and treat provider/gateway hosts as listed whatever the record says; check `sign_in` against the `never` list (honesty.rs tunnel_end skips any sign_in tunnel, no byte bound).
2. important: `check` without --known runs NO secret scan and the report has no `secrets` entry (not even skip) (report.rs:40, main.rs:127). Shapes are built in (secrets::Rules::shapes). Seven point 4 unverified by default. Fix: always add `secrets` (shapes only when no --known).
3. important: `warn` leaves exit 0 and text output hides warns (report.rs:49-53, 136). conn_bytes warn (10-50% of plain bytes unaccounted), refusal-after-trust warn, history warn = "pass". Fix: loss-indicating warns -> named (exit 1) and printed.
4. important: timeline DoS by forged wall times: LINK wall_ns = u64::MAX widens `last`, an anchored recorder_down gap spans first..last, loop makes ~2.7e7 BTreeMap rows (timeline.rs:123-149) then JSON. Fix: cap buckets (e.g. 1e5) or reject LINK wall far from PART+mono delta (also add wall/mono consistency check).
5. minor/important: secrets SSE parse drains the line buffer per line: O(N*L) on one big DATA, and `Stream.line` unbounded (secrets.rs:359-372). Fix: scan with position cursor, cap line.
6. important (versions): usage readers keyed by (connector, row) only, harness_version known (usage/mod.rs:368) but unused (usage/hints.rs:30-51). Add version to reader() key + report `reader_version`.
7. minor: EVENT/GAP/OPEN/END JSON that fails to parse becomes {} silently (integrity.rs:408,546,657; honesty.rs:104) - no finding. Add record_json check.
8. minor: wire_keeps_up is cumulative: early wire-only surplus (title/count_tokens calls) masks later lost calls (legs.rs:Sweep::new). 
9. minor: secrets.rs:402 `off + len` unchecked (L-VR3 says saturating).
10. minor: hermes DB copy opened READ_WRITE from untrusted bytes without trusted_schema=OFF/defensive, temp dir predictable (usage/sqlite.rs:288, 270-278).
11. minor: PartWriter files/dir created with umask mode, relies on caller's 0700 parent (part.rs:122, 289-294).
12. minor: tests: no mutation for PART continuity (prev/raw_offset/part/entry), sealed+open frame, no fuzz of verifier on random bytes, no forged-wall test.
13. minor: recover() deletes a part file whose .idx is missing (part.rs:343-349); quarantine instead.

## Re other lanes
- versions V10: agree, evidence usage/hints.rs:30-51 reader keyed by connector id only; version available at usage/mod.rs:368.
- arch-3: agree; secrets.rs:22-32 copies the masker regex and GUARD.
- cr-scripts / cr-proxy (C3 known hits in SSE deltas): verifier flags them `fail` (secrets.rs deltas); pad relaxes it. history.rs comment says the recorder does not mask across streamed deltas: point 4 gap in the masker (cr-proxy to confirm).
- cr-watchdog: write errors are fatal (writer.rs:526 exit 74), so PartWriter chain divergence after a failed write is moot.

## Answers
Inbox inbox-cr-format-verify.md did not exist at finish: no questions to answer.

## Round 2 responses
- versions-9: agree (hints.rs:30-51 matches (connector,row,ext) only; counters repeats_skipped/copied_skipped exist but no count of unparsed lines of unknown shape).
- arch-2: agree; the verifier hardcodes connector ids in hints.rs reader() and WIRE_IDS.
- arch-8: agree; secrets.rs:22 include_str!("../../goalcup/src/mask/shapes.tsv") reaches into the recorder tree, ESCAPE/GUARD are copies (secrets.rs:25-33).
- cr-upload-server-8: severity-down. Chunk key embeds sha256 (chunks.ts:140) and head is the chain hash over the content, so equal offset+length+head is not a weak match.
- Format has single VERSION=1 and reader hard-rejects others (codec.rs:264): relevant to versions plan, record format itself needs no per-harness version.
