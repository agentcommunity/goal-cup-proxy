// What the server does on its own (docs/UPLOAD-API.md, "What the server does on its own"): the
// 5-minute cron closes runs that are over, records the GitHub commit at the deadline, and queues
// the run emails into the outbox. The Worker runs in this process on a clock the test moves
// (test/helpers/inproc.ts), and `tick` runs the cron once at that time.

import assert from "node:assert/strict";
import { afterEach, mock, test } from "node:test";
import { LocalDb } from "../src/db-local.ts";
import type { Ctx } from "../src/context.ts";
import type { TickRun } from "../src/db.ts";
import { GITHUB_GRACE, GITHUB_SETTLE, queueNotices, recordFinalCommits, runTick } from "../src/notices.ts";
import { credentialFor } from "./helpers/client.ts";
import { T0, WINDOW, type Rig, lifecycle } from "./helpers/rig.ts";

const REPO = "https://github.com/someone/goal-cup-entry"; // what openBody sends
const SHA = "a1".repeat(20);
const ZERO = "0".repeat(40);
const API = "https://api.github.com/repos/someone/goal-cup-entry";
/** A competition run of one hour, so the end is a short step away. */
const HOUR = { ...WINDOW, RUN_SECONDS: "3600" };

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

/** Stand in for the GitHub API: `reply` answers each request (its number, its URL); the calls are recorded. */
function stubGithub(reply: (n: number, url: string) => Response | Error) {
  const calls: { url: string; headers: Record<string, string> }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(input), headers: (init?.headers ?? {}) as Record<string, string> });
    const r = reply(calls.length, String(input));
    if (r instanceof Error) throw r;
    return r;
  }) as typeof fetch;
  return calls;
}
const json = (body: unknown, status = 200, headers: Record<string, string> = {}) => new Response(JSON.stringify(body), { status, headers });

interface Push {
  after: string;
  at: string;
  type?: string;
  before?: string;
}
/**
 * One public repository on GitHub, default branch `main`: `pushes` is its activity, newest first, in
 * pages of `page` (Link rel="next" with an `after` cursor); its head is the newest push. A status in
 * `fail` replaces an endpoint's answer ("offline": the call throws).
 */
function repo(pushes: Push[], fail: { repo?: number; activity?: number | "offline"; head?: number } = {}, page = 100) {
  return (_n: number, url: string): Response | Error => {
    const u = new URL(url);
    if (u.pathname.endsWith("/activity")) {
      if (fail.activity === "offline") return new Error("offline");
      if (fail.activity !== undefined) return json({}, fail.activity);
      const from = Number(u.searchParams.get("after") ?? 0);
      const items = pushes.slice(from, from + page).map((p) => ({
        id: 1, node_id: "n", before: p.before ?? ZERO, after: p.after, ref: "refs/heads/main", timestamp: p.at, activity_type: p.type ?? "push", actor: null,
      }));
      const more = from + page < pushes.length;
      return json(items, 200, more ? { link: `<https://api.github.com/repositories/1/activity?after=${from + page}>; rel="next"` } : {});
    }
    if (u.pathname.endsWith("/commits/HEAD")) return fail.head !== undefined ? json({}, fail.head) : json({ sha: pushes[0]?.after ?? ZERO });
    return fail.repo !== undefined ? json({}, fail.repo) : json({ default_branch: "main", private: false });
  };
}
/** A repository whose last push, during the run, was `sha`. */
const pushed = (sha: string) => repo([{ after: sha, at: T0 }]);
const ASKED = [API, `${API}/activity?ref=refs%2Fheads%2Fmain&per_page=100`];

interface Notice {
  run_id: string;
  kind: string;
  dedupe_key: string;
  payload: Record<string, unknown>;
}
/** The outbox, as the website's mailer would read it: every notice written so far. */
const outbox = (r: Rig): Notice[] =>
  [...r.bucket.map.entries()].filter(([k]) => k.startsWith("state/notices/")).map(([, v]) => JSON.parse(v.text) as Notice);
const kinds = (r: Rig, run?: string) => outbox(r).filter((n) => run === undefined || n.run_id === run).map((n) => n.kind).sort();
const only = (r: Rig, kind: string) => outbox(r).filter((n) => n.kind === kind);
/** An instant `seconds` after the start of the day's runs. */
const t = (seconds: number) => new Date(Date.parse(T0) + seconds * 1000).toISOString().replace(".000Z", "Z");

const MIN = 60;
const HOURS = 3600;
/** The first time GitHub is asked about a run that ends at `deadline`. */
/** When a run that ends at `deadline` closes: 24 hours later. */
const closes = (deadline: string) => new Date(Date.parse(deadline) + 24 * HOURS * 1000).toISOString().replace(".000Z", "Z");
const settled = (deadline: string) => new Date(Date.parse(deadline) + GITHUB_SETTLE * 1000).toISOString().replace(".000Z", "Z");

lifecycle("opening a competition run queues one started notice", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  assert.deepEqual(kinds(r), []); // the cron decides, not the request
  await r.tick();
  assert.deepEqual(outbox(r), [{ run_id: run.run_id, member_id: "alice", kind: "started", dedupe_key: "", payload: { official_start: T0, deadline_at: run.deadline_at, repo_url: REPO }, created_at: T0, sent_at: null }]);
  r.after(5 * MIN);
  await r.tick();
  assert.deepEqual(kinds(r), ["started"]); // once
});

/**
 * Keep the server well up to `iso`: a tick every 5 minutes, and one at `iso`, each just after a
 * heartbeat of the companion run (`other`), so heartbeats are seen reaching the server. Without
 * `other` no run beats.
 */
async function steady(r: Rig, iso: string, other?: () => Promise<unknown>) {
  const target = Date.parse(iso);
  while (Date.now() + 5 * MIN * 1000 < target) {
    r.after(5 * MIN);
    await other?.();
    await r.tick();
  }
  r.at(iso);
  await other?.();
  await r.tick();
}
/** Another member's run that beats whenever the returned function is called. */
async function companion(r: Rig): Promise<() => Promise<unknown>> {
  const token = await r.cred("bob");
  const run = await r.start(token, "b1");
  return () => r.beat(token, run.run_id, run.entry_id);
}

