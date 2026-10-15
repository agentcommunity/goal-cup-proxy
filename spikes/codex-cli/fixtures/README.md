# Fixtures — spike 2, codex-cli (launch), run 2026-09-03

Redacted copies of the artifacts from spike run `ws-20260903T064347Z`
(codex-cli **0.153.0**, isolated install `harnesses/node/bin/codex`, isolated
`CODEX_HOME=harnesses/home/codex`, macOS 26.5.2 arm64). Tracked thread
`01a06614-8f11-79a1-abb9-0b0f45b7f104`.

## Files

| file | sha256 (first 16) | bytes | what it is |
|---|---|---|---|
| `rollout-main-01a06614.jsonl` | `04441b2e0d18be76` | 312,967 | the tracked thread's whole rollout, 161 records, **including the turns appended after `codex resume`** |
| `rollout-subagent-01a06615.jsonl` | `e3bd02af1960b1c7` | 75,750 | the subagent thread, 39 records; carries the parent link in `session_meta` |
| `hooks.jsonl` | `334d72982ed71e55` | 72,435 | leg 2, all 45 posts (post 1 is the operator's `/Ping` smoke test) |
| `history-excerpt.jsonl` | `900823778bbb4040` | 1,446 | all 11 `$CODEX_HOME/history.jsonl` records — the third witness, and the only place the hook-**blocked** 4,100-char prompt survives for the tracked thread |
| `wrong-cwd-01a06616-excerpt.jsonl` | `103fa336d0306cf4` | 2,310 | 2 records from the mis-launched session proving it enumerated the tracker's artifact root (`hooks.jsonl`, `leg1/capture.jsonl`, `baseline/files/auth.json`) |
| `tracker-hooks.json` | `afa41eb4c78d8e76` | 4,514 | the folder-scoped `.codex/hooks.json` the tracker wrote, **unmodified**. Full sha256 `afa41eb4c78d8e769e164cb84b87712b9c0cb83695112fd0ed546f187a8824e0` — this is the post-restructure hash quoted in the report |
| `tracker-hook-post.sh` | `5c2766a3cdea28c3` | 926 | the shim, **unmodified**. Full sha256 `5c2766a3cdea28c34b3d451ac2b6a4a380f492cf4761ac76ba215398545c55f4` |
| `probes/p1-trusted-exec.stderr.txt` | `4c873f29b54607e7` | 6,309 | P1: `codex exec` with **already-trusted** hooks and no bypass flag — the `hook: <Event>` lines are Codex's own trace |
| `probes/hooks-p1-trusted.jsonl` | `129cc77fafe445f0` | 10,424 | the 6 posts P1 produced |
| `probes/p1b-untrusted-exec.stderr.txt` | `f19aea49a5f72675` | 6,555 | P1b: same command, **untrusted** hooks. No `hook:` line, no warning, exit 0 |
| `probes/hooks-p1b-untrusted.jsonl` | (empty) | 0 | P1b produced zero posts. The empty file is the evidence |
| `probes/p2-nogit-exec.stderr.txt` | `943deb412f7ae279` | 6,517 | P2: non-git cwd, `--skip-git-repo-check --dangerously-bypass-hook-trust` |
| `probes/hooks-p2-nogit.jsonl` | `85b0b21e9ac8f401` | 10,375 | the 6 posts P2 produced — hooks **are** discovered outside a git repo |

Total 524 KB.

## Redaction