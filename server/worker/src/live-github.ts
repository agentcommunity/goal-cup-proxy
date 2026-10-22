// The GitHub reader for the live totals (docs/UPLOAD-API.md, `GET /v1/live.json`, the `github`
// block). Step (c) of the live stats design (goal-cup-proxy PR #241). Each cron tick, after the live
// snapshot, it reads the public repositories of a few competition runs of the current event and
// stores numbers only (`Db.recordGithub`); the next snapshot sums them. Off unless GITHUB_STATS=on.
//
// Per run, three cheap calls, with the headers and timeout of every GitHub call (src/github.ts):
//   commits   GET /repos/{o}/{r}/commits?since=<start>&per_page=1, the count read from the Link
//             header's last page. During the run: the default branch. After the end: the judged
//             commit (`sha=<final_commit>`), so pushes GitHub timestamped after the end are left out.
//   PRs       GET /search/issues?q=repo:{o}/{r} is:pr author:{o} created:<start>..<end>, total_count:
//             pull requests the repository's owner opened during the run, by GitHub's timestamp
//             (anyone can open one on a public repository; outsiders' never count).
//   lines     GET /repos/{o}/{r}/stats/code_frequency, additions and deletions of the weeks the run
//             touches. 202 means GitHub is still computing: asked again next tick.
//
// A run is read about hourly while it runs. From GITHUB_SETTLE after its end it gets one last
// reading (the judged commit decided, every number read), retried each tick for at most
// GITHUB_GRACE, and then is never read again. A number not read this time is sent as null and the
// stored one stays, so a repository that turns private or disappears keeps its last numbers.
//
// Bounded and polite: at most GITHUB_STATS_PER_TICK runs a tick, oldest due first. A 429, a 403 that
// is a rate limit, or a GitHub that cannot be reached ends the step for this tick (the run being read
// is not stored, and stays first in line). It also ends once GitHub says fewer than GITHUB_RESERVE
// core calls are left this hour: the judged commits and the public-repo check on every start need
// them more. On its own (D64): a failure is one log line (`github-stats-failed`), and no other step
// waits on it or sees it.

import type { Ctx } from "./context.ts";
import type { GithubDue, GithubReading } from "./db.ts";
import { GITHUB_GRACE, GITHUB_SETTLE, type GithubAnswer, githubGet, prop } from "./github.ts";
import { REPO_URL, addSeconds, formatTime } from "./util.ts";

/** The most runs one tick reads. 3 calls each: 60 calls a tick, 720 an hour, of the token's 5,000. */
export const GITHUB_STATS_PER_TICK = 20;
/** A run is read again this long after its last reading while it runs. */
export const GITHUB_STATS_EVERY = 3600;
/** ... or this soon (the next tick) when GitHub was still computing, or the last reading is not complete. */
export const GITHUB_STATS_RETRY = 300;
/** Stop for the tick when GitHub says fewer core calls than this are left. */
export const GITHUB_RESERVE = 1000;

const API = "https://api.github.com";
const WEEK_MS = 7 * 24 * 3600 * 1000;

/** Why the step stops for this tick, from one answer; null to go on. */
function stopFor(res: GithubAnswer): string | null {
  if (res === null) return "GitHub could not be reached";
  if (res.status === 429) return "GitHub answered 429";
  // A 403 is a rate limit when GitHub says so; otherwise it is this repository's (blocked, say).
  if (res.status === 403 && (res.remaining === 0 || res.retryAfter)) return "GitHub answered 403, rate limited";
  return null;
}

/** GitHub's time format for a search qualifier (`Z` spelled as an offset). */
const searchTime = (t: string) => t.replace(/Z$/, "+00:00");

const wholeCount = (v: unknown): number | null => (typeof v === "number" && Number.isSafeInteger(v) && v >= 0 ? v : null);

type Reading = Omit<GithubReading, "final">;

/**
 * Read the three numbers of one repository. `stop`: end the step, store nothing for this run.
 * `computing`: GitHub had not finished the line counts. `low`: the core limit is at the reserve.
 */
