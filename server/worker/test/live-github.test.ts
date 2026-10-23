// The live totals' GitHub reader (src/live-github.ts; docs/UPLOAD-API.md, the `github` block of
// `GET /v1/live.json`): each cron tick reads a few competition runs' public repositories and stores
// numbers only; the next snapshot sums them. The Worker runs in this process on a clock the test
// moves (test/helpers/inproc.ts) and GitHub is a stubbed `fetch`: no network.

import assert from "node:assert/strict";
import { afterEach, mock } from "node:test";
import { LocalDb } from "../src/db-local.ts";
import type { LiveSnapshot } from "../src/db.ts";
import { GITHUB_GRACE, GITHUB_SETTLE } from "../src/github.ts";
import { GITHUB_RESERVE, GITHUB_STATS_PER_TICK } from "../src/live-github.ts";
import { liveKey } from "../src/live.ts";
import { credentialFor } from "./helpers/client.ts";
import { T0, WINDOW, type Opened, type Rig, lifecycle } from "./helpers/rig.ts";

const ON = { ...WINDOW, GITHUB_STATS: "on" };
/** Runs of one hour, so the end is a short step away. */
const HOUR = { ...ON, RUN_SECONDS: "3600" };
const MIN = 60;
const SHA = "a1".repeat(20);
const LATER = "b2".repeat(20);

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

/** One public repository as the stub serves it. */
interface Repo {
  /** Commits on the default branch since the run's start, and reachable from a given sha. */
  commits: number;
  commitsAt?: Record<string, number>;
  prs: number;
  /** code_frequency weeks: [week start in seconds, additions, deletions (negative)]. */
  weeks: [number, number, number][];
  /** Answer 202 to code_frequency this many times first. */
  computing?: number;
}

/** Week start (Sunday 00:00 UTC) of the run's start, and of the week before. */
const WEEK = Date.parse("2026-11-01T00:00:00Z") / 1000;
const BEFORE = WEEK - 7 * 24 * 3600;

/**
 * GitHub with these repositories (by `owner/name`). `fail(url)` may answer a call instead (a status,
 * with headers) or throw. Repository info and activity are there for the judged commit: the last
 * push before the end is SHA, and LATER came after it.
 */
function github(repos: Record<string, Repo>, fail: (url: URL) => Response | Error | undefined = () => undefined) {
  const calls: URL[] = [];
  globalThis.fetch = (async (input: string | URL | Request) => {
    const u = new URL(String(input));
    calls.push(u);
    const f = fail(u);
    if (f instanceof Error) throw f;
    if (f !== undefined) return f;
    if (u.pathname === "/search/issues") {
      const path = /repo:(\S+)/.exec(u.searchParams.get("q") ?? "")![1]!;
      return json({ total_count: repos[path]?.prs ?? 0, incomplete_results: false, items: [] });
    }
    const [, owner, name, ...rest] = u.pathname.slice("/repos".length).split("/");
    const repo = repos[`${owner}/${name}`];
    if (repo === undefined) return json({ message: "Not Found" }, 404);
    const tail = rest.join("/");
    if (tail === "") return json({ default_branch: "main", private: false });
    if (tail === "activity") {
      return json([
        { before: SHA, after: LATER, ref: "refs/heads/main", timestamp: t(62 * MIN), activity_type: "push" },
        { before: "0".repeat(40), after: SHA, ref: "refs/heads/main", timestamp: t(30 * MIN), activity_type: "push" },
      ]);
    }
    if (tail === "commits") {
      const sha = u.searchParams.get("sha");
      const n = sha === null ? repo.commits : (repo.commitsAt?.[sha] ?? 0);
      const link: Record<string, string> = n > 1 ? { link: `<https://api.github.com/repositories/1/commits?per_page=1&page=2>; rel="next", <https://api.github.com/repositories/1/commits?per_page=1&page=${n}>; rel="last"` } : {};
      return json(n === 0 ? [] : [{ sha: SHA }], 200, link);
    }
    if (tail === "stats/code_frequency") {
      if ((repo.computing ?? 0) > 0) {
        repo.computing!--;
        return json({}, 202);
      }
      return json(repo.weeks);
    }
    return json({ message: "Not Found" }, 404);
  }) as typeof fetch;
  return calls;
}
const json = (body: unknown, status = 200, headers: Record<string, string> = {}) => new Response(JSON.stringify(body), { status, headers });
const t = (seconds: number) => new Date(Date.parse(T0) + seconds * 1000).toISOString().replace(".000Z", "Z");

