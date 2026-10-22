// The public live totals (docs/UPLOAD-API.md, `GET /v1/live.json`): the cron writes the event's
// community totals to R2, and the GET serves that copy. The Worker runs in this process on a clock
// the test moves (test/helpers/inproc.ts); `tick` runs the cron once.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { LiveSnapshot } from "../src/db.ts";
import { liveKey, publicSnapshot } from "../src/live.ts";
import { credentialFor, openBody } from "./helpers/client.ts";
import { inProcess } from "./helpers/inproc.ts";
import { T0, WINDOW, type Opened, type Rig, lifecycle } from "./helpers/rig.ts";

const MIN = 60;
/** Test and practice runs land in the competition event too, so only the run's mode can leave them out. */
const ONE_EVENT = { ...WINDOW, TEST_EVENT_ID: "cup", PRACTICE_EVENT_ID: "cup" };

/** `n` members each start a run of `mode` and beat once, reporting `calls` model requests. */
async function runs(r: Rig, n: number, opts: { mode?: "competition" | "practice" | "test"; calls?: number; connector?: string; tag?: string } = {}) {
  const out: (Opened & { token: string })[] = [];
  for (let i = 0; i < n; i++) {
    const token = await credentialFor(r.api, `${opts.tag ?? "m"}-${i}`, opts.mode ?? "competition");
    const res = await r.open(token, `${opts.tag ?? "m"}-s${i}`, opts.connector === undefined ? {} : { connector_id: opts.connector });
    assert.equal(res.status, 201, JSON.stringify(res.body));
    const run = { ...(res.body as Opened), token };
    assert.equal((await r.beat(token, run.run_id, run.entry_id, { model_requests: opts.calls ?? 10 })).status, 200);
    out.push(run);
  }
  return out;
}

/** The snapshot the GET serves now. */
async function live(r: Rig): Promise<LiveSnapshot> {
  const res = await r.api.call("GET", "/v1/live.json");
  assert.equal(res.status, 200, JSON.stringify(res.body));
  return res.body as LiveSnapshot;
}

lifecycle("the cron writes live.json and the GET serves it, public for 60 seconds", async (r) => {
  const before = await r.api.call("GET", "/v1/live.json");
  assert.equal(before.status, 404);
  assert.equal(before.body.error, "not_ready");
  await runs(r, 5);
  r.after(MIN);
  await r.tick();
  const res = await r.api.call("GET", "/v1/live.json");
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "public, max-age=60");
  assert.equal(res.headers.get("access-control-allow-origin"), "*");
  assert.equal(res.headers.get("content-type"), "application/json");
  assert.equal(res.body.generated_at, "2026-11-06T09:01:00Z");
  assert.equal(res.body.entries.started, 5);
  assert.deepEqual(res.body, JSON.parse(r.bucket.map.get(liveKey("cup"))!.text));
});

lifecycle("test and practice runs are not counted", async (r) => {
  await runs(r, 5, { calls: 10 });
  await runs(r, 5, { mode: "test", calls: 1000, tag: "t" });
  await runs(r, 5, { mode: "practice", calls: 1000, tag: "p" });
  await r.tick();
  const doc = await live(r);
  assert.equal(doc.entries.started, 5);
  assert.equal(doc.entries.running, 5);
  assert.deepEqual(doc.harnesses, [{ id: "codex-cli", started: 5, running: 5 }]);
  assert.equal(doc.work.model_calls, 50);
}, ONE_EVENT);

lifecycle("a number fed by fewer than 5 runs is null, and a harness under 5 folds into other", async (r) => {
  await runs(r, 4);
  await r.tick();
  let doc = await live(r);
  assert.equal(doc.entries.started, null);
  assert.equal(doc.harnesses, null);
  assert.equal(doc.work.model_calls, null);
  assert.equal(doc.work.agent_hours, null);

  // 4 codex-cli, 1 amp-cli, 1 hermes-cli: codex-cli is under 5 too, so all fold into other (6).
  await runs(r, 1, { connector: "amp-cli", tag: "a" });
  await runs(r, 1, { connector: "hermes-cli", tag: "h" });
  await r.tick();
  doc = await live(r);
  assert.equal(doc.entries.started, 6);
  assert.deepEqual(doc.harnesses, [{ id: "other", started: 6, running: 6 }]);

  // One more codex-cli: it has 5 and shows; other (2) is left out.
  await runs(r, 1, { tag: "c" });
  await r.tick();
  doc = await live(r);
  assert.deepEqual(doc.harnesses, [{ id: "codex-cli", started: 5, running: 5 }]);
  assert.equal(doc.entries.quiet, null); // 0 quiet runs: also under 5
});