lifecycle("20 minutes of silence queues one notice; a second silence another", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const other = await companion(r);
  assert.equal((await r.beat(token, run.run_id, run.entry_id)).status, 200); // 09:00:00
  await other();
  await r.tick(); // the server's well stretch starts at 09:00
  await steady(r, t(19 * MIN + 59), other);
  assert.deepEqual(kinds(r, run.run_id), ["started"]);

  await steady(r, t(20 * MIN + 1), other); // 20 minutes and 1 second since the last heartbeat
  assert.deepEqual(kinds(r, run.run_id), ["silence", "started"]);
  assert.deepEqual(only(r, "silence")[0]!.dedupe_key, T0);
  assert.deepEqual(only(r, "silence")[0]!.payload, { last_heartbeat_at: T0 });
  await steady(r, t(25 * MIN + 1), other);
  assert.equal(only(r, "silence").length, 1, "the same silence is mailed once");

  // The run comes back for a while, then goes quiet again: another silence, another notice.
  const back = t(35 * MIN + 1);
  await steady(r, back, other);
  await r.beat(token, run.run_id, run.entry_id);
  await steady(r, t(40 * MIN + 1), other);
  assert.equal(only(r, "silence").length, 1);
  await steady(r, t(55 * MIN + 2), other);
  assert.deepEqual(only(r, "silence").map((n) => n.dedupe_key), [T0, back]);
  assert.deepEqual(kinds(r).filter((k) => k === "silence").length, 2, "the companion, which beats, is never silent");
});

lifecycle("silence counts from the start when no heartbeat ever came", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  const other = await companion(r);
  await other();
  await r.tick();
  await steady(r, t(20 * MIN + 1), other);
  assert.deepEqual(only(r, "silence").map((n) => [n.run_id, n.dedupe_key, n.payload]), [[run.run_id, T0, { last_heartbeat_at: null }]]);
});

lifecycle("no silence email while the server missed its ticks; one when it has been well for 20 minutes", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const other = await companion(r);
  await r.beat(token, run.run_id, run.entry_id); // 09:00, and never again
  await other();
  await r.tick();
  await steady(r, t(10 * MIN), other);
  // No tick from 09:10 to 09:40 (the server's own trouble). The first tick after it is not well.
  r.at(t(40 * MIN));
  await other();
  await r.tick();
  assert.deepEqual(only(r, "silence"), [], "silent for 40 minutes, but the server was not well");
  await steady(r, t(60 * MIN), other);
  assert.deepEqual(only(r, "silence"), [], "the server has been well for exactly 20 minutes: not more");
  await steady(r, t(65 * MIN), other);
  assert.deepEqual(only(r, "silence").map((n) => [n.run_id, n.dedupe_key]), [[run.run_id, T0]]);
});

lifecycle("no silence email while no run's heartbeat reaches the server, though the cron ticks", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const other = await companion(r);
  await r.beat(token, run.run_id, run.entry_id);
  await other();
  await r.tick();
  await steady(r, t(60 * MIN)); // every 5 minutes, and no heartbeat from either run
  assert.deepEqual(only(r, "silence"), [], "a whole fleet gone quiet at once is our trouble, not theirs");
  // The companion's heartbeats get through again; alice's do not. Once the server has been well for
  // 20 minutes (since 10:00, its last tick with no heartbeat), alice is mailed, and only alice.
  await steady(r, t(80 * MIN), other);
  assert.deepEqual(only(r, "silence"), []);
  await steady(r, t(85 * MIN), other);
  assert.deepEqual(only(r, "silence").map((n) => [n.run_id, n.dedupe_key]), [[run.run_id, T0]]);
});

lifecycle("a run alone gets no silence email: the server cannot tell its silence from its own", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.beat(token, run.run_id, run.entry_id);
  await r.tick();
  await steady(r, t(2 * HOURS));
  assert.deepEqual(kinds(r), ["started"]);
});

lifecycle("the maintenance switch starts the server's well stretch again, even for one tick", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const other = await companion(r);
  await r.beat(token, run.run_id, run.entry_id);
  await other();
  await r.tick();
  await steady(r, t(10 * MIN), other);
  r.at(t(15 * MIN));
  await runTick({ ...r.env, MAINTENANCE: "on" });
  assert.equal(r.bucket.map.has("cron/health.json"), false);
  await steady(r, t(40 * MIN), other); // well again from 09:20
  assert.deepEqual(only(r, "silence"), []);
  await steady(r, t(45 * MIN), other);
  assert.deepEqual(only(r, "silence").map((n) => n.run_id), [run.run_id]);
});

lifecycle("a health marker that cannot be read counts as trouble: no silence email", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const other = await companion(r);
  await r.beat(token, run.run_id, run.entry_id);
  await other();
  await r.tick();
  await steady(r, t(25 * MIN), other);
  const silences = only(r, "silence").length;
  assert.equal(silences, 1);
  // A second silence, while the marker is garbage at every tick: never mailed.
  await r.beat(token, run.run_id, run.entry_id);
  const back = Date.now();
  while (Date.now() < back + 40 * MIN * 1000) {
    r.after(5 * MIN);
    r.bucket.map.set("cron/health.json", { text: "{not json", etag: "x" });
    await other();
    const log = mock.method(console, "error", () => {});
    try {
      await r.tick();
    } finally {
      log.mock.restore();
    }
  }
  assert.equal(only(r, "silence").length, silences);
});