/** `n` members each start a competition run on their own repository `owner{i}/entry`; in run id order, which is the reader's order for runs due at once. */
async function runs(r: Rig, n: number, owner = "owner"): Promise<(Opened & { path: string })[]> {
  const out: (Opened & { path: string })[] = [];
  for (let i = 0; i < n; i++) {
    const token = await credentialFor(r.api, `${owner}-member-${i}`);
    const path = `${owner}${i}/entry`;
    const res = await r.open(token, `${owner}-s${i}`, { repo_url: `https://github.com/${path}` });
    assert.equal(res.status, 201, JSON.stringify(res.body));
    out.push({ ...(res.body as Opened), path });
  }
  return out.sort((x, y) => x.run_id.localeCompare(y.run_id));
}
/** Every repository the same: 12 commits, 3 PRs, 250 lines added and 40 removed this week (and 9999 the week before, not counted). */
const same = (all: { path: string }[], extra: Partial<Repo> = {}): Record<string, Repo> =>
  Object.fromEntries(all.map((x) => [x.path, { commits: 12, prs: 3, weeks: [[BEFORE, 9999, -9999], [WEEK, 250, -40]], ...extra }]));

/** The repositories whose commits were asked for. */
const readRepos = (calls: URL[]) => calls.filter((u) => u.pathname.endsWith("/commits")).map((u) => u.pathname.split("/").slice(2, 4).join("/"));
const stored = (r: Rig, run: string) => JSON.parse(r.bucket.map.get(`state/github/${run}.json`)?.text ?? "null") as Record<string, unknown> | null;
const live = async (r: Rig) => (await r.api.call("GET", "/v1/live.json")).body as LiveSnapshot;

lifecycle("off unless GITHUB_STATS=on: GitHub is never asked", async (r) => {
  const all = await runs(r, 5);
  const calls = github(same(all));
  await r.tick();
  assert.equal(calls.length, 0);
});

lifecycle("each tick reads at most 20 runs, oldest due first; each is read again an hour later", async (r) => {
  const all = await runs(r, GITHUB_STATS_PER_TICK + 5);
  let calls = github(same(all));
  await r.tick();
  const first = readRepos(calls);
  assert.equal(first.length, GITHUB_STATS_PER_TICK);
  assert.equal(calls.length, 3 * GITHUB_STATS_PER_TICK, "three calls a run");
  r.after(5 * MIN);
  calls = github(same(all));
  await r.tick();
  const second = readRepos(calls);
  assert.equal(second.length, 5);
  assert.deepEqual([...first, ...second].sort(), all.map((x) => x.path).sort(), "every run once");
  r.after(5 * MIN);
  calls = github(same(all));
  await r.tick();
  assert.equal(calls.length, 0, "nothing is due within the hour");
  r.after(55 * MIN);
  calls = github(same(all));
  await r.tick();
  assert.deepEqual(readRepos(calls).sort(), [...first].sort(), "an hour after their reading, the first 20 again");
}, ON);

lifecycle("lines GitHub is still computing (202) are asked again next tick; commits and PRs are kept", async (r) => {
  const all = await runs(r, 5);
  const repos = same(all, { computing: 1 });
  github(repos);
  await r.tick(); // reads: lines 202
  assert.deepEqual(stored(r, all[0]!.run_id), { commits: 12, prs_opened: 3, lines_added: null, lines_removed: null, read_at: T0, due_at: t(5 * MIN), final: false });
  r.after(5 * MIN);
  await r.tick(); // the snapshot sums what is stored; then the lines are read
  let doc = await live(r);
  assert.deepEqual(doc.github, { as_of: T0, repos: 5, commits: 60, prs_opened: 15, lines_added: null, lines_removed: null });
  r.after(5 * MIN);
  await r.tick();
  doc = await live(r);
  // 5 x 250 added, 5 x 40 removed (floored to 100); the week before the run is not counted.
  assert.deepEqual(doc.github, { as_of: t(5 * MIN), repos: 5, commits: 60, prs_opened: 15, lines_added: 1200, lines_removed: 200 });
}, ON);

