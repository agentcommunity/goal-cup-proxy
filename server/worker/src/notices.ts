// What the server does on its own, every 5 minutes (the `scheduled` export; wrangler.toml has the
// cron): close runs that are over, record the GitHub commit at each deadline, and queue the run
// emails into the outbox. The Worker never sends mail: the website's mailer reads the unsent
// rows of `notices`, writes the email and marks the row sent. Contract: docs/UPLOAD-API.md,
// "What the server does on its own".
//
// Every notice is decided here, in `queueNotices`, never in a request handler. A notice is
// stored once per (run, kind, dedupe key), so a repeated or a concurrent tick queues nothing twice.

import { type Ctx, makeCtx } from "./context.ts";
import { type NoticeKind, type TickRun, endsAt, isReplaced, statusAt } from "./db.ts";
import type { Env } from "./env.ts";
import { githubHeaders } from "./runs.ts";
import { REPO_URL, addSeconds, formatTime } from "./util.ts";

/** A run with no heartbeat for this long is silent. */
export const SILENCE_AFTER = 20 * 60;
/** Hours without a model request after which an idle notice goes out, one notice each. */
export const IDLE_HOURS = [3, 6] as const;
/** The ended notice waits this long for the deadline commit, then goes out without one. */
export const ENDED_WAIT = 3600;
/** The most runs one tick asks GitHub about (oldest first; the rest wait for the next tick). */
export const COMMITS_PER_TICK = 50;

const earlier = (now: Date, seconds: number) => formatTime(addSeconds(now, -seconds));

/** Write the close of every run that is over: `sealed`, or `incomplete` once the close time has passed. Returns how many. */
export async function closeRuns(ctx: Ctx, now: Date): Promise<number> {
  let closed = 0;
  for (const run of await ctx.db.runsForTick(formatTime(now))) {
    if (statusAt(run, now, ctx.cfg.closeAfterSeconds) === run.status) continue;
    await ctx.db.closeRun(run.member_id, run.run_id, formatTime(now));
    closed++;
  }
  return closed;
}

/**
 * The commit GitHub has at the head of the repository: its sha, `none` when GitHub says there is no
 * such repository or it is empty, null when it could not say (retry next tick).
 */
async function headCommit(ctx: Ctx, repoUrl: string): Promise<string | null> {
  if (!REPO_URL.test(repoUrl)) return null;
  const path = repoUrl.slice("https://github.com/".length);
  let res: Response;
  try {
    res = await fetch(`https://api.github.com/repos/${path}/commits/HEAD`, { headers: githubHeaders(ctx.env), signal: AbortSignal.timeout(8000) });
  } catch {
    return null;
  }
  if (res.status === 404 || res.status === 409) return "none";
  if (res.status !== 200) return null;
  try {
    const sha = ((await res.json()) as { sha?: unknown }).sha;
    return typeof sha === "string" && /^[0-9a-f]{40}$/.test(sha) ? sha : null;
  } catch {
    return null;
  }
}

/** For runs that ended at their deadline: ask GitHub for the head commit and keep it. Returns how many were recorded. */
export async function recordFinalCommits(ctx: Ctx, now: Date): Promise<number> {
  const due = (await ctx.db.runsForTick(formatTime(now))).filter(
    (r) => r.mode === "competition" && !isReplaced(r) && r.final_commit === null && endsAt(r) <= formatTime(now),
  );
  let recorded = 0;
  for (const run of due.slice(0, COMMITS_PER_TICK)) {
    const commit = await headCommit(ctx, run.repo_url);
    if (commit === null) continue;
    await ctx.db.recordFinalCommit(run.member_id, run.run_id, commit, formatTime(now));
    recorded++;
  }
  return recorded;
}

/** The notices a competition run is owed at `now`: kind, dedupe key, payload. */
function owed(run: TickRun, now: Date): { kind: NoticeKind; key: string; payload: Record<string, unknown> }[] {
  const at = formatTime(now);
  const ends = endsAt(run);
  if (at >= ends) {
    // Over. Only a run that ran to its deadline has an ended notice, with the commit if GitHub gave one by now.
    if (isReplaced(run)) return [];
    if (run.final_commit === null && at < formatTime(addSeconds(new Date(ends), ENDED_WAIT))) return [];
    const commit = run.final_commit === null || run.final_commit === "none" ? null : run.final_commit;
    return [{ kind: "ended", key: "", payload: { ends_at: ends, commit, repo_url: run.repo_url } }];
  }

  const notices: ReturnType<typeof owed> = [{ kind: "started", key: "", payload: { official_start: run.official_start, deadline_at: run.deadline_at, repo_url: run.repo_url } }];
  if (run.flagged_at !== null) notices.push({ kind: "recording", key: "", payload: { flagged_at: run.flagged_at, recording: run.flag_reason } });

  const heard = run.last_heartbeat_at ?? run.official_start;
  if (heard < earlier(now, SILENCE_AFTER)) {
    notices.push({ kind: "silence", key: heard, payload: { last_heartbeat_at: run.last_heartbeat_at } });
  } else if (run.last_heartbeat_at !== null) {
    // The run is beating, so its agent is what is quiet.
    for (const hours of IDLE_HOURS) {
      if (run.last_model_request_at < earlier(now, hours * 3600)) {
        notices.push({ kind: hours === 3 ? "idle_3h" : "idle_6h", key: run.last_model_request_at, payload: { hours, since: run.last_model_request_at } });
      }
    }
  }
  return notices;
}

/** Queue the notices that are due. Returns how many were queued by this call (not already there). */
export async function queueNotices(ctx: Ctx, now: Date): Promise<number> {
  let queued = 0;
  for (const run of await ctx.db.runsForTick(formatTime(now))) {
    if (run.mode !== "competition") continue;
    for (const n of owed(run, now)) {
      const added = await ctx.db.queueNotice({
        run_id: run.run_id,
        member_id: run.member_id,
        kind: n.kind,
        dedupe_key: n.key,
        payload: n.payload,
        at: formatTime(now),
      });
      if (added) queued++;
    }
  }
  return queued;
}

/** One cron tick. Each step is on its own: one that fails is logged and the others still run. */
export async function runTick(env: Env): Promise<void> {
  // While the maintenance switch is on tracers cannot beat, so silence would be false news.
  if (env.MAINTENANCE === "on") return;
  let ctx: Ctx;
  try {
    ctx = await makeCtx(env);
  } catch (e) {
    console.error(`tick: ${message(e)}`);
    return;
  }
  const now = ctx.now();
  for (const [name, step] of [["closeRuns", closeRuns], ["recordFinalCommits", recordFinalCommits], ["queueNotices", queueNotices]] as const) {
    try {
      await step(ctx, now);
    } catch (e) {
      console.error(`tick ${name}: ${message(e)}`);
    }
  }
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