lifecycle("three and six hours without model_requests growth queue one idle notice each", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const beat = (extra: Record<string, unknown>) => r.beat(token, run.run_id, run.entry_id, extra);
  r.after(MIN);
  await beat({ model_requests: 3 }); // the last model request: 09:01:00
  const since = t(MIN);

  // A heartbeat just before each tick, so the run is not silent; only the model calls are.
  r.at(t(3 * HOURS));
  await beat({ model_requests: 3 });
  await r.tick();
  assert.deepEqual(kinds(r), ["started"]); // exactly 3 h is not older than 3 h
  r.at(t(3 * HOURS + MIN + 1));
  await beat({ model_requests: 3 });
  await r.tick();
  assert.deepEqual(only(r, "idle_3h").map((n) => [n.dedupe_key, n.payload]), [[since, { hours: 3, since }]]);
  r.after(5 * MIN);
  await beat({ model_requests: 3 });
  await r.tick();
  assert.equal(only(r, "idle_3h").length, 1);
  assert.equal(only(r, "idle_6h").length, 0);

  r.at(t(6 * HOURS + MIN + 1));
  await beat({});
  await r.tick();
  assert.deepEqual(only(r, "idle_6h").map((n) => [n.dedupe_key, n.payload]), [[since, { hours: 6, since }]]);
  assert.equal(only(r, "idle_3h").length, 1);

  // A model call ends the idle period; three more hours without one is a new one.
  const resumed = 6 * HOURS + 6 * MIN + 1;
  const again = t(resumed);
  r.at(again);
  await beat({ model_requests: 4 });
  await r.tick();
  assert.equal(outbox(r).length, 3); // started, idle_3h, idle_6h, and nothing new
  r.at(t(resumed + 3 * HOURS + 1));
  await beat({ model_requests: 4 });
  await r.tick();
  assert.deepEqual(only(r, "idle_3h").map((n) => n.dedupe_key), [since, again]);
  assert.equal(only(r, "idle_6h").length, 1);
});

lifecycle("a silent run is silent, not idle", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const other = await companion(r);
  await r.beat(token, run.run_id, run.entry_id, { model_requests: 3 });
  await other();
  await r.tick();
  await steady(r, t(4 * HOURS), other); // no heartbeat for 4 hours, and no model call either
  assert.deepEqual(kinds(r, run.run_id), ["silence", "started"]);
});

lifecycle("a flagged run queues one recording notice", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  r.after(MIN);
  const flagged = t(MIN);
  await r.beat(token, run.run_id, run.entry_id, { recording: "gaps" });
  r.after(2 * MIN);
  await r.beat(token, run.run_id, run.entry_id, { recording: "failed" });
  await r.tick();
  assert.deepEqual(only(r, "recording").map((n) => [n.dedupe_key, n.payload]), [["", { flagged_at: flagged, recording: "gaps" }]]);
  r.after(5 * MIN);
  await r.beat(token, run.run_id, run.entry_id, { recording: "failed" });
  await r.tick();
  assert.equal(only(r, "recording").length, 1);
});

lifecycle("recording failed flags the run once", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const stored = () => {
    const s = r.storedRun(run.run_id);
    return [s.flagged_at, s.flag_reason];
  };
  const beat = (recording: string) => r.beat(token, run.run_id, run.entry_id, { recording });
  r.after(MIN);
  for (const fine of ["ok", "unknown"]) assert.equal((await beat(fine)).status, 200);
  assert.deepEqual(stored(), [null, null]);
  await beat("failed");
  assert.deepEqual(stored(), [t(MIN), "failed"]);
  r.after(MIN);
  for (const later of ["gaps", "failed", "ok"]) await beat(later);
  assert.deepEqual(stored(), [t(MIN), "failed"]); // the first flag stays
});

lifecycle("a heartbeat without the new fields is still 200 and changes nothing about them", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  r.after(MIN);
  const plain = await r.beat(token, run.run_id, run.entry_id);
  assert.equal(plain.status, 200, JSON.stringify(plain.body));
  assert.deepEqual(plain.body, { stop: false, deadline_at: run.deadline_at, seconds_left: 168 * 3600 - MIN, reason: null });
  const nulls = await r.beat(token, run.run_id, run.entry_id, { recording: null, model_requests: null });
  assert.equal(nulls.status, 200, JSON.stringify(nulls.body));
  const s = r.storedRun(run.run_id) as { entries: { model_requests: number | null }[] } & ReturnType<typeof r.storedRun>;
  assert.deepEqual([s.flagged_at, s.entries[0]!.model_requests, s.last_model_request_at, s.last_heartbeat_at], [null, null, null, t(MIN)]);
});

lifecycle("model_requests above the entry's stored value moves the last model request; equal or lower does not", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const seen = async (after: number, model_requests: number, entry = run.entry_id) => {
    r.at(t(after));
    await r.beat(token, run.run_id, entry, { model_requests });
    const s = r.storedRun(run.run_id) as { last_model_request_at: string; entries: { model_requests: number | null }[] };
    return [s.entries.map((e) => e.model_requests), s.last_model_request_at];
  };
  assert.deepEqual(await seen(60, 5), [[5], t(60)]);
  assert.deepEqual(await seen(120, 5), [[5], t(60)]);
  assert.deepEqual(await seen(180, 2), [[5], t(60)]); // lower, on the same entry
  assert.deepEqual(await seen(240, 6), [[6], t(240)]);
  assert.deepEqual(await seen(300, 0), [[6], t(240)]);
});

lifecycle("a continue counts from zero: its first report and its growth move the last model request", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  r.at(t(60));
  await r.beat(token, run.run_id, run.entry_id, { model_requests: 50 });
  r.at(t(2 * HOURS));
  const next = (await r.cont(token, run.run_id, run.entry_id, "s-two")).body.entry_id as string;
  const state = () => {
    const s = r.storedRun(run.run_id) as { last_model_request_at: string; entries: { model_requests: number | null }[] };
    return [s.entries.map((e) => e.model_requests), s.last_model_request_at];
  };
  assert.deepEqual(state(), [[50, null], t(60)]); // nothing reported by the new process yet
  await r.beat(token, run.run_id, next, { model_requests: 2 }); // lower than the run's old maximum
  assert.deepEqual(state(), [[50, 2], t(2 * HOURS)]);
  r.at(t(2 * HOURS + 10 * MIN));
  await r.beat(token, run.run_id, next, { model_requests: 3 });
  assert.deepEqual(state(), [[50, 3], t(2 * HOURS + 10 * MIN)]);
  r.at(t(2 * HOURS + 20 * MIN));
  await r.beat(token, run.run_id, run.entry_id, { model_requests: 50 }); // the old entry, no growth
  assert.deepEqual(state(), [[50, 3], t(2 * HOURS + 10 * MIN)]);
  r.at(t(2 * HOURS + 30 * MIN));
  await r.beat(token, run.run_id, run.entry_id, { model_requests: 51 }); // the old entry grows on its own count
  assert.deepEqual(state(), [[51, 3], t(2 * HOURS + 30 * MIN)]);
});