lifecycle("the github block is null under 5 repositories; two runs on one repository count it once", async (r) => {
  const four = await runs(r, 4);
  github(same(four));
  await r.tick();
  r.after(5 * MIN);
  await r.tick();
  assert.equal((await live(r)).github, null);

  // A fifth run, on the first run's repository: still 4 repositories.
  const token = await credentialFor(r.api, "twin");
  assert.equal((await r.open(token, "twin-s", { repo_url: `https://github.com/${four[0]!.path}` })).status, 201);
  r.after(5 * MIN);
  await r.tick();
  r.after(5 * MIN);
  await r.tick();
  assert.equal((await live(r)).github, null);

  const fifth = await runs(r, 1, "late");
  github({ ...same(four), ...same(fifth) });
  r.after(5 * MIN);
  await r.tick();
  r.after(5 * MIN);
  await r.tick();
  assert.equal((await live(r)).github?.repos, 5);
  assert.equal((await live(r)).github?.commits, 60);
}, ON);

lifecycle("a 429 or a rate-limit 403 ends the step for the tick, and that run is read first next tick; another 403 does not", async (r) => {
  const all = await runs(r, 3);
  const limited = (u: URL, headers: Record<string, string>, status: number) => (u.pathname.includes(all[1]!.path) ? json({ message: "limited" }, status, headers) : undefined);
  for (const [status, headers] of [[429, {}], [403, { "x-ratelimit-remaining": "0" }], [403, { "retry-after": "60" }]] as const) {
    let calls = github(same(all), (u) => limited(u, headers, status));
    await r.tick();
    assert.equal(readRepos(calls).length, 2, `${status}: the third run is not asked`);
    assert.equal(stored(r, all[1]!.run_id), null, `${status}: the limited run is not stored`);
    r.after(5 * MIN);
    calls = github(same(all), (u) => limited(u, headers, status));
    await r.tick();
    assert.deepEqual(readRepos(calls), [all[1]!.path], `${status}: the limited run is still first in line`);
    for (const k of [...r.bucket.map.keys()].filter((key) => key.startsWith("state/github/"))) r.bucket.map.delete(k);
    r.after(5 * MIN);
  }

  // Search answers its secondary limit with a bare 403: that ends the step too.
  let bare = github(same(all), (u) => (u.pathname === "/search/issues" ? json({ message: "secondary rate limit" }, 403) : undefined));
  await r.tick();
  assert.equal(readRepos(bare).length, 1);
  assert.equal(stored(r, all[0]!.run_id), null);
  r.after(5 * MIN);

  // A 403 that is not a rate limit (a blocked repository, say) is that repository's: the others are read.
  bare = github(same(all), (u) => (u.pathname.includes(all[0]!.path) ? json({ message: "blocked" }, 403) : undefined));
  await r.tick();
  assert.equal(readRepos(bare).length, 3);
  assert.equal(stored(r, all[0]!.run_id)?.commits, null);
  assert.equal(stored(r, all[2]!.run_id)?.commits, 12);
}, ON);

lifecycle("the step stops once GitHub says fewer than the reserve of calls is left, after storing that run", async (r) => {
  const all = await runs(r, 3);
  const calls = github(same(all), (u) =>
    u.pathname.endsWith("/commits") ? json([{ sha: SHA }], 200, { "x-ratelimit-remaining": String(GITHUB_RESERVE - 1), "x-ratelimit-resource": "core" }) : undefined,
  );
  await r.tick();
  assert.equal(readRepos(calls).length, 1);
  assert.equal(stored(r, all[0]!.run_id)?.commits, 1);
}, ON);

lifecycle("after the end: one last reading up to the judged commit, then never again", async (r) => {
  const [run] = await runs(r, 1);
  // The week after the end (pushes after the deadline) is left out of the last reading.
  const repos = same([run!], { commits: 9, commitsAt: { [SHA]: 7 }, prs: 2, weeks: [[BEFORE, 9999, -9999], [WEEK, 250, -40], [WEEK + 7 * 24 * 3600, 5000, -5000]] });
  let calls = github(repos);
  await r.tick(); // during the run
  const during = calls.find((u) => u.pathname.endsWith("/commits"))!;
  assert.equal(during.searchParams.get("sha"), null);
  assert.equal(during.searchParams.get("since"), T0);
  assert.equal(stored(r, run!.run_id)?.due_at, t(3600), "next at the end, within the hour");

  r.at(t(3600 + GITHUB_SETTLE)); // the judged commit is decided in this tick, then the last reading
  calls = github(repos);
  await r.tick();
  const last = calls.find((u) => u.pathname.endsWith("/commits") && u.searchParams.has("since"))!;
  assert.equal(last.searchParams.get("sha"), SHA, "counted up to the judged commit: LATER, pushed after the end, is not");
  const search = calls.find((u) => u.pathname === "/search/issues")!.searchParams.get("q")!;
  assert.equal(search, `repo:${run!.path} is:pr author:owner0 created:${T0.replace("Z", "+00:00")}..${t(3600).replace("Z", "+00:00")}`);
  assert.deepEqual(stored(r, run!.run_id), { commits: 7, prs_opened: 2, lines_added: 250, lines_removed: 40, read_at: t(3600 + GITHUB_SETTLE), due_at: null, final: true });

  for (const hours of [1, 24, 72]) {
    r.at(t(3600 + hours * 3600));
    calls = github(repos);
    await r.tick();
    assert.deepEqual(calls.filter((u) => u.pathname.endsWith("/commits") || u.pathname.includes("/stats/") || u.pathname === "/search/issues"), [], `${hours} h later`);
  }
}, HOUR);

