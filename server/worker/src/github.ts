// The GitHub API, read by the cron to find each run's judged commit (the last push to the default
// branch that GitHub timestamped at or before `ends_at`), and the headers every GitHub call carries
// (also the public-repo check in src/runs.ts).

import type { Ctx } from "./context.ts";
import { type FinalCommitDetail, NO_FINAL_DETAIL, type TickRun, endsAt } from "./db.ts";
import type { Env } from "./env.ts";
import { REPO_URL, formatTime } from "./util.ts";

/** The headers every call to the GitHub API carries (the token, if there is one, only raises the rate limit). */
export function githubHeaders(env: Env): Record<string, string> {
  const headers: Record<string, string> = {
    accept: "application/vnd.github+json",
    "user-agent": "goalcup-upload-worker",
    "x-github-api-version": "2022-11-28",
  };
  if (env.GITHUB_TOKEN) headers.authorization = `Bearer ${env.GITHUB_TOKEN}`;
  return headers;
}

/**
 * GitHub gets this long after `ends_at` to answer properly. Until then a 404 (a repository briefly
 * private, or a transient 404), any other error status, and a failing or incomplete activity API are
 * retried each tick. After it a 404 is kept as `none`, a failing or incomplete activity API falls
 * back to the flagged commit list, and an error status that still comes is kept as `none`, flagged.
 * Only a GitHub that cannot be reached at all is asked again for as long as the cron looks at the run.
 */
export const GITHUB_GRACE = 3600;
/** GitHub is first asked this long after `ends_at`, so a push it received just before has reached its activity list. */
export const GITHUB_SETTLE = 300;
/** The most activity pages (100 activities each) read for one run. */
export const ACTIVITY_PAGES = 5;

const SHA = /^[0-9a-f]{40}$/;
const ZERO_SHA = "0".repeat(40);

type GithubAnswer = { status: number; body: unknown; next: string | null } | null;

/**
 * One GET to the GitHub API: status, JSON body (null if none) and the Link header's next page; null
 * when GitHub could not be reached. Anything but a 200 is logged with the rate limit GitHub says is
 * left, and its body is cancelled unread.
 */
async function githubGet(ctx: Ctx, url: string): Promise<GithubAnswer> {
  const where = new URL(url).pathname;
  let res: Response;
  try {
    res = await fetch(url, { headers: githubHeaders(ctx.env), signal: AbortSignal.timeout(8000) });
  } catch (e) {
    console.warn(`github-unreachable ${where}: ${e instanceof Error ? e.message : String(e)}`);
    return null;
  }
  const next = /<(https:\/\/api\.github\.com\/[^>]+)>;\s*rel="next"/.exec(res.headers.get("link") ?? "")?.[1] ?? null;
  if (res.status !== 200) {
    console.warn(`github-status ${res.status} ${where} ratelimit-remaining=${res.headers.get("x-ratelimit-remaining") ?? "-"}`);
    await res.body?.cancel().catch(() => {});
    return { status: res.status, body: null, next };
  }
  let body: unknown = null;
  try {
    body = await res.json();
  } catch {
    body = null;
  }
  return { status: res.status, body, next };
}