lifecycle("a run that never reported model_requests gets no idle notice", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  for (const hours of [3, 7]) {
    r.at(t(hours * HOURS + MIN));
    await r.beat(token, run.run_id, run.entry_id, { recording: "ok" });
    await r.tick();
  }
  assert.deepEqual(kinds(r), ["started"]);
  // Its first report starts the clock; three quiet hours after it are idle.
  r.at(t(8 * HOURS));
  await r.beat(token, run.run_id, run.entry_id, { model_requests: 0 });
  r.at(t(11 * HOURS + MIN));
  await r.beat(token, run.run_id, run.entry_id, { model_requests: 0 });
  await r.tick();
  assert.deepEqual(only(r, "idle_3h").map((n) => n.dedupe_key), [t(8 * HOURS)]);
});

lifecycle("the open keeps its IP and country; heartbeats and a continue keep the latest, and a missing header keeps the last", async (r) => {
  const token = await r.cred("alice");
  const opened = await r.open(token, "s1", {}, { "cf-connecting-ip": "203.0.113.9" });
  const run = opened.body as { run_id: string; entry_id: string };
  type Stored = { client_ip: string; client_country: string; last_client_ip: string; last_client_country: string; entries: { last_heartbeat: Record<string, unknown> }[] };
  const stored = () => r.storedRun(run.run_id) as unknown as Stored;
  assert.deepEqual([stored().client_ip, stored().client_country], ["203.0.113.9", "DE"]);

  const beat = await r.beat(token, run.run_id, run.entry_id, {}, { "cf-connecting-ip": "203.0.113.50" });
  assert.deepEqual(Object.keys(beat.body).sort(), ["deadline_at", "reason", "seconds_left", "stop"]);
  assert.deepEqual([stored().client_ip, stored().last_client_ip, stored().last_client_country], ["203.0.113.9", "203.0.113.50", "DE"]);
  const hb = stored().entries[0]!.last_heartbeat;
  assert.deepEqual([hb.client_ip, hb.client_country], ["203.0.113.50", "DE"]);

  // A heartbeat without the header says so on its own record, and leaves the run's latest pair.
  await r.beat(token, run.run_id, run.entry_id);
  assert.equal(stored().entries[0]!.last_heartbeat.client_ip, null);
  assert.deepEqual([stored().last_client_ip, stored().client_ip], ["203.0.113.50", "203.0.113.9"]);

  // A continue writes the latest pair too; the open's stays.
  const c = await r.cont(token, run.run_id, run.entry_id, "s-two", {}, { "cf-connecting-ip": "198.51.100.7" });
  assert.equal(c.status, 201, JSON.stringify(c.body));
  assert.deepEqual([stored().client_ip, stored().last_client_ip], ["203.0.113.9", "198.51.100.7"]);

  assert.ok(![...r.bucket.map.keys()].some((k) => k.includes("203.0.113")), "an address in a storage key");
}, undefined, undefined, { country: "DE" });

lifecycle("one run whose commit cannot be written does not stop the others", async (r) => {
  stubGithub(pushed(SHA));
  const [a, b] = [await r.cred("alice"), await r.cred("bob")];
  const first = await r.start(a, "s1");
  const second = await r.start(b, "s2");
  const record = LocalDb.prototype.recordFinalCommit;
  const bad = mock.method(LocalDb.prototype, "recordFinalCommit", function (this: LocalDb, member: string, run: string, ...rest: [string, string]) {
    if (run === first.run_id) throw new Error("boom");
    return record.call(this, member, run, ...rest);
  });
  const logged: string[] = [];
  const log = mock.method(console, "error", (...args: unknown[]) => void logged.push(args.map(String).join(" ")));
  try {
    r.at(settled(first.deadline_at));
    await r.tick();
  } finally {
    bad.mock.restore();
    log.mock.restore();
  }
  assert.equal(r.storedRun(second.run_id).final_commit, SHA);
  assert.equal(r.storedRun(first.run_id).final_commit ?? null, null);
  assert.ok(logged.some((l) => l.includes("recordFinalCommits") && l.includes(first.run_id) && l.includes("boom")), logged.join("\n"));
}, HOUR);

lifecycle("a tick makes one queue call however many runs there are, and a repeat inserts none", async (r) => {
  for (let i = 0; i < 12; i++) await r.start(await r.cred(`m${i}`), `s${i}`);
  const queue = LocalDb.prototype.queueNotices;
  const sent: { count: number; inserted: number }[] = [];
  const spy = mock.method(LocalDb.prototype, "queueNotices", async function (this: LocalDb, notices: Parameters<typeof queue>[0]) {
    const inserted = await queue.call(this, notices);
    sent.push({ count: notices.length, inserted });
    return inserted;
  });
  try {
    await r.tick();
    assert.deepEqual(sent, [{ count: 12, inserted: 12 }]); // twelve started notices, one call
    await r.tick();
    assert.deepEqual(sent.slice(1), [{ count: 12, inserted: 0 }]); // all there already
  } finally {
    spy.mock.restore();
  }
  assert.equal(only(r, "started").length, 12);
});

