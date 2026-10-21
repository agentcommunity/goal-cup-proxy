// What the server does on its own, every 5 minutes (the `scheduled` export; wrangler.toml has the
// cron): close runs that are over, record the judged GitHub commit of each run that reached its
// deadline, queue the run emails into the outbox, and send them (src/mail.ts: each queued row once, through Resend; the
// database writes the email and picks the address, and gives a staging Worker none). Contract: docs/UPLOAD-API.md,
// "What the server does on its own".
//
// Every notice is decided here, in `queueNotices`, never in a request handler. A notice is
// stored once per (run, kind, dedupe key), so a repeated or a concurrent tick queues nothing twice.

import { type Ctx, makeCtx } from "./context.ts";
import { type FinalCommitDetail, NO_FINAL_DETAIL, type NoticeKind, type TickRun, endsAt, isReplaced, statusAt } from "./db.ts";
import type { Env } from "./env.ts";
import { sendNotices } from "./mail.ts";
import { githubHeaders } from "./runs.ts";
import { REPO_URL, addSeconds, formatTime } from "./util.ts";

/** A run with no heartbeat for this long is silent. */
export const SILENCE_AFTER = 20 * 60;
/** Hours without a model request after which an idle notice goes out, one notice each. */
export const IDLE_HOURS = [3, 6] as const;
/** The ended notice waits this long for the judged commit, then goes out without one. */
export const ENDED_WAIT = 3600;
/** The most runs one tick asks GitHub about (oldest first; the rest wait for the next tick). Each takes at most 2 + ACTIVITY_PAGES calls. */
export const COMMITS_PER_TICK = 50;

const earlier = (now: Date, seconds: number) => formatTime(addSeconds(now, -seconds));

/** Do `work` for each run; one that throws is logged (its id, the message) and the rest still get their turn. Returns the sum. */
async function eachRun(step: string, runs: TickRun[], work: (run: TickRun) => Promise<number>): Promise<number> {
  let total = 0;
  for (const run of runs) {
    try {
      total += await work(run);
    } catch (e) {
      console.error(`tick ${step} ${run.run_id}: ${message(e)}`);
    }
  }
  return total;
}

/** Write the close of every run that is over: `sealed`, or `incomplete` once the close time has passed. Returns how many. */
export async function closeRuns(ctx: Ctx, now: Date): Promise<number> {
  return eachRun("closeRuns", await ctx.db.runsForTick(formatTime(now)), async (run) => {
    if (statusAt(run, now, ctx.cfg.closeAfterSeconds) === run.status) return 0;
    await ctx.db.closeRun(run.member_id, run.run_id, formatTime(now));
    return 1;
  });
}

/**
 * GitHub gets this long after `ends_at` to answer properly. Until then a 404 (a repository briefly
 * private, or a transient 404) and a failing activity API are retried each tick; after it a 404 is
 * kept as `none` and a failing activity API falls back to `commits/HEAD`.
 */
export const GITHUB_GRACE = 3600;
/** GitHub is first asked this long after `ends_at`, so a push it received just before has reached its activity list. */
export const GITHUB_SETTLE = 300;
/** The most activity pages (100 activities each) read for one run. */
export const ACTIVITY_PAGES = 5;

const SHA = /^[0-9a-f]{40}$/;
const ZERO_SHA = "0".repeat(40);

type GithubAnswer = { status: number; body: unknown; next: string | null } | null;

/** One GET to the GitHub API: status, JSON body (null if none) and the Link header's next page; null when GitHub could not be reached. */
async function githubGet(ctx: Ctx, url: string): Promise<GithubAnswer> {
  try {
    const res = await fetch(url, { headers: githubHeaders(ctx.env), signal: AbortSignal.timeout(8000) });
    const next = /<(https:\/\/api\.github\.com\/[^>]+)>;\s*rel="next"/.exec(res.headers.get("link") ?? "")?.[1] ?? null;
    let body: unknown = null;
    try {
      body = res.status === 200 ? await res.json() : null;
    } catch {
      body = null;
    }
    return { status: res.status, body, next };
  } catch {
    return null;
  }
}

/** One update of a branch as the activity API reports it. */
interface BranchActivity {
  after: string;
  before: string;
  /** Milliseconds since the epoch: when GitHub received it. */
  at: number;
  force: boolean;
}

/**
 * The repository activity API's answer, read in one place so a change of its shape is fixed here.
 * GET /repos/{owner}/{repo}/activity?ref=<branch>&per_page=100 (newest first, `desc` is the default),
 * every activity type (`activity_type` takes only one value; push, force_push, pr_merge,
 * merge_queue_merge and branch_creation all move the branch, and a branch_deletion leaves it at the
 * zero sha). Each item has `before`, `after`, `ref`, `timestamp` and `activity_type`. Null when an item
 * does not have that shape.
 */
