# Fixtures — spike 4, claude-code-cli (launch), run 2026-09-03

Redacted copies of the artifacts from run `run-20260903T073054Z` (standalone CLI
`~/.local/bin/claude` **2.1.259**, macOS Darwin 25.5.0, launch mode with
`CLAUDE_CONFIG_DIR=harnesses/home/claude`). Session
`95addb64-9dac-4046-8ef6-8aa53fa0c7f1`, pinned with `--session-id`.

## Files

| file | sha256 (first 16) | bytes | what it is |
|---|---|---|---|
| `main-session.jsonl` | `832f5d6f97b88d18` | 157,831 | the whole main transcript, 148 records, launch → hard kill → resume → `/exit` |
| `subagent-a8ca2ea8d4a8a1673.jsonl` | `aa54a7993395d07f` | 44,302 | the one real subagent (`general-purpose`), 17 records |
| `subagent-a8ca2ea8d4a8a1673.meta.json` | `4346f85ce5e211d6` | 131 | parent link: `agentType`, `description`, `toolUseId`, `spawnDepth` |
| `hooks.jsonl` | `953811f27dc7d300` | 79,248 | leg 2, all 50 hook posts with headers and receive times, from **both** hook sources |
| `tracker-settings.json` | `0f7aa0d17c0895d7` | 7,264 | the `--settings` file (17 events, `type: http`), **unmodified** — this hash is the config hash in the report |
| `project-settings.json` | `92328c425e7ff838` | 1,039 | the folder-scoped `<ws>/.claude/settings.json` discriminator (2 events, `type: command`), **unmodified** |
| `rawbody-goal-turn.request.json` | `6af9a30728207544` | 40,507 | API request for the goal turn (`f43f8464-…`) |
| `rawbody-goal-turn.response.json` | `3445f86d0d7cbb80` | 1,445 | its response (`req_011Ceg8TuCNL5CMWq1MQVz7j`) |
| `rawbody-killed-turn-orphan.request.json` | `24ee43fd6b8c15f8` | 57,018 | the request in flight when `kill -9` landed (`9a898513-…`): complete JSON, **no response file exists** |
| `rawbody-subagent.request.json` | `6eec2760438c5f9c` | 23,006 | the subagent's first request (`62b92d07-…`): `cc_is_subagent=true`, its own system prompt, 9 tools |
| `a9-other-folder.jsonl` | `c1a58d137f0b29d8` | 33,706 | the A9 parallel `claude -p "say hello"` transcript from `/Users/team/goalcup-spikes/a9-other`, same `CLAUDE_CONFIG_DIR`, no pins — proof it ran and produced **no** hook post and **no** raw body |

Total 447 KB (this file excluded). Both `tracker-settings.json` and `project-settings.json` are byte-identical
to the files the run used; their hashes match `out/.../config-hashes.txt`.

## Redaction

The three `rawbody-*.request.json` files are also trimmed of vendor text: a `system[i].text` over
1,500 characters keeps its first 1,500, a `tools[i].description` over 300 keeps its first 300, and
each cut is followed by `<scrubbed:vendor_prompt:FIELD:NNNNchars:sha256=...>` with the original
length and SHA-256. The `*.redactions.jsonl` offsets describe the original bytes. The table's hashes
are for the trimmed files. The raw capture and the isolated home stay in the git-ignored `out/`.
