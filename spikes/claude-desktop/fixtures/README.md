# Fixtures — spike 1, claude-desktop (attach), run 2026-09-03

Redacted copies of the artifacts from spike run `ws-20260903T041927Z`
(Claude Desktop 1.44121.4, bundled Claude Code 2.1.258, macOS 26.5.2).
Session `6abc89a2-bf72-4e5f-b5c6-be093c9930b4`.
The supporting private historical record is intentionally omitted from this public OSS cut.

Every file went through `spikes/redact.py` (pattern-based) **and** a structural
scrub (`out/analysis/make_fixtures.py`) that removes things the regexes cannot
see. Both logs are listed below. All files were then reviewed by eye.

## Files

| file | sha256 | bytes | what it is |
|---|---|---|---|
| `main-session.jsonl` | `88646aa4…dace1` | 180,795 | the whole main transcript, 131 records |
| `subagent-ac4cb0667aadfa93a.jsonl` | `a1101507…6a50` | 19,015 | the one real subagent, 8 records |
| `subagent-ac4cb0667aadfa93a.meta.json` | `dd8c66d5…8c72` | 131 | subagent parent link (`toolUseId`, `agentType`, `spawnDepth`) |
| `hooks.jsonl` | `efa85c0c…ba0a` | 79,719 | leg 2, all 40 hook posts with headers and receive times |
| `rawbody-goal-turn.request.json` | `e8ace0b2…8bc6` | 94,195 | API request for the goal turn (`40ce5fa3-…`) |
| `rawbody-goal-turn.response.json` | `375894a79286620489d531654a00c069f4fb8ab7ecaa3d8520e98ef88c56b24c` | 1,150 | its response (`req_011Ceg2hf3kynnA8idKukxV1`) |
| `rawbody-resume-keep-going.request.json` | `9844e8cf…7d6e` | 99,767 | API request for the `keep going` turn after the quit/reopen (`82f05d46-…`) |
| `tracker-settings.json` | `53034637a7110057ce326574f897db255149f863739fd984afbe2ea090e2ad8f` | 7,247 | the folder-scoped `.claude/settings.json` the tracker wrote, **unmodified** — this hash is the config hash quoted in the report |

Total 483 KB (this file excluded).

## Redaction logs

`spikes/redact.py` writes one log per file; empty logs mean the patterns found
nothing.

- `main-session.jsonl.redactions.jsonl` — 4 (email)
- `subagent-ac4cb0667aadfa93a.jsonl.redactions.jsonl` — 1 (email)
- `rawbody-goal-turn.request.json.redactions.jsonl` — 3 (email)
- `rawbody-resume-keep-going.request.json.redactions.jsonl` — 4 (email)
- `hooks.jsonl.redactions.jsonl`, `tracker-settings.json.redactions.jsonl`,
  `subagent-ac4cb0667aadfa93a.meta.json.redactions.jsonl`,
  `rawbody-goal-turn.response.json.redactions.jsonl` — empty

## Structural scrub (what the patterns could not catch)

Recorded in `out/analysis/fixture-scrub-log.json` (git-ignored with the rest of
`out/`); 28 edits in total:

| file | field | removed |
|---|---|---|
| `main-session.jsonl` | `attachment.skill_listing.content` | 29,871 chars listing the operator's 82 skills, including private project skills (founder-voice, ICANN, a production VPS with its path) |
| `main-session.jsonl` | `attachment.skill_listing.names` | the 82 skill names, same reason |
| `main-session.jsonl` | `bridgeSessionId`, `ownerAccountUuid`, `ownerOrganizationUuid` on 6 `bridge-session` records | account and organization identifiers |
| `subagent-…jsonl` | same two `skill_listing` fields | 29,900 chars |
| both raw request bodies | `messages[1].content` | 53,605 chars: the startup system message, which concatenates the `SessionStart` hook's additional context, the deferred-tool list, the MCP server instructions **and** the same private skill listing. The placeholder states the original length, because the length and position of this block are the evidence, not its text. |
| both raw request bodies | `metadata.user_id` | `device_id` and `account_uuid`; `session_id` is kept, it is the evidence |
| both raw request bodies | `system[4]` | the one `Git user: <handle>` line inside the 21,163-char session-context system block; the rest of the block is stock Claude Code prompt text and is kept |

Absolute home paths (`/Users/team/…`) are kept deliberately: the project-slug
rule and the store layout are the thing under test.

## Not committed

`spikes/claude-desktop/out/` is git-ignored and stays local. In particular
`out/ws-20260903T041927Z/leg1/capture.jsonl` (18 MB) is the hash-chained leg-1
capture; it contains raw API bodies belonging to three other Claude Code
sessions that were running on the machine during the run (see the report, A9 and
"Surprises"). It is not edited and not committed.

## Vendor-prompt trim (2026-10-04, before the public release)

The raw request bodies carried Claude Code's system prompt and tool descriptions verbatim,
which is vendor text, not ours. Following the Cursor precedent (`spikes/cursor-cli/fixtures/
README.md`, `system_prompt_tail_with_host_skill`), these were trimmed in place and the JSON
structure was kept:

- every `system[i].text` longer than 1,500 chars keeps its first 1,500 chars;
- every `tools[i].description` longer than 300 chars keeps its first 300 chars;
- each cut is followed by `<scrubbed:vendor_prompt:FIELD:NNNNchars:sha256=...>`, carrying the
  original length and the full SHA-256 of the original string;
- tool names, `input_schema` objects, `messages`, `metadata`, `betas` and every other field are
  untouched, so the tool count, the system-block count and their order stay verifiable.

The `*.redactions.jsonl` offsets still describe the original, pre-redaction, pre-trim bytes and
were not rewritten. The sha256 values in the table above are for the trimmed files. The trim
was applied to both `rawbody-*.request.json` files.
