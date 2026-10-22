// Configuration guards and the SupabaseDb request shape (against a stubbed fetch: no Supabase here).

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, test } from "node:test";
import { getDb, readConfig } from "../src/context.ts";
import { SupabaseDb } from "../src/db-supabase.ts";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import { REPO_URL, parseSafeUint } from "../src/util.ts";

const baseEnv = { EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;

/** Run `fn` with console.error captured; returns what was logged. */
async function captureErrors(fn: () => Promise<unknown>): Promise<string> {
  const lines: string[] = [];
  const real = console.error;
  console.error = (...args: unknown[]) => void lines.push(args.map(String).join(" "));
  try {
    await fn();
  } catch {
    // the caller asserts on the rejection separately
  } finally {
    console.error = real;
  }
  return lines.join("\n");
}

test("the default database backend is supabase, and it refuses to start without its secrets", async () => {
  const logged = await captureErrors(() => getDb({ ...baseEnv }));
  assert.match(logged, /SUPABASE_URL/); // the detail is in the log...
  await assert.rejects(getDb({ ...baseEnv }), (e) => e instanceof ApiError && e.code === "misconfigured" && !/SUPABASE/.test(e.message)); // ...not in the answer
});

test("the local backend needs nothing else to start", async () => {
  await getDb({ ...baseEnv, DB_BACKEND: "local" });
});

test("an unknown backend is refused", async () => {
  await captureErrors(async () => {
    await assert.rejects(getDb({ ...baseEnv, DB_BACKEND: "sqlite" }), ApiError);
  });
});

test("config defaults and validation", () => {
  assert.deepEqual(readConfig(baseEnv), {
    eventIds: ["cup"],
    practiceEventId: "practice",
    testEventId: undefined,
    runSeconds: 168 * 3600,
    practiceRunSeconds: 600,
    closeAfterSeconds: 86400,
    runMaxCompressedBytes: 68_719_476_736,
    startWindows: undefined,
    githubCheck: false,
  });
  assert.equal(readConfig({ ...baseEnv, RUN_HOURS: "2" }).runSeconds, 7200);
  assert.equal(readConfig({ ...baseEnv, PRACTICE_RUN_MINUTES: "30" }).practiceRunSeconds, 1800);
  assert.throws(() => readConfig({ ...baseEnv, PRACTICE_RUN_MINUTES: "0" }), ApiError);
  assert.throws(() => readConfig({ ...baseEnv, PRACTICE_EVENT_ID: undefined as unknown as string }), (e) => e instanceof ApiError && e.code === "misconfigured");
  assert.throws(() => readConfig({ ...baseEnv, PRACTICE_EVENT_ID: "../x" }), ApiError);
  assert.equal(readConfig({ ...baseEnv, GITHUB_CHECK: "on" }).githubCheck, true);
  assert.equal(readConfig({ ...baseEnv, GITHUB_CHECK: "yes" }).githubCheck, false);
  assert.throws(() => readConfig({ ...baseEnv, EVENT_ID: "../x" }), ApiError);
  assert.throws(() => readConfig({ ...baseEnv, RUN_HOURS: "0" }), ApiError);
  assert.throws(() => readConfig({ ...baseEnv, RUN_HOURS: "abc" }), ApiError);
  assert.equal(readConfig({ ...baseEnv, RUN_MAX_COMPRESSED_BYTES: "1048576" }).runMaxCompressedBytes, 1_048_576);
  for (const bad of ["0", "-1", "1.5", "64GiB", "1e9", "99999999999999999"]) {
    assert.throws(() => readConfig({ ...baseEnv, RUN_MAX_COMPRESSED_BYTES: bad }), (e) => e instanceof ApiError && e.code === "misconfigured", bad);
  }
});

test("EVENT_ID is one event id, or up to 4 different ids separated by commas, kept in order", () => {
  assert.deepEqual(readConfig({ ...baseEnv, EVENT_ID: "goalcup-dryrun-2026" }).eventIds, ["goalcup-dryrun-2026"]);
  assert.deepEqual(readConfig({ ...baseEnv, EVENT_ID: "goalcup-dryrun-2026,goalcup-cup-2026" }).eventIds, ["goalcup-dryrun-2026", "goalcup-cup-2026"]);
  assert.deepEqual(readConfig({ ...baseEnv, EVENT_ID: "d,c,b,a" }).eventIds, ["d", "c", "b", "a"]);
  for (const bad of ["", ",", "a,", ",a", "a,,b", "a, b", "a,a", "a,b,c,d,e", "a,B", "a,../x", "a;b"]) {
    assert.throws(() => readConfig({ ...baseEnv, EVENT_ID: bad }), (e) => e instanceof ApiError && e.code === "misconfigured" && /EVENT_ID/.test(e.message), JSON.stringify(bad));
  }
  assert.throws(() => readConfig({ ...baseEnv, EVENT_ID: undefined as unknown as string }), (e) => e instanceof ApiError && e.code === "misconfigured");
  // A list holds competition events only: never the practice or the test event.
  for (const bad of ["cup,practice", "test,cup"]) {
    assert.throws(() => readConfig({ ...baseEnv, TEST_EVENT_ID: "test", EVENT_ID: bad }), (e) => e instanceof ApiError && e.code === "misconfigured", bad);
  }
});

test("RUN_SECONDS, CLOSE_AFTER_SECONDS and the start window are honoured only with the local backend", () => {
  const local = { ...baseEnv, DB_BACKEND: "local" };
  assert.equal(readConfig({ ...local, RUN_SECONDS: "3" }).runSeconds, 3);
  assert.equal(readConfig({ ...local, RUN_SECONDS: "3", RUN_HOURS: "1" }).runSeconds, 3);
  assert.equal(readConfig({ ...local, CLOSE_AFTER_SECONDS: "5" }).closeAfterSeconds, 5);
  assert.deepEqual(readConfig({ ...local, START_WINDOW_CLOSES: "2026-11-06T10:00:00Z" }).startWindows, { cup: { opens: null, closes: "2026-11-06T10:00:00Z" } });
  assert.deepEqual(readConfig({ ...local, START_WINDOW_OPENS: "2026-11-06T08:00:00Z", START_WINDOW_CLOSES: "2026-11-06T10:00:00Z" }).startWindows, {
    cup: { opens: "2026-11-06T08:00:00Z", closes: "2026-11-06T10:00:00Z" },
  });
  // With an EVENT_ID list, one time is every event's edge, and a list gives one per event ("" is open).
  const two = { ...local, EVENT_ID: "dry,cup" };
  assert.deepEqual(readConfig({ ...two, START_WINDOW_OPENS: "2026-11-06T08:00:00Z" }).startWindows, {
    dry: { opens: "2026-11-06T08:00:00Z", closes: null },
    cup: { opens: "2026-11-06T08:00:00Z", closes: null },
  });
  assert.deepEqual(readConfig({ ...two, START_WINDOW_OPENS: "2026-10-23T00:00:00Z,2026-11-07T00:00:00Z", START_WINDOW_CLOSES: ",2026-11-08T00:00:00Z" }).startWindows, {
    dry: { opens: "2026-10-23T00:00:00Z", closes: null },
    cup: { opens: "2026-11-07T00:00:00Z", closes: "2026-11-08T00:00:00Z" },
  });
  for (const bad of ["2026-10-23T00:00:00Z,2026-11-07T00:00:00Z,2026-11-08T00:00:00Z", "2026-10-23T00:00:00Z,soon"]) {
    assert.throws(() => readConfig({ ...two, START_WINDOW_OPENS: bad }), (e) => e instanceof ApiError && e.code === "misconfigured", bad);
  }
  assert.throws(() => readConfig({ ...local, START_WINDOW_OPENS: "tomorrow" }), (e) => e instanceof ApiError && e.code === "misconfigured");
  // On any other backend their presence is a mistake, so the Worker refuses rather than ignoring them.
  for (const name of ["RUN_SECONDS", "CLOSE_AFTER_SECONDS"] as const) {
    assert.throws(() => readConfig({ ...baseEnv, [name]: "3" }), (e) => e instanceof ApiError && e.code === "misconfigured");
    assert.throws(() => readConfig({ ...baseEnv, DB_BACKEND: "supabase", [name]: "3" }), ApiError);
  }
  for (const name of ["START_WINDOW_OPENS", "START_WINDOW_CLOSES"] as const) {
    assert.throws(() => readConfig({ ...baseEnv, [name]: "2026-11-06T10:00:00Z" }), (e) => e instanceof ApiError && e.code === "misconfigured");
  }
  assert.throws(() => readConfig({ ...local, RUN_SECONDS: "0" }), ApiError);
  assert.throws(() => readConfig({ ...local, RUN_SECONDS: "x" }), ApiError);
});

test("TEST_APPROVAL_KEY is honoured only with the local backend or STAGING=on", () => {
  const key = { ...baseEnv, TEST_APPROVAL_KEY: "k" };
  readConfig({ ...key, DB_BACKEND: "local" });
  readConfig({ ...key, STAGING: "on" });
  readConfig({ ...key, DB_BACKEND: "supabase", STAGING: "on" });
  readConfig(baseEnv); // no key, nothing to refuse
  readConfig({ ...baseEnv, STAGING: "on" });
  for (const env of [key, { ...key, DB_BACKEND: "supabase" }, { ...key, STAGING: "yes" }, { ...key, STAGING: "off" }]) {
    assert.throws(() => readConfig(env), (e) => e instanceof ApiError && e.status === 500 && e.code === "misconfigured");
  }
});

test("repo_url shape", () => {
  assert.ok(REPO_URL.test("https://github.com/agentcommunity/goal-cup-tracer"));
  assert.ok(REPO_URL.test("https://github.com/a-b/c.d_e"));
  for (const bad of [
    "http://github.com/a/b",
    "https://github.com/a",
    "https://github.com/a/b/",
    "https://github.com/a/b/tree/main",
    "https://github.com/a/b?x=1",
    "https://gitlab.com/a/b",
    "https://github.com/a/b c",
    "https://github.com.evil.example/a/b",
    "https://github.com/a/..",
    "https://github.com/a/.",
    "https://github.com/a_b/c",
  ]) {
    assert.equal(REPO_URL.test(bad), false, bad);
  }
});

test("parseSafeUint is strict", () => {
  assert.equal(parseSafeUint("0"), 0);
  assert.equal(parseSafeUint("123"), 123);
  for (const bad of ["", "01", "-1", "1.5", "1e3", " 1", "99999999999999999", "0x10"]) assert.equal(parseSafeUint(bad), null, bad);
});

// ---- SupabaseDb against a stubbed fetch ---------------------------------------------------

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

function stubFetch(reply: (url: string, init: RequestInit) => Response) {
  const calls: { url: string; init: RequestInit }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, init: init ?? {} });
    return reply(url, init ?? {});
  }) as typeof fetch;
  return calls;
}

