export const meta = {
  name: 'gc-lane',
  description: 'One goalcup 1.0 build lane: per PRD, plan a slice, build tasks in worktrees, review A/B(/breaker), fix, merge via merge.sh, replan',
  phases: [
    { title: 'Plan' }, { title: 'Build' }, { title: 'Review' }, { title: 'Fix' }, { title: 'Merge' }, { title: 'Replan' },
  ],
}

const A = args
const LANE = A.lane
const COMMON = `
You are one agent in the goalcup 1.0 build, lane "${LANE}". Repo: /home/user/goal-cup-proxy .
Integration branch: origin/v1.0/integration. Always \`git fetch origin\` first and read docs from that branch (git show origin/v1.0/integration:<path> or a checkout).
The plan lives in docs/plans/2026-10-30-v1.0/: OWNER-DECISIONS.md (the ledger, wins over everything), PRD-00-index.md (architecture, "File ownership", "Seams" S1..S18 with final shapes, "Build order", "Size budget"), PRD-01-record-format.md, PRD-16 (merge rule section 4), your PRD and the Amendments notes at its end. VISION.md and AGENTS.md hard rules hold (as ledger section 7 restates them).

Rules for you:
- Work in your OWN git worktree (no other agent uses it): \`git -C /home/user/goal-cup-proxy fetch -q origin && git -C /home/user/goal-cup-proxy worktree add /root/gc-wt/<role>-<short-unique-name> <start point>\` (use a detached start point or \`-b <new branch> origin/<base>\`, or \`--track -B <branch> origin/<branch>\` for an existing branch); do all edits and commits there; remove it with \`git -C /home/user/goal-cup-proxy worktree remove --force <dir>\` when you are done. Never edit /home/user/goal-cup-proxy's own checkout.
- NEVER compile, run cargo, npm test, python tests or any build output on this machine. Every build/test runs in a Vercel Sandbox build box: commit your work, write a bash job script to a temp file (begin it with \`cargo fetch --locked -q 2>/dev/null || cargo fetch -q\` when Rust is involved), then run
  node /root/gc-tools/box.mjs run --repo <your git dir> --ref <your branch or HEAD sha> --script <job file> --minutes 40 [--out <path in box>:<local path>]
  The box clones that committed ref into ~/w and runs the script there as a non-root user (Rust 1.99 from rust-toolchain.toml, cargo-nextest, zstd, Node 22, Python 3). Batch checks into one job; a cold Rust build takes several minutes. For long jobs use Bash run_in_background and wait for the notification. A box failing to start: retry later, never fall back to running here.
- Every temp file (job scripts, box output) comes from mktemp (for example J=$(mktemp /root/gc-wt/job.XXXXXX)); never a fixed name in a shared folder, other agents run at the same time. box.mjs --ref takes a branch name or a commit sha. Python in boxes is 3.9 by default; use python3.11 (sudo dnf install -y -q python3.11) when a script needs 3.10+.
- Never read, print, log, or put into argv/files any key, token or secret env value. No key or token enters a build box.
- Edit only paths your PRD owns per PRD-00 "File ownership". Never edit VISION.md, OWNER-DECISIONS.md, BUILD-PROMPT.md. Never touch main, tags, production (ingest.agentcommunity.org, bucket goalcup-traces), or the website database.
- Push only v1.0/* branches. Never force-push. Merge commits only; never rebase a pushed branch. Never commit Cargo.lock from a task branch (the integrator refreshes it).
- Commit messages: conventional style, plain words, no model names or model ids, and end with exactly these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6
- Docs and comments: no em dashes, no model names or ids.
- The seven points; masking before any byte reaches disk; no drop without a GAP; no caps of our own (memory guards write a GAP and go on); no harness names in core code; no secret in fixtures; record raw, interpret later; one process, no IPC for bytes; no 0.7 compatibility; nothing PRD-13's no-return guard names comes back (daemon/IPC for bytes, our own caps, disk-full handling, signing, pins, termination proof, research decoders).
- Tests (ledger B10): automated tests only for the seven points, the format, masking, the never-stop rules and your PRD's acceptance cases. No golden snapshots of internal shapes, no wording asserts.
- A harness-specific part: read the vendor's current docs first (WebFetch) and cite the page in the commit message.
- Log notable events (start, decision, seam, fix, stop, owner question) as one line each appended to /root/gc-night/lanes/${LANE}.log in the form
  <HH:MM>Z | ${LANE} <role> <PRD> | <kind> | <plain text>
  using \`date -u +%H:%M\`. No secrets, no model names, no request bodies.
- Waiting on another lane (a seam or file not yet on origin/v1.0/integration): poll with \`git fetch\` using a background Bash loop or the Monitor tool, about every 3 minutes, up to 90 minutes; build against the seam's PRD-00 final shape meanwhile if you can.
- Report outcomes as they are. A skipped check is said to be skipped.
`