lifecycle("no run owes anything: no queue call at all", async (r) => {
  const spy = mock.method(LocalDb.prototype, "queueNotices", async () => 0);
  try {
    await r.tick(); // no runs
    assert.equal(spy.mock.callCount(), 0);
  } finally {
    spy.mock.restore();
  }
});

lifecycle("a batch that fails is logged once and the next tick retries all of it", async (r) => {
  const [a, b] = [await r.cred("alice"), await r.cred("bob")];
  const first = await r.start(a, "s1");
  const second = await r.start(b, "s2");
  const queue = LocalDb.prototype.queueNotices;
  let fail = true;
  const flaky = mock.method(LocalDb.prototype, "queueNotices", async function (this: LocalDb, notices: Parameters<typeof queue>[0]) {
    if (fail) throw new Error("boom");
    return queue.call(this, notices);
  });
  const logged: string[] = [];
  const log = mock.method(console, "error", (...args: unknown[]) => void logged.push(args.map(String).join(" ")));
  try {
    await r.tick();
    assert.deepEqual(kinds(r), []);
    assert.equal(logged.filter((l) => l.includes("boom")).length, 1, logged.join("\n"));
    assert.match(logged.join("\n"), /tick queueNotices/);
    fail = false;
    r.after(5 * MIN);
    await r.tick();
  } finally {
    flaky.mock.restore();
    log.mock.restore();
  }
  assert.deepEqual([kinds(r, first.run_id), kinds(r, second.run_id)], [["started"], ["started"]]);
});

lifecycle("the end queues one ended notice with the commit", async (r) => {
  const calls = stubGithub(pushed(SHA));
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.beat(token, run.run_id, run.entry_id);
  r.at(t(3600 - 1));
  await r.tick();
  r.at(run.deadline_at);
  await r.tick();
  assert.equal(calls.length, 0, "nothing is asked of GitHub before the end has settled");
  r.at(settled(run.deadline_at));
  await r.tick();
  assert.deepEqual(only(r, "ended").map((n) => [n.dedupe_key, n.payload]), [["", { ends_at: run.deadline_at, commit: SHA, repo_url: REPO, uploaded: false, closes_at: closes(run.deadline_at) }]]);
  assert.deepEqual(calls.map((c) => c.url), ASKED);
  assert.ok(calls.every((c) => c.headers.authorization === "Bearer tok"));
  r.after(5 * MIN);
  await r.tick();
  assert.equal(only(r, "ended").length, 1);
  assert.equal(calls.length, 2);
}, { ...HOUR, GITHUB_TOKEN: "tok" });

lifecycle("the judged commit is the last push before the deadline; a later push is not judged and is flagged", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  const LATE = "c3".repeat(20);
  stubGithub(repo([{ after: LATE, before: SHA, at: t(3600 + 60) }, { after: SHA, at: t(3000) }, { after: "b2".repeat(20), at: t(600) }]));
  r.at(settled(run.deadline_at));
  await r.tick();
  const s = r.storedRun(run.run_id);
  assert.deepEqual(
    [s.final_commit, s.final_pushed_at, s.final_branch, s.final_late_push, s.final_force_push, s.final_head_fallback],
    [SHA, t(3000), "main", true, false, false],
  );
  assert.deepEqual(only(r, "ended").map((n) => n.payload.commit), [SHA]);
}, HOUR);

lifecycle("a force push during the run is flagged (the activity is read page by page back to the start)", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  const calls = stubGithub(repo([
    { after: SHA, at: t(3000) },
    { after: "f0".repeat(20), at: t(2000), type: "force_push" },
    { after: "b2".repeat(20), at: t(-600) },
    { after: "b1".repeat(20), at: t(-1200), type: "force_push" }, // before the run, and on a page never read
  ], {}, 1));
  r.at(settled(run.deadline_at));
  await r.tick();
  const s = r.storedRun(run.run_id);
  assert.deepEqual([s.final_commit, s.final_force_push, s.final_late_push], [SHA, true, false]);
  assert.equal(calls.filter((c) => c.url.includes("/activity")).length, 3, "the page that reaches back before the start is the last read");
}, HOUR);

lifecycle("an activity API that keeps failing falls back to commits/HEAD after an hour, flagged", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  const LATE = "c3".repeat(20);
  const calls = stubGithub(repo([{ after: LATE, at: t(3600 + 60) }], { activity: 503 }));
  r.at(settled(run.deadline_at));
  await r.tick();
  r.at(t(3600 + GITHUB_GRACE - 1));
  await r.tick();
  assert.equal(r.storedRun(run.run_id).final_commit ?? null, null, "a failing activity API is retried within the hour");
  assert.ok(!calls.some((c) => c.url.endsWith("/commits/HEAD")));
  r.at(t(3600 + GITHUB_GRACE));
  await r.tick();
  const s = r.storedRun(run.run_id);
  assert.deepEqual([s.final_commit, s.final_head_fallback, s.final_pushed_at, s.final_branch], [LATE, true, null, "main"]);
}, HOUR);

lifecycle("a 404 is retried for an hour before the absence is recorded, and the ended notice says so", async (r) => {
  // A repository that is briefly private: the 404 is not kept, and the next tick records the push.
  const back = await r.start(await r.cred("bob"), "s0");
  stubGithub(repo([], { repo: 404 }));
  r.at(settled(back.deadline_at));
  await r.tick();
  assert.equal(r.storedRun(back.run_id).final_commit ?? null, null);
  stubGithub(pushed(SHA));
  r.after(5 * MIN);
  await r.tick();
  assert.equal(r.storedRun(back.run_id).final_commit, SHA);

  r.at(t(0));
  const run = await r.start(await r.cred("alice"), "s1");
  const asked = stubGithub(repo([], { repo: 404 }));
  r.at(settled(run.deadline_at));
  await r.tick();
  r.at(t(3600 + GITHUB_GRACE - 1));
  await r.tick();
  assert.equal(r.storedRun(run.run_id).final_commit ?? null, null);
  r.at(t(3600 + GITHUB_GRACE));
  await r.tick();
  assert.equal(r.storedRun(run.run_id).final_commit, "none");
  // The absence is final: later ticks never ask GitHub again.
  const count = asked.length;
  r.after(2 * HOURS);
  await r.tick();
  assert.equal(asked.length, count);
  assert.deepEqual(only(r, "ended").filter((n) => n.run_id === run.run_id).map((n) => n.payload.commit), [null]);
}, HOUR);