test("SupabaseDb calls public.goal_cup_* through PostgREST RPC with the dedicated key", async () => {
  const cred = { member_id: "member-uuid", run_id: null, mode: "practice", expires_at: "2026-11-06T10:05:00Z" };
  const calls = stubFetch(() => new Response(JSON.stringify(cred), { status: 200, headers: { "content-type": "application/json" } }));
  const db = new SupabaseDb("https://proj.supabase.example/", "service-key");
  assert.deepEqual(await db.credential("ab".repeat(32), "2026-11-06T09:06:00Z"), cred);
  const c = calls[0]!;
  assert.equal(c.url, "https://proj.supabase.example/rest/v1/rpc/goal_cup_upload_credential");
  assert.equal(c.init.method, "POST");
  const h = c.init.headers as Record<string, string>;
  assert.equal(h["content-profile"], undefined); // schema public: no profile header
  assert.equal(h["accept-profile"], undefined);
  assert.equal(h.apikey, "service-key");
  assert.equal(h.authorization, "Bearer service-key");
  assert.deepEqual(JSON.parse(c.init.body as string), { p_credential_hash: "ab".repeat(32), p_now: "2026-11-06T09:06:00Z" });
});

test("SupabaseDb: unknown credential is null; errors become 502; odd answers become 502", async () => {
  const db = new SupabaseDb("https://p.example", "k");
  stubFetch(() => new Response("null", { status: 200 }));
  assert.equal(await db.credential("x", "t"), null);
  stubFetch(() => new Response('{"message":"boom"}', { status: 500 }));
  await assert.rejects(db.credential("x", "t"), (e) => e instanceof ApiError && e.status === 502);
  stubFetch(() => new Response('"surprise"', { status: 200 }));
  await assert.rejects(db.abandonRun("m", "r0000000000000000", "why", "2026-11-06T09:00:00Z"), (e) => e instanceof ApiError && e.status === 502);
  stubFetch(() => {
    throw new TypeError("network down");
  });
  await assert.rejects(db.credential("x", "t"), (e) => e instanceof ApiError && e.status === 502);
});