const PLAN = { type: 'object', properties: {
  done: { type: 'boolean', description: 'true if the PRD is fully built and merged; no more slices' },
  slice: { type: 'string' },
  prd_branch: { type: 'string' },
  notes: { type: 'string' },
  tasks: { type: 'array', items: { type: 'object', properties: {
    id: { type: 'string', description: 'short kebab slug' }, files: { type: 'array', items: { type: 'string' } }, description: { type: 'string' } },
    required: ['id', 'files', 'description'] } },
}, required: ['done', 'slice', 'prd_branch', 'tasks', 'notes'] }
const BUILD = { type: 'object', properties: {
  branch: { type: 'string' }, pushed: { type: 'boolean' }, tests: { type: 'string', description: 'what ran in the box and the result' }, summary: { type: 'string' } },
  required: ['branch', 'pushed', 'tests', 'summary'] }
const REVIEW = { type: 'object', properties: {
  findings: { type: 'array', items: { type: 'object', properties: {
    id: { type: 'string' }, severity: { type: 'string', enum: ['blocker', 'fix', 'nit'] }, file: { type: 'string' }, text: { type: 'string' } },
    required: ['id', 'severity', 'text'] } },
  verdict: { type: 'string' } }, required: ['findings', 'verdict'] }
const FIX = { type: 'object', properties: {
  fixed: { type: 'array', items: { type: 'string' } },
  declined: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, reason: { type: 'string' } }, required: ['id', 'reason'] } },
  tests: { type: 'string' } }, required: ['fixed', 'declined', 'tests'] }
const MERGE = { type: 'object', properties: {
  merged: { type: 'boolean' }, commit: { type: 'string' }, gate_line: { type: 'string' }, problem: { type: 'string' } },
  required: ['merged', 'problem'] }

const NEVER_DECLINE = `Never declinable (blockers): a break of one of the seven points or a locked ledger item (backpressure on the wire, a recorder that stops for good, a count taken before its bytes are written, failed not sent while recording is off), a hard-rule break, a path outside ownership, a byte written to disk before masking, a drop with no GAP, a cap of our own, a harness name in core code, a secret in a fixture, a wrong count, a failing acceptance case of the PRD, a check made to pass without record evidence.`