lifecycle("an empty repository (no activity, commits/HEAD 409) is a recorded absence at once", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  stubGithub(repo([], { head: 409 }));
  r.at(settled(run.deadline_at));
  await r.tick();
  const s = r.storedRun(run.run_id);
  assert.deepEqual([s.final_commit, s.final_head_fallback, s.final_late_push], ["none", true, false]);
}, HOUR);

lifecycle("the ended notice goes out an hour after the end even when GitHub never answered", async (r) => {
  stubGithub(() => new Error("offline"));
  const run = await r.start(await r.cred("alice"), "s1");
  r.at(run.deadline_at);
  await r.tick();
  r.after(55 * MIN);
  await r.tick();
  assert.equal(only(r, "ended").length, 0);
  r.after(5 * MIN);
  await r.tick();
  assert.deepEqual(only(r, "ended").map((n) => n.payload), [{ ends_at: run.deadline_at, commit: null, repo_url: REPO, uploaded: false, closes_at: closes(run.deadline_at) }]);
  // GitHub comes back: the commit is recorded, but the notice was already sent once.
  stubGithub(pushed(SHA));
  r.after(5 * MIN);
  await r.tick();
  assert.equal(r.storedRun(run.run_id).final_commit, SHA);
  assert.equal(only(r, "ended").length, 1);
}, HOUR);

lifecycle("the ended notice of a run the server holds sealed says it is all in", async (r) => {
  stubGithub(pushed(SHA));
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.finish(token, run.run_id, run.entry_id, await r.hold("alice", run.run_id, run.entry_id));
  assert.equal((await r.seal(token, run.run_id, run.entry_id)).status, 200);
  r.at(settled(run.deadline_at));
  await r.tick();
  assert.deepEqual(only(r, "ended").map((n) => [n.payload.uploaded, n.payload.closes_at]), [[true, closes(run.deadline_at)]]);
}, HOUR);

lifecycle("the deadline commit is recorded once and retried after a GitHub failure", async (r) => {
  const ok = pushed(SHA);
  const calls = stubGithub((n, url) => (n === 1 ? new Error("offline") : n === 2 ? json({}, 503) : n === 3 ? json({}, 403) : ok(n, url)));
  const run = await r.start(await r.cred("alice"), "s1");
  const stored = () => r.storedRun(run.run_id).final_commit ?? null;
  r.at(settled(run.deadline_at));
  for (let i = 0; i < 3; i++) {
    await r.tick();
    assert.equal(stored(), null, `failure ${i + 1} stores nothing`);
    r.after(5 * MIN);
  }
  await r.tick();
  assert.equal(stored(), SHA);
  assert.equal(calls.length, 5);
  r.after(5 * MIN);
  await r.tick();
  assert.equal(calls.length, 5, "a recorded commit is not asked for again");
  // An activity answer that is not a list of activities is a failure too.
  stubGithub(repo([{ after: "not-a-sha", at: T0 }]));
  r.at(t(0));
  const other = await r.start(await r.cred("bob"), "s2");
  r.at(settled(other.deadline_at));
  await r.tick();
  assert.equal(r.storedRun(other.run_id).final_commit ?? null, null);
}, HOUR);

lifecycle("no notice after the end, for practice, or for a replaced run", async (r) => {
  const asked = stubGithub(pushed(SHA));
  const [a, b, pr] = [await r.cred("alice"), await r.cred("alice"), await r.cred("alice", "practice")];
  const practice = await r.start(pr, "p1");
  await r.beat(pr, practice.run_id, practice.entry_id, { recording: "failed" });
  await r.tick(); // while it is open
  assert.deepEqual(kinds(r), []);
  const first = await r.start(a, "s1");
  await r.beat(a, first.run_id, first.entry_id, { recording: "failed" });
  r.after(10 * MIN);
  const second = await r.start(b, "s2"); // the first run ends now
  await r.beat(b, second.run_id, second.entry_id, { recording: "failed" });

  // The first tick comes after the first run's end: no started, no recording, no ended (it was replaced).
  r.after(MIN);
  await r.tick();
  assert.deepEqual(kinds(r, first.run_id), []);
  assert.deepEqual(kinds(r, practice.run_id), []);
  assert.deepEqual(kinds(r, second.run_id), ["recording", "started"]);

  // The second run's end: the ended notice, and nothing else (it was flagged, and it is over).
  r.at(settled(second.deadline_at));
  await r.tick();
  assert.deepEqual(kinds(r, second.run_id), ["ended", "recording", "started"]);
  r.at(t(10 * HOURS));
  await r.tick();
  assert.deepEqual(kinds(r, first.run_id), []);
  assert.deepEqual(kinds(r, practice.run_id), []);
  assert.deepEqual(asked.map((c) => c.url), ASKED, "only the main run's commit is asked for");
  assert.equal(r.storedRun(first.run_id).final_commit ?? null, null);
  assert.equal(r.storedRun(practice.run_id).final_commit ?? null, null);
}, HOUR);

lifecycle("after the end nothing but ended is queued, even for a run the tick never saw", async (r) => {
  stubGithub(pushed(SHA));
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.beat(token, run.run_id, run.entry_id, { recording: "gaps" });
  r.at(settled(run.deadline_at)); // no tick since the start: no started, no recording, no silence either
  assert.equal(await queueNotices(await r.ctx(), new Date(settled(run.deadline_at))), 0, "the commit is not recorded yet, and the hour has not passed");
  await r.tick();
  assert.deepEqual(kinds(r), ["ended"]);
}, HOUR);

