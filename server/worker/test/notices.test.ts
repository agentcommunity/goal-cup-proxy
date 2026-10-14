// What the server does on its own (docs/UPLOAD-API.md, "What the server does on its own"): the
// 5-minute cron closes runs that are over, records the GitHub commit at the deadline, and queues
// the run emails into the outbox. The Worker runs in this process on a clock the test moves
// (test/helpers/inproc.ts), and `tick` runs the cron once at that time.

import assert from "node:assert/strict";
import { afterEach, mock, test } from "node:test";
import { LocalDb } from "../src/db-local.ts";
import { queueNotices, runTick } from "../src/notices.ts";
import { T0, WINDOW, type Rig, lifecycle } from "./helpers/rig.ts";

const REPO = "https://github.com/someone/goal-cup-entry"; // what openBody sends
const SHA = "a1".repeat(20);
/** A competition run of one hour, so the end is a short step away. */
const HOUR = { ...WINDOW, RUN_SECONDS: "3600" };

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

/** Stand in for the GitHub API: `reply` answers each request; the calls are recorded. */
function stubGithub(reply: (n: number) => Response | Error) {
  const calls: { url: string; headers: Record<string, string> }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(input), headers: (init?.headers ?? {}) as Record<string, string> });
    const r = reply(calls.length);
    if (r instanceof Error) throw r;
    return r;
  }) as typeof fetch;
  return calls;
}
const head = (sha: string) => new Response(JSON.stringify({ sha }), { status: 200 });

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

lifecycle("opening a competition run queues one started notice", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  assert.deepEqual(kinds(r), []); // the cron decides, not the request
  await r.tick();
  assert.deepEqual(outbox(r), [{ run_id: run.run_id, member_id: "alice", kind: "started", dedupe_key: "", payload: { official_start: T0, deadline_at: run.deadline_at, repo_url: REPO }, created_at: T0, sent_at: null }]);
  r.after(5 * MIN);
  await r.tick();
  assert.deepEqual(kinds(r), ["started"]); // once
});

lifecycle("20 minutes of silence queues one notice; a second silence another", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  assert.equal((await r.beat(token, run.run_id, run.entry_id)).status, 200); // 09:00:00
  r.at(t(19 * MIN + 59));
  await r.tick();
  assert.deepEqual(kinds(r), ["started"]);

  r.at(t(20 * MIN + 1)); // 20 minutes and 1 second since the last heartbeat
  await r.tick();
  assert.deepEqual(kinds(r), ["silence", "started"]);
  assert.deepEqual(only(r, "silence")[0]!.dedupe_key, T0);
  assert.deepEqual(only(r, "silence")[0]!.payload, { last_heartbeat_at: T0 });
  r.at(t(25 * MIN + 1));
  await r.tick();
  assert.equal(only(r, "silence").length, 1, "the same silence is mailed once");

  // The run comes back for a while, then goes quiet again: another silence, another notice.
  const back = t(35 * MIN + 1);
  r.at(back);
  await r.beat(token, run.run_id, run.entry_id);
  r.at(t(40 * MIN + 1));
  await r.tick();
  assert.equal(only(r, "silence").length, 1);
  r.at(t(55 * MIN + 2));
  await r.tick();
  assert.deepEqual(only(r, "silence").map((n) => n.dedupe_key), [T0, back]);
});

lifecycle("silence counts from the start when no heartbeat ever came", async (r) => {
  const run = await r.start(await r.cred("alice"), "s1");
  r.after(20 * MIN + 1);
  await r.tick();
  assert.deepEqual(only(r, "silence").map((n) => [n.run_id, n.dedupe_key, n.payload]), [[run.run_id, T0, { last_heartbeat_at: null }]]);
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
  await r.beat(token, run.run_id, run.entry_id);
  r.at(t(4 * HOURS)); // no heartbeat for 4 hours, and no model call either
  await r.tick();
  assert.deepEqual(kinds(r), ["silence", "started"]);
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
  assert.deepEqual(plain.body, { stop: false, deadline_at: run.deadline_at, reason: null });
  const nulls = await r.beat(token, run.run_id, run.entry_id, { recording: null, model_requests: null });
  assert.equal(nulls.status, 200, JSON.stringify(nulls.body));
  const s = r.storedRun(run.run_id);
  assert.deepEqual([s.flagged_at, s.model_requests, s.last_model_request_at, s.last_heartbeat_at], [null, 0, T0, t(MIN)]);
});

lifecycle("model_requests above the stored value moves the last model request; equal or lower does not", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const seen = async (after: number, model_requests: number) => {
    r.at(t(after));
    await r.beat(token, run.run_id, run.entry_id, { model_requests });
    const s = r.storedRun(run.run_id);
    return [s.model_requests, s.last_model_request_at];
  };
  assert.deepEqual(await seen(60, 5), [5, t(60)]);
  assert.deepEqual(await seen(120, 5), [5, t(60)]);
  assert.deepEqual(await seen(180, 2), [5, t(60)]); // a counter that restarted lower
  assert.deepEqual(await seen(240, 6), [6, t(240)]);
  assert.deepEqual(await seen(300, 0), [6, t(240)]);
});

lifecycle("each heartbeat keeps the connection's IP and country on the server, and answers neither", async (r) => {
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  const beat = await r.beat(token, run.run_id, run.entry_id, {}, { "cf-connecting-ip": "203.0.113.50" });
  assert.deepEqual(Object.keys(beat.body).sort(), ["deadline_at", "reason", "stop"]);
  const s = r.storedRun(run.run_id) as { client_ip: string; client_country: string; entries: { last_heartbeat: Record<string, unknown> }[] };
  assert.deepEqual([s.client_ip, s.client_country], ["203.0.113.50", "DE"]);
  assert.deepEqual([s.entries[0]!.last_heartbeat.client_ip, s.entries[0]!.last_heartbeat.client_country], ["203.0.113.50", "DE"]);
  assert.ok(!JSON.stringify(beat.body).includes("203.0.113.50"));
  assert.ok(![...r.bucket.map.keys()].some((k) => k.includes("203.0.113.50")), "an address in a storage key");
}, undefined, undefined, { country: "DE" });