async function reviewRound(p, branch, slice, prior, noBreaker) {
  const base = `${COMMON}\nYou are a fresh, independent reviewer of merge request branch origin/${branch} (${p.id}, slice "${slice}") against origin/v1.0/integration. Read the diff yourself: \`git fetch origin && git diff origin/v1.0/integration...origin/${branch}\`. Read docs/plans/2026-10-30-v1.0/${p.file} and the ledger. Do not trust any summary. You do not edit code; you return findings. You may run checks in a build box (box.mjs) on that branch. ${prior ? 'Earlier findings and the fixer\'s answer:\n' + prior + '\nRe-check: return only findings still open (and any new ones the fix introduced). Accept a decline only if its reason is sound and it is declinable.' : ''}\n${NEVER_DECLINE}\nSeverity: blocker for the never-declinable list, fix for real defects, nit for taste. A PRD-00 'Size budget' overage by itself is a nit (those are owner estimates; the replan reports wc -l against them); raise a size finding as fix only with a concrete cut that keeps behaviour, and then the cut itself is the finding. Write your findings before anything else; do not look for other reviewers' notes.${p.contract ? '\nREVIEW CONTRACT for this slice (judge the slice against these items and the never-declinable list; a finding that belongs to a later slice of this PRD and is not never-declinable goes in as a nit whose text starts with "next slice:"):\n' + p.contract : ''}`
  const roles = [
    agent(`${base}\nYou are REVIEWER A: correctness, the seven points, the ledger, masking before disk, no drop without a GAP, no harness names in core code, no secret in fixtures, no path outside ${p.id}'s ownership (PRD-00), PRD-13's no-return guard, no 0.7 shim, the PRD's acceptance cases.`, { label: `A:${p.id}`, phase: 'Review', schema: REVIEW }),
    agent(`${base}\nYou are REVIEWER B: code quality and less code. Duplication, dead paths, needless caps, anything that serves no point, overbuilt tests (pins internal shapes or wording, or costs more to change than the code it guards), and ${p.id}'s row in PRD-00 "Size budget" (count lines with wc -l). A less-code finding is only worth raising if the change makes the code smaller or clearer.`, { label: `B:${p.id}`, phase: 'Review', schema: REVIEW, model: CHEAP }),
  ]
  if (p.core && !A.noBreaker && !noBreaker) roles.push(agent(`${base}\nYou are the BREAKER. Try to break the diff against run 1's shape: about 40 and then about 200 synthetic subagents, 1.5 MB bodies in bursts, a held writer, WebSockets. Use PRD-02's tests/pipeline_guard.rs, tests/pipeline_stall.rs and by_hand_load if they exist on the branch, and write throwaway stress code in a box if they do not (never commit it). Never build a run 1 replay tool. Report what broke (lost bytes without a GAP, blocking the relay, counts before write, memory blowup, stops for good) as blockers.`, { label: `X:${p.id}`, phase: 'Review', schema: REVIEW }))
  const res = await Promise.all(roles)
  const dead = res.map((r, i) => r ? null : ['A', 'B', 'X'][i]).filter(Boolean)
  const out = res.filter(Boolean).flatMap((r, i) => r.findings.map(f => ({ ...f, by: ['A', 'B', 'X'][i] })))
  for (const d of dead) out.push({ id: 'REVIEWER-' + d + '-DID-NOT-REPORT', severity: 'blocker', text: 'reviewer ' + d + ' died before reporting; this slice is not reviewed', by: 'conductor', dead: true })
  return out
}