/** `body[key]` when the body is a JSON object that has that key; undefined otherwise. */
function prop(body: unknown, key: string): unknown {
  return typeof body === "object" && body !== null && !Array.isArray(body) && Object.hasOwn(body, key) ? Reflect.get(body, key) : undefined;
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
  const listed: unknown[] = body;
  const out: BranchActivity[] = [];
  for (const item of listed) {
    const [after, before, ref, timestamp, activityType] = ["after", "before", "ref", "timestamp", "activity_type"].map((k) => prop(item, k));
    if (typeof after !== "string" || !SHA.test(after) || typeof before !== "string" || !SHA.test(before)) return null;
    if (typeof timestamp !== "string" || Number.isNaN(Date.parse(timestamp)) || typeof activityType !== "string") return null;
    if (typeof ref === "string" && ref !== branch && ref !== `refs/heads/${branch}`) continue; // the filter is the server's; check it
    out.push({ after, before, at: Date.parse(timestamp), force: activityType === "force_push" });
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

/** A GitHub answer that settles nothing: the status it came with, or null when GitHub could not be reached. */
type Failed = { failed: number | null };

/**
 * The flagged fallback, for when the activity API cannot say: the newest commit on the branch whose
 * commit date is at or before `ends_at` (`commits?sha=<branch>&until=<ends_at>`), else the head
 * GitHub has now (`commits/HEAD`). Commit dates are the pusher's to set and the head may be a later
 * push, so a row from here is flagged (`head_fallback`) for an organizer to review by hand. `none`
 * for a repository GitHub does not have or that is empty.
 */
async function fallbackCommit(ctx: Ctx, path: string, branch: string | null, ends: string): Promise<string | Failed> {
  if (branch !== null) {
    const query = `sha=${encodeURIComponent(branch)}&until=${encodeURIComponent(ends)}&per_page=1`;
    const listed = await githubGet(ctx, `https://api.github.com/repos/${path}/commits?${query}`);
    const sha = listed?.status === 200 && Array.isArray(listed.body) ? prop(listed.body[0], "sha") : undefined;
    if (typeof sha === "string" && SHA.test(sha)) return sha;
  }
  const res = await githubGet(ctx, `https://api.github.com/repos/${path}/commits/HEAD`);
  if (res === null) return { failed: null };
  if (res.status === 404 || res.status === 409) return "none";
  const sha = res.status === 200 ? prop(res.body, "sha") : undefined;
  return typeof sha === "string" && SHA.test(sha) ? sha : { failed: res.status };
}

type Judged = { commit: string; detail: FinalCommitDetail };

/**
 * GitHub answered with an error status. After the grace hour it is not going away (a rate limit,
 * 451, a lasting 5xx): `none` is kept, flagged for an organizer. Before it, or when GitHub could not
 * be reached at all, null: ask again next tick.
 */
function gaveUp(run: TickRun, graceOver: boolean, failed: number | null, branch: string | null): Judged | null {
  if (!graceOver || failed === null) return null;
  console.error(`final commit ${run.run_id}: GitHub still answers ${failed} an hour after the end; recorded none (head_fallback)`);
  return { commit: "none", detail: { ...NO_FINAL_DETAIL, branch, head_fallback: true } };
}

/** The fallback's answer as a judged commit, flagged; null when it settles nothing yet. */
function fromFallback(run: TickRun, got: string | Failed, graceOver: boolean, detail: FinalCommitDetail): Judged | null {
  if (typeof got !== "string") return gaveUp(run, graceOver, got.failed, detail.branch);
  return { commit: got, detail: { ...detail, pushed_at: null, head_fallback: true } };
}

/**
 * The judged commit of a run: the last push to the repository's default branch that GitHub
 * timestamped at or before `ends_at`. Later pushes are not judged (`late_push`); a force push during
 * the run is flagged (`force_push`). Null: not decided yet, ask again next tick.
 */
export async function judgedCommit(ctx: Ctx, run: TickRun, now: Date): Promise<Judged | null> {
  if (!REPO_URL.test(run.repo_url)) return null;
  const path = run.repo_url.slice("https://github.com/".length);
  const ends = endsAt(run);
  const endsMs = Date.parse(ends);
  const startMs = Date.parse(run.official_start);
  const graceOver = now.getTime() >= endsMs + GITHUB_GRACE * 1000;

  const repo = await githubGet(ctx, `https://api.github.com/repos/${path}`);
  if (repo === null) return null;
  if (repo.status === 404) return graceOver ? { commit: "none", detail: NO_FINAL_DETAIL } : null;
  if (repo.status !== 200) return gaveUp(run, graceOver, repo.status, null);
  const named = prop(repo.body, "default_branch");
  const branch = typeof named === "string" && named !== "" ? named : null;
  const activity = branch === null ? null : await branchActivity(ctx, path, branch, startMs);

  if (activity === null || branch === null) {
    // The activity API is unavailable: after the grace hour, the flagged fallback.
    if (!graceOver) return null;
    console.error(`final commit ${run.run_id}: the activity API is unavailable; using the commit list (head_fallback)`);
    return fromFallback(run, await fallbackCommit(ctx, path, branch, ends), graceOver, { ...NO_FINAL_DETAIL, branch });
  }

  const { items, complete } = activity;
  if (!complete) console.error(`final commit ${run.run_id}: more than ${ACTIVITY_PAGES} activity pages; force_push covers only the newest`);
  const late = items.filter((a) => a.at > endsMs);
  const flags = {
    branch,
    late_push: late.length > 0,
    force_push: items.some((a) => a.force && a.at <= endsMs && a.at >= startMs),
  };
  const last = items.find((a) => a.at <= endsMs);
  if (last !== undefined) {
    const commit = last.after === ZERO_SHA ? "none" : last.after; // the branch was deleted at the deadline
    return { commit, detail: { ...flags, pushed_at: formatTime(new Date(last.at)), head_fallback: false } };
  }
  const oldestLate = late.at(-1);
  if (oldestLate !== undefined && complete) {
    // No update at or before the deadline was listed: the branch stood where the first later push found it.
    const before = oldestLate.before;
    return { commit: before === ZERO_SHA ? "none" : before, detail: { ...flags, pushed_at: null, head_fallback: false } };
  }
  if (oldestLate !== undefined) {
    // Every update read is after the deadline and the page limit stopped the read, so the update
    // that set the branch at the deadline was not read. Ask again until the grace hour ends, then fall back.
    if (!graceOver) return null;
    console.error(`final commit ${run.run_id}: ${ACTIVITY_PAGES} activity pages, all after the end; using the commit list (head_fallback)`);
  }
  // No activity on the branch (it has not moved since before the run), or the case above: flagged.
  return fromFallback(run, await fallbackCommit(ctx, path, branch, ends), graceOver, { ...flags, pushed_at: null, head_fallback: true });
}