lifecycle("queueNotices counts the notices it queued, and a repeat queues none", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.beat(token, run.run_id, run.entry_id, { recording: "failed" });
  const ctx = await r.ctx();
  assert.equal(await queueNotices(ctx, new Date(T0)), 2); // started, recording
  assert.equal(await queueNotices(ctx, new Date(T0)), 0);
  // Two ticks at once queue each notice once.
  r.after(21 * MIN);
  await r.start(await r.cred("bob"), "s2");
  const now = new Date(t(21 * MIN));
  const [x, y] = await Promise.all([queueNotices(ctx, now), queueNotices(ctx, now)]);
  assert.equal(x + y, 1); // bob's started (alice is silent, but 21 minutes without a tick is the server's trouble)
  assert.deepEqual(kinds(r), ["recording", "started", "started"]);
});

lifecycle("the cron closes a run that is over, so its status is final without anyone calling", async (r) => {
  stubGithub(pushed(SHA));
  const [c1, c2] = [await r.cred("alice"), await r.cred("bob")];
  const sealed = await r.start(c1, "s1");
  const lost = await r.start(c2, "s2");
  await r.finish(c1, sealed.run_id, sealed.entry_id, await r.hold("alice", sealed.run_id, sealed.entry_id));
  assert.equal((await r.seal(c1, sealed.run_id, sealed.entry_id)).body.run_status, "open"); // sealed before its end
  await r.tick();
  assert.deepEqual([r.storedRun(sealed.run_id).status, r.storedRun(lost.run_id).status], ["open", "open"]);

  r.at(sealed.deadline_at);
  await r.tick();
  assert.deepEqual([r.storedRun(sealed.run_id).status, r.storedRun(lost.run_id).status], ["sealed", "open"]);
  r.after(24 * HOURS);
  await r.tick();
  assert.equal(r.storedRun(lost.run_id).status, "open", "closed strictly more than 24 hours after the end");
  r.after(1);
  await r.tick();
  assert.deepEqual([r.storedRun(sealed.run_id).status, r.storedRun(lost.run_id).status], ["sealed", "incomplete"]);
}, HOUR);

lifecycle("a step that fails does not stop the others", async (r) => {
  stubGithub(pushed(SHA));
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.finish(token, run.run_id, run.entry_id, await r.hold("alice", run.run_id, run.entry_id));
  assert.equal((await r.seal(token, run.run_id, run.entry_id)).status, 200); // at the deadline the run is to be closed as sealed
  const closeRun = mock.method(LocalDb.prototype, "closeRun", async () => {
    throw new Error("boom");
  });
  const logged: string[] = [];
  const log = mock.method(console, "error", (...args: unknown[]) => void logged.push(args.map(String).join(" ")));
  try {
    r.at(settled(run.deadline_at)); // closing is the first step, and it fails
    await r.tick();
  } finally {
    closeRun.mock.restore();
    log.mock.restore();
  }
  assert.deepEqual(logged.filter((l) => l.includes("boom")).length, 1, logged.join("\n"));
  assert.match(logged.join("\n"), /closeRuns/);
  assert.equal(r.storedRun(run.run_id).status, "open");
  assert.equal(r.storedRun(run.run_id).final_commit, SHA); // the next steps ran
  assert.deepEqual(kinds(r), ["ended"]);
}, HOUR);

lifecycle("while the maintenance switch is on the cron does nothing", async (r) => {
  await r.start(await r.cred("alice"), "s1");
  await runTick({ ...r.env, MAINTENANCE: "on" });
  assert.deepEqual(kinds(r), []);
  await runTick(r.env);
  assert.deepEqual(kinds(r), ["started"]);
});

lifecycle("the cron reads every run, however many there are, a page at a time", async (r) => {
  for (let i = 0; i < 7; i++) await r.start(await r.cred(`m${i}`), `s${i}`);
  r.bucket.pageLimit = 3;
  await r.tick();
  assert.equal(only(r, "started").length, 7);
});

// ---- vitals and the disk_full email ----------------------------------------------------------

const PAUSED = { vitals: { disk_free_bytes: 1024, disk_short: true, agent_paused: "disk" } };
const RUNNING = { vitals: { disk_free_bytes: 50 * 2 ** 30, disk_short: false, agent_paused: null } };
type DiskState = { last_vitals: unknown; disk_paused_at: string | null; disk_notice_at: string | null; entries: { last_heartbeat: { vitals?: unknown } }[] };

lifecycle("vitals are kept on the heartbeat and as the run's latest; unknown keys and nulls are dropped", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const stored = () => r.storedRun(run.run_id) as unknown as DiskState;
  r.after(MIN);
  const sent = { disk_free_bytes: 2 ** 53 - 1, disk_short: false, agent_paused: null, rss_bytes: 123_456_789, cpu_ms: 4200, open_files: 17, gpu_temp: 80, nested: { x: 1 } };
  const res = await r.beat(token, run.run_id, run.entry_id, { vitals: sent });
  assert.equal(res.status, 200, JSON.stringify(res.body));
  const kept = { disk_free_bytes: 2 ** 53 - 1, disk_short: false, rss_bytes: 123_456_789, cpu_ms: 4200, open_files: 17 };
  assert.deepEqual(stored().entries[0]!.last_heartbeat.vitals, kept);
  assert.deepEqual(stored().last_vitals, kept);

  // A heartbeat without vitals has none on its own record and keeps the run's latest.
  r.after(5 * MIN);
  for (const without of [{}, { vitals: null }]) assert.equal((await r.beat(token, run.run_id, run.entry_id, without)).status, 200);
  assert.equal(stored().entries[0]!.last_heartbeat.vitals ?? null, null);
  assert.deepEqual(stored().last_vitals, kept);
  // An empty object is vitals with nothing measured.
  await r.beat(token, run.run_id, run.entry_id, { vitals: { cpu_ms: null } });
  assert.deepEqual(stored().last_vitals, {});
});

