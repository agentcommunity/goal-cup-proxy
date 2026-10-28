# One capture system for every harness (spec)

28 Oct 2026. Owner decisions taken in the Vercel Gateway session ([results](../research/results/2026-10-vercel-gateway.md)).
The implementation plan is [2026-10-28-one-capture-system.md](2026-10-28-one-capture-system.md).

## Why

Goal Cup Proxy picks the hosts it opens in three ways: `providers.toml` rows, `[[capture.hosts]]` rows
and `model_host_from` (D62, Hermes only). So every gateway became a special case: Claude Code and
Codex pointed at Vercel AI Gateway send their model calls through an opaque tunnel (measured 28 Oct:
Claude Code, 0 wire requests). The proxy also throws away the names of the hosts it tunnels, although
RULES promises "the address and the bytes each way". One rule set fixes both, with no vendor list.

## The four rules (README wording)

1. **The chat file.** If the harness keeps its own conversation file, Goal Cup records it. It is a
   backup of the wire record; a harness may have none.
2. **Model traffic.** Goal Cup opens only the model service: the connector's default hosts, or the
   one address the harness's own settings name. Every request under that address is recorded, and
   the standard formats (`/chat/completions`, `/messages`, `/responses`) are decoded.
3. **Everything else is sealed.** Any other connection passes through closed, noted only as host,
   port, bytes each way, times and reason. Sign-in and token refresh are always sealed.
4. **The label.** The verifier labels what each run holds. RULES says what each label is worth.

One sentence for README and SECURITY: the harness's access token passes through the recording
proxy's memory on its way to the model service; it is forwarded unread and never written
(`crates/goalcup-cli/src/proxy/http.rs`, "every other header is skipped unread").

## Requirements

- **R1, chat file optional.** A connector may declare zero `[[capture.native]]` rows (schema 1, a
  widened bound). A run with no native row records the wire only and resumes as today.
- **R2, model host from config, with defaults.** `capture.model_host_from` gains:
  - `via` (optional): a dotted key in the same file whose string value names one table; the `key`
    then contains exactly one `*` part, replaced by that value. One indirection, never two.
  - `absent` (optional, closed set `"refuse"` default, `"defaults"`): with `"defaults"`, a missing
    file, key or `via` value is not an error; the run opens the connector's host rows only.
  - The deny word `env` is removed from the file and key check. All other deny words stay. The D62
    refusals (http, loopback, IP literal, single label, Goal Cup host, sign-in host) stay and apply
    whenever a value is present.
  - With `absent = "defaults"` the connector must list at least one host row.
- **R3, sealed connections listed and uploaded.** Each completed or cut tunnel is noted as host, port,
  reason, start and end time, bytes each way. The list is written at finish to
  `tunnels.jsonl` in the run directory (format tag `goalcup-tunnels-v1`, at most 10,000 rows plus a
  dropped count, mode 0600), summarised on one finish line, and sent with the seal POST as an optional
  `tunnels_jsonl` field. The server stores it create-only in R2 beside the seal. It is not part of the
  v5 chain and the v5 verifier never reads it (CONTRACTS "Side artifacts").
- **R4, evidence label.** The research summary gains `analysis.evidence` (`wire_and_native`,
  `wire_only`, `native_only`, `wire_undecoded`, `none`) and the wall-clock range of decoded wire
  model requests. `RESEARCH_SCHEMA_VERSION` becomes 5. `cross_leg` is unchanged.
- **Connectors.** Claude Code reads `env.ANTHROPIC_BASE_URL` in `settings.json` (`absent =
  "defaults"`, keeps `providers = ["anthropic"]`). Codex reads `model_providers.*.base_url` via
  `model_provider` in `config.toml` (`absent = "defaults"`, keeps `chatgpt.com`). Hermes lists its
  OAuth refresh hosts in `auth_hosts`. Amp is unchanged.
- **Label worth (RULES).** `wire_and_native`, `wire_only`, `wire_undecoded` and `native_only` count;
  `none` does not. OpenClaw `--local` (chat in one process) is allowed.
- **Frozen, not deleted.** `providers.toml` keeps `anthropic` and `deepseek` (CONNECTOR-SCHEMA promise
  6); no row is added. No `vercel` row. No per-path hint in `HostSpec`.

## Changed while building (see [ledger](2026-10-28-one-capture-system-ledger.md))

- The local list is `tunnels-<entry_id>.jsonl`, one per entry, not `tunnels.jsonl` (L26).
- The server keeps it once per entry, create-only, as `traces/<event>/<run>/<entry>/tunnels.jsonl`,
  and the list is capped at 8 MiB (L25).
- Hermes keeps its `auth_hosts` as they were (L12).
- A gateway the config names opens beside the connector's default hosts: the default hosts *and*
  that address, not one or the other (L11).

## Out of scope

Codex `api.openai.com` row (needs a measured API-key run); a decoder for OpenCode's
`/v3/ai/language-model`; tunnel notes inside the chain (v6); OpenCode and OpenClaw connectors (the
other session builds them on this shape); refusing harness proxy-override settings.