function readActivity(body: unknown, branch: string): BranchActivity[] | null {
  if (!Array.isArray(body)) return null;
  const out: BranchActivity[] = [];
  for (const item of body as Record<string, unknown>[]) {
    const { after, before, ref, timestamp, activity_type } = item ?? {};
    if (typeof after !== "string" || !SHA.test(after) || typeof before !== "string" || !SHA.test(before)) return null;
    if (typeof timestamp !== "string" || Number.isNaN(Date.parse(timestamp)) || typeof activity_type !== "string") return null;
    if (typeof ref === "string" && ref !== branch && ref !== `refs/heads/${branch}`) continue; // the filter is the server's; check it
    out.push({ after, before, at: Date.parse(timestamp), force: activity_type === "force_push" });
  }
  return out;
}

/**
 * The branch's updates, newest first, read back to the run's start (or ACTIVITY_PAGES pages): null
 * when the activity API did not answer with a list (the caller falls back). `complete` is false when
 * the page limit stopped the read before the start.
 */
async function branchActivity(ctx: Ctx, path: string, branch: string, startMs: number): Promise<{ items: BranchActivity[]; complete: boolean } | null> {
  const items: BranchActivity[] = [];
  let url: string | null = `https://api.github.com/repos/${path}/activity?ref=${encodeURIComponent(`refs/heads/${branch}`)}&per_page=100`;
  for (let page = 0; url !== null; page++) {
    if (page === ACTIVITY_PAGES) return { items, complete: false };
    const res = await githubGet(ctx, url);
    if (res === null || res.status !== 200) return null;
    const got = readActivity(res.body, branch);
    if (got === null) return null;
    items.push(...got);
    if (got.some((a) => a.at < startMs)) break; // older pages are from before the run
    url = res.next;
  }
  return { items, complete: true };
}

/** What `commits/HEAD` says: the head sha, `none` for a repository GitHub does not have or that is empty, null when it could not say. */
async function headCommit(ctx: Ctx, path: string): Promise<string | null> {
  const res = await githubGet(ctx, `https://api.github.com/repos/${path}/commits/HEAD`);
  if (res === null) return null;
  if (res.status === 404 || res.status === 409) return "none";
  const sha = res.status === 200 ? (res.body as { sha?: unknown } | null)?.sha : undefined;
  return typeof sha === "string" && SHA.test(sha) ? sha : null;
}

type Judged = { commit: string; detail: FinalCommitDetail };

/**
 * The judged commit of a run (owner, 21 Oct 2026): the last push to the repository's default branch
 * that GitHub timestamped at or before `ends_at`. Later pushes are not judged (`late_push`); a force
 * push during the run is flagged (`force_push`). Null: not decided yet, ask again next tick.
 */
async function judgedCommit(ctx: Ctx, run: TickRun, now: Date): Promise<Judged | null> {
  if (!REPO_URL.test(run.repo_url)) return null;
  const path = run.repo_url.slice("https://github.com/".length);
  const endsMs = Date.parse(endsAt(run));
  const graceOver = now.getTime() >= endsMs + GITHUB_GRACE * 1000;

  const repo = await githubGet(ctx, `https://api.github.com/repos/${path}`);
  if (repo === null) return null;
  if (repo.status === 404) return graceOver ? { commit: "none", detail: NO_FINAL_DETAIL } : null;
  if (repo.status !== 200) return null;
  const branch = (repo.body as { default_branch?: unknown } | null)?.default_branch;
  const activity = typeof branch === "string" && branch !== "" ? await branchActivity(ctx, path, branch, Date.parse(run.official_start)) : null;

  if (activity === null) {
    // The activity API is unavailable: after the grace hour, the head GitHub has now, flagged.
    if (!graceOver) return null;
    const head = await headCommit(ctx, path);
    if (head === null) return null;
    console.error(`final commit ${run.run_id}: the activity API is unavailable; recorded commits/HEAD (head_fallback)`);
    return { commit: head, detail: { ...NO_FINAL_DETAIL, branch: typeof branch === "string" ? branch : null, head_fallback: true } };
  }

  const { items, complete } = activity;
  if (!complete) console.error(`final commit ${run.run_id}: more than ${ACTIVITY_PAGES} activity pages; force_push covers only the newest`);
  const late = items.filter((a) => a.at > endsMs);
  const flags = {
    branch: branch as string,
    late_push: late.length > 0,
    force_push: items.some((a) => a.force && a.at <= endsMs && a.at >= Date.parse(run.official_start)),
  };
  const last = items.find((a) => a.at <= endsMs);
  if (last !== undefined) {
    const commit = last.after === ZERO_SHA ? "none" : last.after; // the branch was deleted at the deadline
    return { commit, detail: { ...flags, pushed_at: formatTime(new Date(last.at)), head_fallback: false } };
  }
  if (late.length > 0) {
    // No update at or before the deadline was listed: the branch stood where the first later push found it.
    const before = late.at(-1)!.before;
    return { commit: before === ZERO_SHA ? "none" : before, detail: { ...flags, pushed_at: null, head_fallback: false } };
  }
  // No activity at all on the branch: it has not moved, so its head now is its head at the deadline (flagged all the same).
  const head = await headCommit(ctx, path);
  if (head === null) return null;
  return { commit: head, detail: { ...flags, pushed_at: null, head_fallback: true } };
}