async function readRepo(ctx: Ctx, run: GithubDue, ref: string | null, ends: string | null): Promise<{ stop: string } | { reading: Reading; computing: boolean; low: boolean }> {
  const path = run.repo_url.slice("https://github.com/".length);
  const owner = path.split("/")[0]!;
  const since = encodeURIComponent(run.official_start);
  let low = false;
  const ask = async (url: string): Promise<{ stop: string } | { res: NonNullable<GithubAnswer> }> => {
    const res = await githubGet(ctx, url);
    const stop = stopFor(res);
    if (stop !== null || res === null) return { stop: stop ?? "GitHub could not be reached" };
    if (res.resource !== "search" && res.remaining !== null && res.remaining < GITHUB_RESERVE) low = true;
    return { res };
  };

  const c = await ask(`${API}/repos/${path}/commits?${ref === null ? "" : `sha=${ref}&`}since=${since}&per_page=1`);
  if ("stop" in c) return c;
  let commits: number | null = null;
  if (c.res.status === 200 && Array.isArray(c.res.body)) commits = c.res.lastPage ?? c.res.body.length;
  else if (c.res.status === 409) commits = 0; // an empty repository

  const q = `repo:${path} is:pr author:${owner} created:${searchTime(run.official_start)}..${ends === null ? "*" : searchTime(ends)}`;
  const p = await ask(`${API}/search/issues?q=${encodeURIComponent(q)}&per_page=1`);
  if ("stop" in p) return p;
  const complete = prop(p.res.body, "incomplete_results") === false;
  const prs = p.res.status === 200 && complete ? wholeCount(prop(p.res.body, "total_count")) : null;

  const l = await ask(`${API}/repos/${path}/stats/code_frequency`);
  if ("stop" in l) return l;
  let added: number | null = null;
  let removed: number | null = null;
  if (l.res.status === 204) [added, removed] = [0, 0]; // nothing committed
  if (l.res.status === 200 && Array.isArray(l.res.body)) {
    // [week start in seconds, additions, deletions (negative)]; the weeks the run touches.
    const startMs = Date.parse(run.official_start);
    const weeks = (l.res.body as unknown[]).filter((w): w is [number, number, number] => Array.isArray(w) && w.length === 3 && w.every((x) => Number.isSafeInteger(x)));
    if (weeks.length === l.res.body.length) {
      const touched = weeks.filter(([week]) => week * 1000 + WEEK_MS > startMs);
      added = touched.reduce((sum, [, a]) => sum + Math.max(0, a), 0);
      removed = touched.reduce((sum, [, , d]) => sum + Math.max(0, -d), 0);
    }
  }
  return { reading: { commits, prs_opened: prs, lines_added: added, lines_removed: removed }, computing: l.res.status === 202, low };
}

const NOTHING: Reading = { commits: null, prs_opened: null, lines_added: null, lines_removed: null };

/** What to store for one due run, and when it is due again; `stop` ends the step. */
async function readRun(ctx: Ctx, run: GithubDue, now: Date): Promise<{ stop: string } | { reading: GithubReading; nextAt: string | null; low: boolean }> {
  const at = (seconds: number) => formatTime(addSeconds(now, seconds));
  const settled = addSeconds(new Date(run.ends_at), GITHUB_SETTLE);
  const graceOver = now.getTime() >= addSeconds(new Date(run.ends_at), GITHUB_GRACE).getTime();
  if (!REPO_URL.test(run.repo_url)) return { reading: { ...NOTHING, final: true }, nextAt: null, low: false }; // cannot happen: checked at the open

  if (now.getTime() < settled.getTime()) {
    // During the run: everything so far. Again in an hour, or at the end, whichever is first.
    const got = await readRepo(ctx, run, null, null);
    if ("stop" in got) return got;
    const next = got.computing ? at(GITHUB_STATS_RETRY) : formatTime(new Date(Math.min(addSeconds(now, GITHUB_STATS_EVERY).getTime(), settled.getTime())));
    return { reading: { ...got.reading, final: false }, nextAt: next, low: got.low };
  }

  // The last reading counts up to the judged commit, which the cron decides after the end.
  if (run.final_commit === null && !graceOver) return { reading: { ...NOTHING, final: false }, nextAt: at(GITHUB_STATS_RETRY), low: false };
  if (run.final_commit === null || run.final_commit === "none") return { reading: { ...NOTHING, final: true }, nextAt: null, low: false }; // keeps its last numbers
  const got = await readRepo(ctx, run, run.final_commit, run.ends_at);
  if ("stop" in got) return got;
  const r = got.reading;
  const complete = r.commits !== null && r.prs_opened !== null && r.lines_added !== null && r.lines_removed !== null;
  if (complete || graceOver) return { reading: { ...r, final: true }, nextAt: null, low: got.low };
  return { reading: { ...r, final: false }, nextAt: at(GITHUB_STATS_RETRY), low: got.low };
}

/**
 * The cron's step: read the due runs' repositories and store their numbers. Returns how many runs
 * got a row stored. Never throws.
 */
export async function readGithubStats(ctx: Ctx, now: Date): Promise<number> {
  if (ctx.env.GITHUB_STATS !== "on") return 0;
  try {
    if (!ctx.env.GITHUB_TOKEN && ctx.env.DB_BACKEND !== "local") {
      // Without a token every call shares GitHub's small anonymous limit with the judged commits.
      console.error("github-stats-failed: GITHUB_TOKEN is not set; the GitHub totals are not read");
      return 0;
    }
    const at = formatTime(now);
    let stored = 0;
    for (const run of await ctx.db.githubDue(ctx.cfg.eventId, at, GITHUB_STATS_PER_TICK)) {
      const got = await readRun(ctx, run, now);
      if ("stop" in got) {
        console.warn(`github-stats: stopped for this tick: ${got.stop}`);
        break;
      }
      if (await ctx.db.recordGithub(run.run_id, got.reading, at, got.nextAt)) stored++;
      if (got.low) {
        console.warn(`github-stats: stopped for this tick: fewer than ${GITHUB_RESERVE} GitHub calls left this hour`);
        break;
      }
    }
    return stored;
  } catch (e) {
    console.error(`github-stats-failed: ${e instanceof Error ? e.message : String(e)}`);
    return 0;
  }
}
