# Fixtures — spike 1, claude-desktop (attach), run 2026-09-03

Redacted copies of the artifacts from spike run `ws-20260903T041927Z`
(Claude Desktop 1.44121.4, bundled Claude Code 2.1.258, macOS 26.5.2).
Session `6abc89a2-bf72-4e5f-b5c6-be093c9930b4`.

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

## Redaction