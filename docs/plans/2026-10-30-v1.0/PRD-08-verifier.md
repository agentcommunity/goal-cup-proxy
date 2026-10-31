
### 2026-10-31, C3 placeholder and glued-word fix: size decision

- `secrets.rs` is 551 lines (was 528) and `tests/secrets_scan.rs` 270 (was 214). The extra lines are the `placeholder` rule (a mask, `[REDACTED]`, `[HIDDEN]` or `<redacted>` is no secret) and the masker's 24-byte guard. A glued shape hit stays `warn`. Anything past 551 in `secrets.rs` needs a new decision line. A PEM placeholder split across SSE deltas passes on the joined deltas; its raw-stream hit stays a `fail` on purpose. Asked of PRD-05: export `glued` so the `ESCAPE` copy can go.