lifecycle("malformed vitals are 400 and change nothing", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  for (const vitals of [
    5, "disk", [], true,
    { disk_free_bytes: -1 }, { disk_free_bytes: 1.5 }, { disk_free_bytes: "12" }, { disk_free_bytes: 2 ** 53 },
    { rss_bytes: true }, { cpu_ms: {} }, { open_files: [] },
    { disk_short: "yes" }, { disk_short: 1 },
    { agent_paused: "memory" }, { agent_paused: true }, { agent_paused: "" },
  ]) {
    const res = await r.beat(token, run.run_id, run.entry_id, { vitals });
    assert.equal(res.status, 400, JSON.stringify(vitals));
    assert.equal((res.body as { error: string }).error, "malformed");
  }
  const s = r.storedRun(run.run_id) as unknown as DiskState & { last_heartbeat_at: string | null };
  assert.deepEqual([s.last_heartbeat_at, s.last_vitals, s.disk_paused_at], [null, null, null]);
});

lifecycle("a disk pause queues one disk_full notice per pause, again after a resume, but not twice within 6 hours", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const beat = (at: number, extra: Record<string, unknown>) => {
    r.at(t(at));
    return r.beat(token, run.run_id, run.entry_id, extra);
  };
  const tickAt = async (at: number) => {
    r.at(t(at));
    await r.tick();
  };
  const stored = () => {
    const s = r.storedRun(run.run_id) as unknown as DiskState;
    return [s.disk_paused_at, s.disk_notice_at];
  };
  const disk = () => only(r, "disk_full").map((n) => [n.dedupe_key, n.payload]);

  await beat(MIN, PAUSED);
  await beat(2 * MIN, PAUSED); // the same pause
  assert.deepEqual(stored(), [t(MIN), t(MIN)]);
  assert.deepEqual(disk(), []); // the cron decides, not the request
  await tickAt(5 * MIN);
  assert.deepEqual(disk(), [[t(MIN), { since: t(MIN) }]]);
  await tickAt(10 * MIN);
  assert.equal(disk().length, 1, "one pause, one email");

  // Space again: the pause ends. A new pause within 6 hours of the last email is owed nothing.
  await beat(20 * MIN, RUNNING);
  assert.deepEqual(stored(), [null, t(MIN)]);
  await beat(HOURS, PAUSED);
  assert.deepEqual(stored(), [t(HOURS), t(MIN)]);
  await tickAt(HOURS + 5 * MIN);
  assert.equal(disk().length, 1, "throttled: less than 6 hours since the last");
  await beat(HOURS + 10 * MIN, { vitals: {} }); // vitals without agent_paused: not paused
  assert.deepEqual(stored(), [null, t(MIN)]);

  // Six hours after the last email, a new pause is owed one.
  await beat(6 * HOURS + MIN, PAUSED);
  assert.deepEqual(stored(), [t(6 * HOURS + MIN), t(6 * HOURS + MIN)]);
  await tickAt(6 * HOURS + 5 * MIN);
  assert.deepEqual(disk().map(([key]) => key), [t(MIN), t(6 * HOURS + MIN)]);
});

lifecycle("a heartbeat without vitals neither starts nor ends a disk pause", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const paused = () => (r.storedRun(run.run_id) as unknown as DiskState).disk_paused_at;
  r.after(MIN);
  await r.beat(token, run.run_id, run.entry_id, { model_requests: 1 });
  assert.equal(paused() ?? null, null);
  await r.beat(token, run.run_id, run.entry_id, PAUSED);
  r.after(MIN);
  await r.beat(token, run.run_id, run.entry_id, { vitals: null, model_requests: 2 });
  assert.equal(paused(), t(MIN));
});

lifecycle("no disk_full for a practice run; a test run gets one (its operator's, marked [TEST] by the database)", async (r) => {
  const practice = await r.cred("alice", "practice");
  const p = await r.start(practice, "p1");
  const test_ = await credentialFor(r.api, "op-1", "test");
  const tr = await r.start(test_, "t1");
  r.after(MIN);
  await r.beat(practice, p.run_id, p.entry_id, PAUSED);
  await r.beat(test_, tr.run_id, tr.entry_id, PAUSED);
  assert.equal((r.storedRun(p.run_id) as unknown as DiskState).disk_notice_at, t(MIN)); // recorded all the same
  await r.tick();
  assert.deepEqual(kinds(r, p.run_id), []);
  assert.deepEqual(kinds(r, tr.run_id), ["disk_full", "started"]);
}, { ...WINDOW, TEST_EVENT_ID: "test" });

lifecycle("after the run's end a disk pause is owed no email", async (r) => {
  stubGithub(pushed(SHA));
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  r.at(run.deadline_at);
  await r.beat(token, run.run_id, run.entry_id, PAUSED);
  await r.tick();
  assert.deepEqual(only(r, "disk_full"), []);
}, HOUR);

test("without GITHUB_TOKEN outside local tests the final-commit step warns loudly and still runs", async () => {
  const due = { run_id: "r0000000000000001", member_id: "m", mode: "competition", repo_url: REPO, official_start: T0, deadline_at: t(3600), replaced_at: null, final_commit: null } as unknown as TickRun;
  const ctx = { env: { DB_BACKEND: "supabase" }, db: { runsForTick: async () => [due] } } as unknown as Ctx;
  stubGithub(() => new Error("offline"));
  const logged: string[] = [];
  const log = mock.method(console, "error", (...args: unknown[]) => void logged.push(args.map(String).join(" ")));
  try {
    assert.equal(await recordFinalCommits(ctx, new Date(t(3600 + 10 * MIN))), 0);
    assert.ok(logged.some((l) => l.startsWith("WARNING") && l.includes("GITHUB_TOKEN")), logged.join("\n"));
    logged.length = 0;
    assert.equal(await recordFinalCommits({ ...ctx, env: { DB_BACKEND: "supabase", GITHUB_TOKEN: "tok" } } as unknown as Ctx, new Date(t(3600 + 10 * MIN))), 0);
    assert.ok(!logged.some((l) => l.includes("GITHUB_TOKEN")), logged.join("\n"));
  } finally {
    log.mock.restore();
  }
});