lifecycle("the end queues one ended notice with the commit", async (r) => {
  const calls = stubGithub(() => head(SHA));
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.beat(token, run.run_id, run.entry_id);
  r.at(t(3600 - 1));
  await r.tick();
  assert.equal(calls.length, 0, "nothing is asked of GitHub before the end");
  r.at(run.deadline_at);
  await r.tick();
  assert.deepEqual(only(r, "ended").map((n) => [n.dedupe_key, n.payload]), [["", { ends_at: run.deadline_at, commit: SHA, repo_url: REPO }]]);
  assert.deepEqual(calls.map((c) => c.url), ["https://api.github.com/repos/someone/goal-cup-entry/commits/HEAD"]);
  assert.equal(calls[0]!.headers.authorization, "Bearer tok");
  r.after(5 * MIN);
  await r.tick();
  assert.equal(only(r, "ended").length, 1);
  assert.equal(calls.length, 1);
}, { ...HOUR, GITHUB_TOKEN: "tok" });

lifecycle("a repository GitHub does not have is a recorded absence, and the ended notice says so", async (r) => {
  for (const status of [404, 409]) {
    stubGithub(() => new Response("{}", { status }));
    const run = await r.start(await r.cred(`m${status}`), `s${status}`);
    r.at(run.deadline_at);
    await r.tick();
    assert.equal(r.storedRun(run.run_id).final_commit, "none");
    assert.deepEqual(only(r, "ended").filter((n) => n.run_id === run.run_id).map((n) => n.payload.commit), [null]);
    r.at(t(0));
  }
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
  assert.deepEqual(only(r, "ended").map((n) => n.payload), [{ ends_at: run.deadline_at, commit: null, repo_url: REPO }]);
  // GitHub comes back: the commit is recorded, but the notice was already sent once.
  stubGithub(() => head(SHA));
  r.after(5 * MIN);
  await r.tick();
  assert.equal(r.storedRun(run.run_id).final_commit, SHA);
  assert.equal(only(r, "ended").length, 1);
}, HOUR);

lifecycle("the deadline commit is recorded once and retried after a GitHub failure", async (r) => {
  const calls = stubGithub((n) => (n === 1 ? new Error("offline") : n === 2 ? new Response("{}", { status: 503 }) : n === 3 ? new Response("{}", { status: 403 }) : head(SHA)));
  const run = await r.start(await r.cred("alice"), "s1");
  const stored = () => r.storedRun(run.run_id).final_commit ?? null;
  r.at(run.deadline_at);
  for (let i = 0; i < 3; i++) {
    await r.tick();
    assert.equal(stored(), null, `failure ${i + 1} stores nothing`);
    r.after(5 * MIN);
  }
  await r.tick();
  assert.equal(stored(), SHA);
  assert.equal(calls.length, 4);
  r.after(5 * MIN);
  await r.tick();
  assert.equal(calls.length, 4, "a recorded commit is not asked for again");
  // A body without a commit is a failure too.
  stubGithub(() => new Response(JSON.stringify({ sha: "not-a-sha" }), { status: 200 }));
  r.at(t(0));
  const other = await r.start(await r.cred("bob"), "s2");
  r.at(other.deadline_at);
  await r.tick();
  assert.equal(r.storedRun(other.run_id).final_commit ?? null, null);
}, HOUR);

lifecycle("no notice after the end, for practice, or for a replaced run", async (r) => {
  const asked = stubGithub(() => head(SHA));
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
  r.at(second.deadline_at);
  await r.tick();
  assert.deepEqual(kinds(r, second.run_id), ["ended", "recording", "started"]);
  r.at(t(10 * HOURS));
  await r.tick();
  assert.deepEqual(kinds(r, first.run_id), []);
  assert.deepEqual(kinds(r, practice.run_id), []);
  assert.deepEqual(asked.map((c) => c.url), ["https://api.github.com/repos/someone/goal-cup-entry/commits/HEAD"], "only the main run's commit is asked for");
  assert.equal(r.storedRun(first.run_id).final_commit ?? null, null);
  assert.equal(r.storedRun(practice.run_id).final_commit ?? null, null);
}, HOUR);

lifecycle("after the end nothing but ended is queued, even for a run the tick never saw", async (r) => {
  stubGithub(() => head(SHA));
  const token = await r.cred("alice");
  const run = await r.start(token, "s1");
  await r.beat(token, run.run_id, run.entry_id, { recording: "gaps" });
  r.at(run.deadline_at); // no tick since the start: no started, no recording, no silence either
  assert.equal(await queueNotices(await r.ctx(), new Date(run.deadline_at)), 0, "the commit is not recorded yet, and the hour has not passed");
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
  const now = new Date(t(21 * MIN));
  const [x, y] = await Promise.all([queueNotices(ctx, now), queueNotices(ctx, now)]);
  assert.equal(x + y, 1);
  assert.deepEqual(kinds(r), ["recording", "silence", "started"]);
});

lifecycle("the cron closes a run that is over, so its status is final without anyone calling", async (r) => {
  stubGithub(() => head(SHA));
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
  stubGithub(() => head(SHA));
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
    r.at(run.deadline_at); // closing is the first step, and it fails
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