const results = []
const CHEAP = 'sonnet'
function heavy(p) { return p.core ? {} : { model: CHEAP } }
for (const p of A.prds) {
  let doneSlices = []
  for (let s = 0; s < (A.maxSlices || 6); s++) {
    const pend = (s === 0 && p.pending) ? p.pending : null
    const plan = pend ? { done: false, slice: pend.slice, prd_branch: pend.branch, notes: 'resumed', tasks: [{ id: 'resumed', files: [], description: '' }] } : await agent(`${COMMON}\nYou are the PLANNER for ${p.id} (docs/plans/2026-10-30-v1.0/${p.file}). PRD branch: v1.0/${p.slug} (create it from origin/v1.0/integration and push it if it does not exist; if it exists, merge origin/v1.0/integration into it with a merge commit and push).
${A.laneNote || ''}
${p.note || ''}
Slices already done by this lane: ${doneSlices.length ? doneSlices.join('; ') : 'none'}.
Read the PRD (and its Amendments notes), the ledger, PRD-00's ownership, seams and Build order, PRD-17 LESSONS.md if present, and the current code on integration. Decide the NEXT slice of ${p.id} to build: small, green-able, mergeable on its own (PRD-00 phase 0 stubs first, then the core slice, second slice, third slice parts that belong to ${p.id}). Return done=true only if nothing of ${p.id} is left to build in this build. Split the slice into 1 to 4 tasks that can be built in parallel without touching the same files; every file must be inside ${p.id}'s owned paths. Each task description must be complete enough for a builder who reads the PRD: what to write, which PRD sections, what tests (B10), what proves it works.`, { label: `plan:${p.id}#${s + 1}`, phase: 'Plan', schema: PLAN, ...heavy(p) })
    if (!plan || plan.done || !plan.tasks.length) { log(`${p.id}: planner says done (${plan ? plan.notes.slice(0, 120) : 'no plan'})`); break }
    const prdBranch = plan.prd_branch || `v1.0/${p.slug}`
    log(`${p.id} slice ${s + 1}: ${plan.slice} (${plan.tasks.length} tasks)`)

    const built = pend ? [{ pushed: true, branch: pend.branch }] : (await parallel(plan.tasks.map(t => () => agent(`${COMMON}\nYou are a BUILDER for ${p.id} (docs/plans/2026-10-30-v1.0/${p.file}), slice "${plan.slice}". Use your own git worktree (rule above).
Task "${t.id}": ${t.description}
Files you may touch: ${t.files.join(', ')} (all inside ${p.id}'s ownership; touch nothing else).
Planner notes: ${plan.notes}
Steps: git fetch origin; create branch v1.0/${p.slug}-${t.id} from origin/${prdBranch}; build it; commit; run its tests in a build box (box.mjs) until green (and \`cargo build\`/clippy for the crate if Rust); push the branch with -u. Return the branch, whether pushed, and what ran in the box with the result.`, { label: `build:${p.id}:${t.id}`, phase: 'Build', schema: BUILD, ...heavy(p) })))).filter(Boolean)
    const okBranches = built.filter(b => b.pushed).map(b => b.branch)
    if (!okBranches.length) { log(`${p.id}: no task branch pushed in slice ${plan.slice}; stopping this PRD`); results.push({ prd: p.id, slice: plan.slice, merged: false, problem: 'no task branch pushed' }); break }

    if (!pend) await agent(`${COMMON}\nYou are the LANE MERGER for ${p.id}. In your worktree: git fetch origin; check out ${prdBranch} (tracking origin/${prdBranch}); merge origin/v1.0/integration and then each task branch with merge commits: ${okBranches.join(', ')}. Resolve conflicts inside ${p.id}'s files only. Run the PRD's tests plus cargo build and clippy (if Rust) in one build box on the result; fix small breakages from the joining yourself. Push ${prdBranch}. Return a short text: the head sha and the box result.`, { label: `join:${p.id}`, phase: 'Build', model: CHEAP })

    if (pend && pend.prepare) await agent(`${COMMON}\nYou are the PREPARER for ${p.id}, branch ${pend.branch}, slice "${pend.slice}". The lane stopped part way (usage limit). ${pend.prepare}\nIn your own worktree on ${pend.branch} (tracking origin): merge origin/v1.0/integration and every pushed task branch v1.0/${p.slug}-* of this slice that is not yet in it (merge commits); finish any task the note says is unfinished, inside ${p.id}'s owned paths; run the PRD's tests plus cargo build and clippy in one build box; push. Return the head sha and the box result.`, { label: `prep:${p.id}`, phase: 'Build', ...heavy(p) })
    let findings = (pend && pend.findings && pend.findings.length) ? pend.findings : await reviewRound(p, prdBranch, plan.slice, null)
    if (pend && pend.decision) findings = findings.map(f => ({ ...f, conductor_decision: pend.decision }))
    let round = 0
    while (findings.some(f => f.severity !== 'nit' && !f.dead) && round < (p.core ? (A.maxRounds || 6) : 3)) {
      round++
      log(`${p.id} ${plan.slice}: review round ${round}: ${findings.filter(f => f.severity === 'blocker').length} blockers, ${findings.filter(f => f.severity === 'fix').length} fixes`)
      const fx = await agent(`${COMMON}\nYou are the FIXER for ${p.id} on branch ${prdBranch} (in your worktree; fetch, check it out tracking origin). Fix every finding below; a nit you may skip. You may decline a finding only with a one-line reason, and only if it is declinable. ${NEVER_DECLINE}\nFindings:\n${JSON.stringify(findings, null, 1)}\nRe-run the tests in a build box until green, commit, push ${prdBranch}.`, { label: `fix:${p.id}#${round}`, phase: 'Fix', schema: FIX, ...heavy(p) })
      const prior = JSON.stringify({ findings, answer: fx }, null, 1)
      findings = await reviewRound(p, prdBranch, plan.slice, prior, pend && pend.noBreaker)
    }
    if (findings.some(f => f.dead)) { log(`${p.id} ${plan.slice}: a reviewer died; not merged`); results.push({ prd: p.id, slice: plan.slice, merged: false, problem: 'reviewer died (usage limit?)' }); break }
    if (findings.some(f => f.severity === 'blocker' || f.severity === 'fix')) {
      log(`${p.id} ${plan.slice}: open findings after 3 rounds, not merged`)
      await agent(`${COMMON}\nAppend one line to /root/gc-night/lanes/${LANE}.log: kind "stop", text: "${p.id} ${plan.slice} not merged: findings still open after the last review round: ${findings.filter(f => f.severity !== 'nit').map(f => f.id).join(', ')} (branch ${prdBranch})". Then return "ok".`, { label: `log:${p.id}`, phase: 'Merge', effort: 'low', model: 'haiku' })
      results.push({ prd: p.id, slice: plan.slice, merged: false, problem: 'open findings', findings })
      break
    }

    let mr = null
    const REQ = { type: 'object', properties: { queued: { type: 'boolean' }, id: { type: 'string', description: 'the request id (file name without .req), empty if not queued' }, problem: { type: 'string' } }, required: ['queued', 'id', 'problem'] }
    const WAIT = { type: 'object', properties: { done: { type: 'boolean' }, exit: { type: 'number' }, commit: { type: 'string' }, gate_line: { type: 'string' }, gate_log: { type: 'string' } }, required: ['done'] }
    for (let tryN = 0; tryN < 3; tryN++) {
      const rq = await agent(`${COMMON}\nYou are the MERGE REQUESTER for ${p.id}, branch ${prdBranch}, slice "${plan.slice}". Reviewers have no open findings.${mr && mr.problem ? ' Previous attempt: ' + mr.problem : ''}
1. git fetch origin; list \`git diff --name-only origin/v1.0/integration...origin/${prdBranch}\` and check every path is owned by ${p.id} (PRD-00 "File ownership" and the PRD's "Files it owns"). Exceptions: a seam change approved in the night log, and the integrator's one Cargo.toml members line. If any path is outside ownership, return queued=false naming the paths.
2. If this branch creates a crate that must join the workspace (crates/goalcup-format, crates/goalcup, crates/goalcup-verify) and the workspace Cargo.toml on the branch does not list it in members, add exactly that members line in a commit "chore(workspace): add <crate> to members (integrator, PRD-16)" and push.
${mr && mr.gate_log ? '3a. The last gate run FAILED; read ' + mr.gate_log + '. If the cause is in ' + p.id + "'s files, fix it on " + prdBranch + ' (own worktree, test in a box, push). If it is in another PRD\'s files, return queued=false naming the failing check and its owner.\n' : ''}3. Queue the merge: write the JSON {"branch": "${prdBranch}", "message": "<${p.id} and the slice in a few words>", "refresh_lock": <true if the diff touches any Cargo.toml>} to a new file /root/gc-night/mrq/<id>.req where <id> is \`date -u +%H%M%S\`-${p.slug} (write it to a .tmp name first, then mv). A queue run by the conductor merges one request at a time with gate fast in a build box, and holds requests until PRD-14's gate change is on integration. Do not run merge.sh yourself. Return queued=true and the id.`, { label: `mreq:${p.id}`, phase: 'Merge', schema: REQ, model: CHEAP })
      if (rq && rq.queued && !rq.id) rq.queued = false, rq.problem = 'requester returned no id'
      if (!rq || !rq.queued) { mr = { merged: false, problem: rq ? rq.problem : 'requester died' }; break }
      let w = null
      for (let k = 0; k < 40 && !(w && w.done); k++) {
        w = await agent(`Wait for the merge queue result file /root/gc-night/mrq/${rq.id}.res (the conductor drives the queue; you only read). Run exactly this one Bash command with a 590000 ms timeout: \`timeout 560 bash -c 'until [ -f /root/gc-night/mrq/${rq.id}.res ]; do sleep 15; done'; cat /root/gc-night/mrq/${rq.id}.res 2>/dev/null || echo NOT_YET\`. If it printed NOT_YET return done=false. Otherwise it is JSON with exit (0 merged, 1 gate FAIL, 3 conflict, 4 push refused), output and gate_log: return done=true, exit, the merge commit sha and the 'gate fast: ...' line if the output has them, and gate_log. Do nothing else.`, { label: `mwait:${p.id}`, phase: 'Merge', effort: 'low', schema: WAIT, model: 'haiku' })
      }
      if (!w || !w.done) { mr = { merged: false, problem: 'merge queue gave no result in about 6 hours' }; break }
      mr = { merged: w.exit === 0, commit: w.commit, gate_line: w.gate_line, gate_log: w.exit === 1 ? w.gate_log : '', problem: w.exit === 0 ? '' : (w.exit === 3 ? 'merge conflict with integration' : w.exit === 4 ? 'push refused' : 'gate fast FAIL') }
      if (mr.merged || w.exit === 3 || w.exit === 4) break
    }
    results.push({ prd: p.id, slice: plan.slice, merged: !!(mr && mr.merged), commit: mr && mr.commit, gate: mr && mr.gate_line, problem: mr && mr.problem })
    if (!mr || !mr.merged) { log(`${p.id} ${plan.slice}: merge failed: ${mr ? mr.problem : 'agent died'}`); break }
    doneSlices.push(plan.slice)
    log(`${p.id} ${plan.slice}: merged ${mr.commit} ${mr.gate_line}`)

    if (!A.noReplan) await agent(`${COMMON}\nYou are the REPLAN agent after the merge of ${p.id} slice "${plan.slice}" (merge ${mr.commit}). Read what was learned: the review findings and fixes below, the merged diff, gate results, and recent lane logs in /root/gc-night/lanes/. Decide what affects the remaining PRDs and PRD-16 (the build plan). If anything does, append an "## Amendments" note (or a dated entry under an existing one) at the END of each affected PRD file in docs/plans/2026-10-30-v1.0/ (never rewrite a section; never edit OWNER-DECISIONS.md, VISION.md, BUILD-PROMPT.md; a note may not change a locked ledger item: if one seems to need changing, write a line of kind "owner question" with W6 in the log instead). Also report \`wc -l\` of the merged ${p.id} code against its PRD-00 "Size budget" row in the PRD-16 note. Commit docs-only on integration under the lock:
  flock /root/gc-night/integration.lock bash -c 'cd /home/user/goal-cup-proxy && git fetch -q origin && git worktree add -q --detach /root/gc-night/replan-wt-$$ origin/v1.0/integration' ; edit files there; commit; push HEAD:v1.0/integration (still inside the lock; retry fetch+rebase-free re-apply if the push is refused); remove the worktree.
Append an "amendment" line to /root/gc-night/lanes/${LANE}.log. Findings and answers:\n${JSON.stringify(findings).slice(0, 6000)}\nReturn one short paragraph of what you changed.`, { label: `replan:${p.id}`, phase: 'Replan', model: CHEAP })
  }
}
return results