lifecycle("a continue adds its model calls and never doubles them; the last hour counts only its growth", async (r) => {
  const all = await runs(r, 5, { calls: 10 }); // 09:00
  r.after(45 * MIN);
  for (const run of all) await r.beat(run.token, run.run_id, run.entry_id, { model_requests: 10 }); // 09:45, no new calls
  r.after(45 * MIN);
  for (const run of all) await r.beat(run.token, run.run_id, run.entry_id, { model_requests: 30 }); // 10:30
  // One run's agent died: the new segment counts from zero.
  const a = all[0]!;
  const c = await r.cont(a.token, a.run_id, a.entry_id, "s-continued");
  assert.equal(c.status, 201, JSON.stringify(c.body));
  await r.beat(a.token, a.run_id, c.body.entry_id, { model_requests: 4 });
  await r.tick();
  const doc = await live(r);
  assert.equal(doc.work.model_calls, 5 * 30 + 4);
  assert.equal(doc.work.model_calls_last_hour, 5 * 20 + 4);
  assert.equal(doc.work.restarts, null); // one run restarted: under 5
  assert.equal(doc.work.agent_hours, 1.3); // 3 slots of 5 minutes per run (the continue beat in a slot its run had): 75 minutes
  assert.equal(doc.work.working_hours, 0.8); // only 2 of them had new calls: 50 minutes
  assert.deepEqual(doc.harnesses, [{ id: "codex-cli", started: 5, running: 5 }]); // the continue's connector does not count
});

/** Every key path of a JSON value, arrays as `[]`. */
function paths(v: unknown, at = ""): string[] {
  if (Array.isArray(v)) return v.flatMap((x) => paths(x, `${at}[]`));
  if (typeof v === "object" && v !== null) return Object.entries(v).flatMap(([k, x]) => [`${at}.${k}`, ...paths(x, `${at}.${k}`)]);
  return [];
}

const PUBLIC_KEYS = [
  ".schema_version", ".event", ".event.id", ".event.kind", ".event.start_window_opens", ".event.start_window_closes", ".event.run_hours",
  ".generated_at", ".min_group",
  ".entries", ".entries.signed_up", ".entries.started", ".entries.running", ".entries.working", ".entries.quiet", ".entries.finished", ".entries.sealed", ".entries.incomplete",
  ".harnesses", ".harnesses[].id", ".harnesses[].started", ".harnesses[].running",
  ".work", ".work.agent_hours", ".work.working_hours", ".work.model_calls", ".work.model_calls_last_hour", ".work.restarts", ".work.data_bytes",
  ".notices", ".notices.silence", ".notices.idle_3h", ".notices.idle_6h", ".notices.disk_full",
  ".survival", ".hourly", ".trace", ".github", ".server",
].sort();

lifecycle("the snapshot holds community totals only: no run, entry, member, repository or address", async (r) => {
  const all = await runs(r, 6);
  await r.tick();
  const res = await r.api.call("GET", "/v1/live.json", { headers: { "cf-connecting-ip": "203.0.113.9" } });
  assert.deepEqual([...new Set(paths(res.body))].sort(), PUBLIC_KEYS);
  const text = JSON.stringify(res.body);
  for (const run of all) {
    assert.ok(!text.includes(run.run_id) && !text.includes(run.entry_id), text);
  }
  for (const secret of ["m-0", "github.com", openBody().workspace, openBody().session_id, "203.0.113"]) assert.ok(!text.includes(secret), `${secret} in ${text}`);
});

test("a field the database adds is never published, and a harness under 5 from the database is refused", () => {
  const good = {
    schema_version: 1,
    event: { id: "cup", kind: "competition", start_window_opens: T0, start_window_closes: null, run_hours: 168, staging: false },
    generated_at: T0, min_group: 5,
    entries: { signed_up: null, started: 6, running: 6, working: null, quiet: null, finished: null, sealed: null, incomplete: null, run_ids: ["r0000000000000001"] },
    harnesses: [{ id: "codex-cli", started: 6, running: 6, members: ["m"] }],
    work: { agent_hours: 1.5, model_calls: 60, model_calls_last_hour: null, restarts: null, data_bytes: 600 },
    notices: { silence: null, idle_3h: null, idle_6h: null, disk_full: null },
    trace: null, github: null, runs: [{ run_id: "r0000000000000001" }],
  };
  assert.deepEqual([...new Set(paths(publicSnapshot(good)))].sort(), PUBLIC_KEYS);
  assert.throws(() => publicSnapshot({ ...good, harnesses: [{ id: "codex-cli", started: 4, running: null }] }));
  assert.throws(() => publicSnapshot({ ...good, harnesses: [{ id: "Robert'); DROP", started: 6, running: 6 }] }));
});

test("the GET never calls the database", async () => {
  // A Supabase backend whose every call would fail: the GET still answers from the bucket.
  const p = inProcess(T0, { DB_BACKEND: "supabase", SUPABASE_URL: "https://db.invalid", SUPABASE_KEY: "k", TEST_APPROVAL_KEY: "" });
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async () => {
    throw new Error("the GET called out");
  }) as typeof fetch;
  try {
    p.bucket.map.set(liveKey("cup"), { text: JSON.stringify({ schema_version: 1 }), etag: "e" });
    const res = await p.api.call("GET", "/v1/live.json");
    assert.equal(res.status, 200, JSON.stringify(res.body));
    assert.deepEqual(res.body, { schema_version: 1 });
  } finally {
    globalThis.fetch = realFetch;
    p.stop();
  }
});