lifecycle("the GitHub numbers are community totals only: no repository, owner, run or sha is stored or published", async (r) => {
  const all = await runs(r, 6, "secret-owner");
  github(same(all));
  await r.tick();
  r.after(5 * MIN);
  await r.tick();
  const res = await r.api.call("GET", "/v1/live.json");
  assert.deepEqual(Object.keys(res.body.github).sort(), ["as_of", "commits", "lines_added", "lines_removed", "prs_opened", "repos"]);
  const text = JSON.stringify(res.body);
  for (const secret of ["secret-owner", "entry", "github.com", SHA, ...all.map((x) => x.run_id)]) assert.ok(!text.includes(secret), `${secret} in ${text}`);
  for (const x of all) {
    assert.deepEqual(Object.keys(stored(r, x.run_id)!).sort(), ["commits", "due_at", "final", "lines_added", "lines_removed", "prs_opened", "read_at"]);
  }
}, ON);

lifecycle("a failing GitHub reader changes nothing else: the tick finishes, notices go out, the snapshot is written (D64)", async (r) => {
  const all = await runs(r, 5);
  const logged: string[] = [];
  const log = mock.method(console, "error", (line: string) => void logged.push(String(line)));
  // The database fails under the step twice: before it asks GitHub, and while it stores a reading.
  const due = mock.method(LocalDb.prototype, "githubDue", () => Promise.reject(new Error("database down")));
  try {
    github({}, () => new Error("GitHub must not be asked"));
    await r.tick();
    assert.ok(logged.some((l) => l.startsWith("github-stats-failed: database down")), logged.join("\n"));
    assert.equal([...r.bucket.map.keys()].filter((k) => k.startsWith("state/notices/")).length, 5, "every started notice");
    assert.equal((await live(r)).entries.started, 5);
  } finally {
    due.mock.restore();
  }
  const write = mock.method(LocalDb.prototype, "recordGithub", () => Promise.reject(new Error("write refused")));
  try {
    const before = r.bucket.map.get(liveKey("cup"))!.text;
    const calls = github(same(all));
    r.after(5 * MIN);
    await r.tick(); // resolves: the step's failure is not the tick's
    assert.equal(readRepos(calls).length, 1, "the step ends at the failure");
    assert.ok(logged.some((l) => l.startsWith("github-stats-failed: write refused")), logged.join("\n"));
    assert.notEqual(r.bucket.map.get(liveKey("cup"))!.text, before, "the snapshot of this tick was written");
  } finally {
    write.mock.restore();
    log.mock.restore();
  }
}, ON);

lifecycle("when the tick's first call cannot reach GitHub the step ends; that run goes to the back of the line", async (r) => {
  const all = await runs(r, 3);
  const warned: string[] = [];
  const warn = mock.method(console, "warn", (line: string) => void warned.push(String(line)));
  try {
    let calls = github({}, () => new Error("offline"));
    await r.tick();
    assert.equal(calls.length, 1, "one 8-second wait, not one per run");
    assert.ok(warned.some((l) => l.includes("stopped for this tick: GitHub could not be reached")), warned.join("\n"));
    assert.deepEqual(stored(r, all[0]!.run_id), { commits: null, prs_opened: null, lines_added: null, lines_removed: null, read_at: null, due_at: t(5 * MIN), final: false });
    r.after(5 * MIN);
    calls = github(same(all));
    await r.tick();
    assert.deepEqual(readRepos(calls), [all[1]!.path, all[2]!.path, all[0]!.path], "the runs due before it come first");
  } finally {
    warn.mock.restore();
  }
}, ON);

