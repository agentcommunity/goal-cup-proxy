// The GitHub API, read by the cron to find each run's judged commit (owner, 21 Oct 2026: the last
// push to the default branch that GitHub timestamped at or before `ends_at`), and the headers every
// GitHub call carries (also the public-repo check in src/runs.ts).

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
export async function judgedCommit(ctx: Ctx, run: TickRun, now: Date): Promise<Judged | null> {
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
