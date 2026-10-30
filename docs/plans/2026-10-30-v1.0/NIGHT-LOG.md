# Night log, goalcup 1.0 build

Keys (checked 2026-10-30 at session start; names only, no values):

| Variable | State |
|---|---|
| CLAUDE_API_KEY | set |
| OPENAI_API_KEY | set |
| DEEPSEEK_API_KEY | set |
| GEMINI_API_KEY | set |
| AI_GATEWAY_API_KEY | set |
| VERCEL_TOKEN | set |
| CLOUDFLARE_API_TOKEN | set |
| CLOUDFLARE_ACCOUNT_ID | set |
| DATABASE_URL | set |
| GOALCUP_TEST_APPROVAL_KEY | set |
| GOALCUP_TEST_MEMBER_ID | set |
| GH_TOKEN | set |
| Vercel team id | set, as `VERCEL_TEAM_ID` (also `VERCEL_PROJECT_ID` present) |
| Pad push token (owner input, PRD-12 section 7) | no secret with a clear name; rows say "push not tested" until it is given |

## Log

10:32Z | integrator conductor | start | H+0. Night dir /root/gc-night (lane logs in /root/gc-night/lanes/<lane>.log, outside the repo). Box tool /root/gc-tools/box.mjs. Workflow scripts live where the workflow tool saves them. Mode: lane workflows in this one session.
10:32Z | integrator conductor | decision | Vercel Sandbox probe: box up in under 1 s, 8 vCPU, 16 GB, non-root user is `vercel-sandbox` (not `ubuntu`); the gate runs as that non-root user. Plan tier and boxes-at-once limit: not readable from the SDK; read by load (queue on refusal).
10:32Z | integrator conductor | start | v1.0/integration branched from main 4b226de and pushed; push to v1.0/* works.
10:32Z | integrator conductor | merge | smart/intelligent-newton-0avrpi merged into v1.0/integration (docs only; diff --name-only shows nothing outside docs/plans/).