lifecycle("a run GitHub does not answer for on its 2nd or 3rd call is asked again behind the others; the rest are read", async (r) => {
  const all = await runs(r, GITHUB_STATS_PER_TICK + 2);
  const stuck = all[0]!;
  for (const second of [true, false]) {
    for (const k of [...r.bucket.map.keys()].filter((key) => key.startsWith("state/github/"))) r.bucket.map.delete(k);
    const hangs = (u: URL) =>
      (second ? u.pathname === "/search/issues" && u.searchParams.get("q")!.includes(stuck.path) : u.pathname === `/repos/${stuck.path}/stats/code_frequency`)
        ? new Error("timed out")
        : undefined;
    let calls = github(same(all), hangs);
    await r.tick();
    assert.equal(readRepos(calls).length, GITHUB_STATS_PER_TICK, "every run of the tick was asked");
    assert.equal(stored(r, all[1]!.run_id)?.commits, 12);
    assert.deepEqual(stored(r, stuck.run_id), { commits: null, prs_opened: null, lines_added: null, lines_removed: null, read_at: null, due_at: later(5 * MIN), final: false }, "nothing new");
    r.after(5 * MIN);
    calls = github(same(all), hangs);
    await r.tick();
    assert.deepEqual(readRepos(calls), [all[20]!.path, all[21]!.path, stuck.path], "the two never read come first");
    r.after(5 * MIN);
  }
}, ON);

lifecycle("after the end a judged commit of none, or none decided by the grace hour, makes the run final with its last numbers", async (r) => {
  const [gone, unknown] = await runs(r, 2);
  github(same([gone!, unknown!]));
  await r.tick(); // the readings during the run
  // From the end: GitHub has no repository `gone` (the judged commit becomes none after the grace
  // hour), and cannot be reached for `unknown`'s repository (no judged commit at all).
  const after = (u: URL) => (u.pathname === `/repos/${gone!.path}` ? json({}, 404) : u.pathname === `/repos/${unknown!.path}` ? new Error("offline") : undefined);
  r.at(t(3600 + GITHUB_SETTLE));
  let calls = github(same([gone!, unknown!]), after);
  await r.tick();
  assert.deepEqual(readRepos(calls), [], "no last reading before a judged commit");
  assert.equal(stored(r, gone!.run_id)?.final, false);
  r.at(t(3600 + GITHUB_GRACE));
  calls = github(same([gone!, unknown!]), after);
  await r.tick();
  assert.deepEqual(readRepos(calls), []);
  for (const run of [gone!, unknown!]) {
    assert.deepEqual(stored(r, run.run_id), { commits: 12, prs_opened: 3, lines_added: 250, lines_removed: 40, read_at: T0, due_at: null, final: true });
  }
}, HOUR);

lifecycle("a last reading whose lines GitHub is still computing is retried until the grace hour, then final without them", async (r) => {
  const [run] = await runs(r, 1);
  const repos = same([run!], { commits: 9, commitsAt: { [SHA]: 7 }, prs: 2, computing: 1000 });
  github(repos);
  r.at(t(3600 + GITHUB_SETTLE));
  await r.tick();
  assert.equal(stored(r, run!.run_id)?.final, false);
  assert.equal(stored(r, run!.run_id)?.due_at, t(3600 + GITHUB_SETTLE + 5 * MIN));
  r.at(t(3600 + GITHUB_GRACE));
  await r.tick();
  assert.deepEqual(stored(r, run!.run_id), { commits: 7, prs_opened: 2, lines_added: null, lines_removed: null, read_at: t(3600 + GITHUB_GRACE), due_at: null, final: true });
  r.after(5 * MIN);
  const calls = github(repos);
  await r.tick();
  assert.deepEqual(readRepos(calls), [], "never again");
}, HOUR);

lifecycle("an empty repository (commits 409, code_frequency 204) counts 0", async (r) => {
  const [run] = await runs(r, 1);
  github(same([run!]), (u) => (u.pathname.endsWith("/commits") ? json({ message: "Git Repository is empty." }, 409) : u.pathname.endsWith("/code_frequency") ? new Response(null, { status: 204 }) : undefined));
  await r.tick();
  assert.deepEqual(stored(r, run!.run_id), { commits: 0, prs_opened: 3, lines_added: 0, lines_removed: 0, read_at: T0, due_at: t(3600), final: false });
}, ON);

/** The rig's clock (it moves `Date.now`) plus `seconds`, as the reader writes times. */
function later(seconds: number): string {
  return new Date(Date.now() + seconds * 1000).toISOString().replace(".000Z", "Z");
}