/**
 * For runs that ended at their deadline: decide the judged commit (`judgedCommit`) and keep it, with
 * how it was found. Asked from the first tick at or after `ends_at` + GITHUB_SETTLE. Returns how many were recorded.
 */
export async function recordFinalCommits(ctx: Ctx, now: Date): Promise<number> {
  const due = (await ctx.db.runsForTick(formatTime(now))).filter(
    (r) => r.mode === "competition" && !isReplaced(r) && r.final_commit === null && formatTime(addSeconds(new Date(endsAt(r)), GITHUB_SETTLE)) <= formatTime(now),
  );
  if (due.length > 0 && !ctx.env.GITHUB_TOKEN && ctx.env.DB_BACKEND !== "local") {
    console.error("WARNING: GITHUB_TOKEN is not set. Production must set it: unauthenticated GitHub calls share one small rate limit and the judged commits may not be read.");
  }
  return eachRun("recordFinalCommits", due.slice(0, COMMITS_PER_TICK), async (run) => {
    const judged = await judgedCommit(ctx, run, now);
    if (judged === null) return 0;
    await ctx.db.recordFinalCommit(run.member_id, run.run_id, judged.commit, formatTime(now), judged.detail);
    return 1;
  });
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
  // A disk pause the heartbeats marked as owed (at most one per run per 6 hours, decided where the
  // heartbeat is stored): one email per pause start, whether or not the run is still paused or beating.
  if (run.disk_notice_at != null) notices.push({ kind: "disk_full", key: run.disk_notice_at, payload: { since: run.disk_notice_at } });

  const heard = run.last_heartbeat_at ?? run.official_start;
  if (heard < earlier(now, SILENCE_AFTER)) {
    notices.push({ kind: "silence", key: heard, payload: { last_heartbeat_at: run.last_heartbeat_at } });
  } else if (run.last_heartbeat_at !== null && run.last_model_request_at !== null) {
    // The run is beating, so its agent is what is quiet. (A run that has never reported
    // model_requests is not judged: goalcup may not send them.)
    for (const hours of IDLE_HOURS) {
      if (run.last_model_request_at < earlier(now, hours * 3600)) {
        notices.push({ kind: hours === 3 ? "idle_3h" : "idle_6h", key: run.last_model_request_at, payload: { hours, since: run.last_model_request_at } });
      }
    }
  }
  return notices;
}

/**
 * Queue the notices that are due, all in one database call (a call per notice would pass the
 * Worker's subrequest limit at about a thousand open runs, and every open run owes `started`
 * every tick). Returns how many were queued by this call (not already there). If the call fails the
 * tick logs it and the next tick sends the same batch again: the key makes that safe.
 */
export async function queueNotices(ctx: Ctx, now: Date): Promise<number> {
  const at = formatTime(now);
  const batch = (await ctx.db.runsForTick(at))
    .filter((r) => r.mode === "competition" || r.mode === "test") // test runs mail their operator ("[TEST]", from the database)
    .flatMap((run) => owed(run, now).map((n) => ({ run_id: run.run_id, member_id: run.member_id, kind: n.kind, dedupe_key: n.key, payload: n.payload, at })));
  return batch.length === 0 ? 0 : ctx.db.queueNotices(batch);
}

/** One cron tick. Each step is on its own: one that fails is logged and the others still run. */
export async function runTick(env: Env): Promise<void> {
  // While the maintenance switch is on no goalcup can beat, so silence would be false news.
  if (env.MAINTENANCE === "on") return;
  let ctx: Ctx;
  try {
    ctx = await makeCtx(env);
  } catch (e) {
    console.error(`tick: ${message(e)}`);
    return;
  }
  const now = ctx.now();
  for (const [name, step] of [["closeRuns", closeRuns], ["recordFinalCommits", recordFinalCommits], ["queueNotices", queueNotices], ["sendNotices", sendNotices]] as const) {
    try {
      await step(ctx, now);
    } catch (e) {
      console.error(`tick ${name}: ${message(e)}`);
    }
  }
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