// ---- wrangler.toml ---------------------------------------------------------------------------

/** The tables of wrangler.toml with their `key = value` lines as written (strings unquoted). Enough for this file; not TOML. */
const WRANGLER = readFileSync(join(import.meta.dirname, "../wrangler.toml"), "utf8");

function wranglerTables(): Record<string, Record<string, string>> {
  const tables: Record<string, Record<string, string>> = { "": {} };
  let current = "";
  for (const raw of WRANGLER.split("\n")) {
    const line = raw.trim();
    const table = /^\[\[?([A-Za-z0-9_.-]+)\]\]?$/.exec(line);
    if (table !== null) {
      current = table[1]!;
      tables[current] ??= {};
      continue;
    }
    const pair = /^([A-Za-z_]+)\s*=\s*(.*)$/.exec(line);
    if (pair !== null) tables[current]![pair[1]!] = pair[2]!.replace(/^"(.*)"$/, "$1");
  }
  return tables;
}

test("TEST_APPROVAL_KEY is absent from the production block; only staging says STAGING = on", () => {
  const tables = wranglerTables();
  for (const [name, keys] of Object.entries(tables)) {
    assert.ok(!("TEST_APPROVAL_KEY" in keys), `TEST_APPROVAL_KEY is a secret, not a setting of [${name}]`);
  }
  assert.ok(!/^[^#\n]*TEST_APPROVAL_KEY\s*=/m.test(WRANGLER));
  assert.notEqual(tables.vars!.STAGING, "on");
  assert.equal(tables["env.staging.vars"]!.STAGING, "on");
  assert.equal(tables.vars!.DB_BACKEND, "supabase"); // production never runs the local database
});

test("the 5-minute cron runs in production and in staging; staging is a Worker of its own", () => {
  const tables = wranglerTables();
  assert.equal(tables.triggers!.crons, '["*/5 * * * *"]');
  assert.equal(tables["env.staging.triggers"]!.crons, '["*/5 * * * *"]');
  assert.equal(tables[""]!.name, "goalcup-ingest");
  assert.equal(tables["env.staging"]!.name, "goalcup-ingest-staging");
  assert.equal(tables["env.staging.vars"]!.NOTICE_RECIPIENT_OVERRIDE, "delivered@resend.dev"); // staging never emails a member
  assert.ok(!("NOTICE_RECIPIENT_OVERRIDE" in tables.vars!));
  assert.equal(tables[""]!.workers_dev, "false"); // production: custom domain only
  assert.equal(tables[""]!.routes, '[{ pattern = "ingest.agentcommunity.org", custom_domain = true }]');
  // wrangler DOES inherit `routes`: without this, a staging deploy moves the production custom domain
  // to the staging Worker (it happened once, 2026-10-15).
  assert.equal(tables["env.staging"]!.routes, "[]");
  assert.notEqual(tables.r2_buckets!.preview_bucket_name, tables.r2_buckets!.bucket_name); // dev never writes production
  // wrangler does not inherit vars into an environment: staging repeats every one, and has its own events and bucket.
  for (const name of Object.keys(tables.vars!)) assert.ok(name in tables["env.staging.vars"]!, `staging needs ${name}`);
  assert.notEqual(tables["env.staging.vars"]!.EVENT_ID, tables.vars!.EVENT_ID);
  assert.notEqual(tables["env.staging.vars"]!.PRACTICE_EVENT_ID, tables.vars!.PRACTICE_EVENT_ID);
  assert.notEqual(tables["env.staging.r2_buckets"]!.bucket_name, tables.r2_buckets!.bucket_name);
});

test("the per-run size limit is 64 GiB of compressed chunks in production and in staging", () => {
  const tables = wranglerTables();
  assert.equal(tables.vars!.RUN_MAX_COMPRESSED_BYTES, "68719476736");
  assert.equal(tables["env.staging.vars"]!.RUN_MAX_COMPRESSED_BYTES, "68719476736"); // not inherited by an environment
});

test("production and staging both bind the device limiter; staging's limits are low enough to see a 429", () => {
  const tables = wranglerTables();
  assert.equal(tables["durable_objects.bindings"]!.name, "DEVICE_LIMITER");
  assert.equal(tables["env.staging.durable_objects.bindings"]!.name, "DEVICE_LIMITER"); // not inherited by an environment
  assert.equal(tables.migrations!.new_sqlite_classes, '["DeviceLimiter"]');
  assert.ok(!/^\s*\[\[(env\.staging\.)?ratelimits\]\]/m.test(WRANGLER)); // the binding that never tripped is gone
  assert.ok(Number(tables.vars!.DEVICE_POLL_PER_MINUTE) >= 24); // one goalcup polls its code 12 times a minute
  assert.ok(Number(tables.vars!.DEVICE_START_PER_MINUTE) >= 120); // a venue or campus NAT starts many sign-ins
  assert.ok(Number(tables.vars!.DEVICE_TOKEN_PER_MINUTE) >= 1200); // per address: an abuse cap only
  assert.ok(Number(tables["env.staging.vars"]!.DEVICE_START_PER_MINUTE) <= 10);
  assert.ok(Number(tables["env.staging.vars"]!.DEVICE_POLL_PER_MINUTE) <= 10);
  assert.ok(!/^[^#\n]*RATE_LIMIT_KEY\s*=/m.test(WRANGLER)); // a secret, never a var
});

test("Workers Logs are on in production and in staging", () => {
  const tables = wranglerTables();
  assert.equal(tables.observability!.enabled, "true");
  assert.equal(tables["env.staging.observability"]!.enabled, "true"); // pinned here, not left to inheritance
});

test("Workers Logs keep the Worker's own console lines and no invocation logs, in production and in staging", () => {
  const tables = wranglerTables();
  assert.equal(tables["observability.logs"]!.invocation_logs, "false");
  assert.equal(tables["env.staging.observability.logs"]!.invocation_logs, "false"); // pinned here, not left to inheritance
});
