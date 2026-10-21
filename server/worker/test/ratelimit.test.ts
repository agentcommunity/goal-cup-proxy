// The device-flow rate limits (src/ratelimit.ts) against the real DeviceLimiter class behind a fake
// Durable Object namespace: starts per address, polls per device code with a per-address cap, the
// IPv6 /64 key, the keyed hash, the missing-binding and missing-key refusals, and the health check.

import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { test } from "node:test";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import worker from "../src/index.ts";
import { DeviceLimiter, clientAddress, limitAddress, limitDeviceCode } from "../src/ratelimit.ts";

const RATE_LIMIT_KEY = "test-rate-limit-key-".padEnd(48, "x");

/** A namespace that keeps one DeviceLimiter per name, as Cloudflare does, and records the names. */
function fakeNamespace() {
  const objects = new Map<string, DeviceLimiter>();
  const names: string[] = [];
  const ns = {
    idFromName: (name: string) => {
      names.push(name);
      return name;
    },
    get: (id: string) => {
      if (!objects.has(id)) objects.set(id, new DeviceLimiter());
      const obj = objects.get(id)!;
      return { fetch: (url: string, init: RequestInit) => obj.fetch(new Request(url, init)) };
    },
  };
  return { names, objects, binding: ns as unknown as DurableObjectNamespace };
}

const req = (ip: string | null) => new Request("https://x/v1/device", { method: "POST", headers: ip === null ? {} : { "cf-connecting-ip": ip } });
const limited = (e: unknown) => e instanceof ApiError && e.status === 429 && e.code === "rate_limited" && Number(e.headers["retry-after"]) >= 1 && Number(e.headers["retry-after"]) <= 60;
const code = (n: number) => n.toString(16).padStart(64, "0");

/** Run `fn` with console.error captured; returns what was logged. */
async function quietly(fn: () => Promise<unknown>): Promise<string[]> {
  const logged: string[] = [];
  const real = console.error;
  console.error = (...args: unknown[]) => void logged.push(args.map(String).join(" "));
  try {
    await fn();
  } finally {
    console.error = real;
  }
  return logged;
}

test("over the start limit is 429 rate_limited with Retry-After; addresses and calls count apart", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY, DEVICE_START_PER_MINUTE: "2" } as unknown as Env;
  await limitAddress(env, req("203.0.113.5"), "start");
  await limitAddress(env, req("203.0.113.5"), "start");
  await assert.rejects(limitAddress(env, req("203.0.113.5"), "start"), limited);
  await limitAddress(env, req("203.0.113.6"), "start"); // another address has its own count
  await limitAddress(env, req("203.0.113.5"), "token"); // and so has the other call
});

test("a key is an HMAC under RATE_LIMIT_KEY: no raw address, and not the plain SHA-256 an attacker could enumerate", async () => {
  const f = fakeNamespace();
  await limitAddress({ DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY } as unknown as Env, req("203.0.113.5"), "start");
  const expected = createHmac("sha256", RATE_LIMIT_KEY).update("start\u0000" + "203.0.113.5").digest("hex").slice(0, 32);
  assert.deepEqual(f.names, [`start:${expected}`]);
  const other = fakeNamespace();
  await limitAddress({ DEVICE_LIMITER: other.binding, RATE_LIMIT_KEY: RATE_LIMIT_KEY.replace("x", "y") } as unknown as Env, req("203.0.113.5"), "start");
  assert.notEqual(other.names[0], f.names[0]); // another secret, another key
});

test("polls count per device code: many codes behind one address each keep their own limit", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY } as unknown as Env; // the production defaults
  // 50 participants behind one NAT, each polling every 5 seconds for a minute: 600 polls, no 429.
  for (let poll = 0; poll < 12; poll++) {
    for (let p = 0; p < 50; p++) {
      await limitAddress(env, req("198.51.100.1"), "token");
      await limitDeviceCode(env, code(p));
    }
  }
  // One code polled far faster than goalcup does is refused, and only that code.
  for (let i = 12; i < 30; i++) await limitDeviceCode(env, code(0));
  await assert.rejects(limitDeviceCode(env, code(0)), limited);
  await limitDeviceCode(env, code(99));
});

test("the per-address poll cap still stops a flood of codes from one address", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY, DEVICE_TOKEN_PER_MINUTE: "3" } as unknown as Env;
  for (let i = 0; i < 3; i++) await limitAddress(env, req("198.51.100.1"), "token");
  await assert.rejects(limitAddress(env, req("198.51.100.1"), "token"), limited);
});

test("an IPv6 client is counted by its /64", async () => {
  assert.equal(clientAddress("2001:db8:aa:bb:1:2:3:4"), "2001:db8:aa:bb::/64");
  assert.equal(clientAddress("2001:DB8:AA:BB::99"), "2001:db8:aa:bb::/64");
  assert.equal(clientAddress("2001:db8::1"), "2001:db8:0:0::/64");
  assert.equal(clientAddress("2001:0db8:00aa:00bb:ffff:ffff:ffff:ffff"), "2001:db8:aa:bb::/64");
  assert.equal(clientAddress("::ffff:192.0.2.1"), "192.0.2.1");
  assert.equal(clientAddress("192.0.2.1"), "192.0.2.1");
  assert.equal(clientAddress(null), "unknown");

  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY, DEVICE_START_PER_MINUTE: "1" } as unknown as Env;
  await limitAddress(env, req("2001:db8:aa:bb::1"), "start");
  await assert.rejects(limitAddress(env, req("2001:db8:aa:bb:ffff::2"), "start"), limited); // same /64: same count
  await limitAddress(env, req("2001:db8:aa:bc::1"), "start"); // the next /64 is another client
});

test("no limiter on the local backend means no limit", async () => {
  await limitAddress({ DB_BACKEND: "local" } as unknown as Env, req("203.0.113.5"), "start");
  await limitDeviceCode({ DB_BACKEND: "local" } as unknown as Env, code(1));
});

test("no limiter on the Supabase backend refuses the call with 500 misconfigured and says why in the log", async () => {
  const logged = await quietly(async () => {
    for (const env of [{}, { DB_BACKEND: "supabase" }]) {
      await assert.rejects(limitAddress(env as unknown as Env, req("203.0.113.5"), "start"), (e) => e instanceof ApiError && e.status === 500 && e.code === "misconfigured");
    }
    // Through the Worker too: a deployed Worker without the binding never runs the device flow unlimited.
    const env = { DB_BACKEND: "supabase", SUPABASE_URL: "https://example.supabase.co", SUPABASE_KEY: "k", EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;
    for (const path of ["/v1/device", "/v1/device/token"]) {
      const res = await worker.fetch(new Request(`https://x${path}`, { method: "POST", headers: { "cf-connecting-ip": "198.51.100.7", "content-type": "application/json" }, body: "{}" }), env);
      assert.equal(res.status, 500);
      assert.equal(((await res.json()) as { error: string }).error, "misconfigured");
    }
  });
  assert.ok(logged.some((l) => /DEVICE_LIMITER/.test(l)));
});

test("a bound limiter without RATE_LIMIT_KEY (or with a short one) is misconfigured, and the log says so without the key", async () => {
  const f = fakeNamespace();
  const logged = await quietly(async () => {
    for (const key of [undefined, "short"]) {
      await assert.rejects(limitAddress({ DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY: key } as unknown as Env, req("203.0.113.5"), "start"), (e) => e instanceof ApiError && e.status === 500 && e.code === "misconfigured");
    }
  });
  assert.ok(logged.some((l) => /RATE_LIMIT_KEY/.test(l)));
  assert.ok(!logged.some((l) => l.includes("short")));
  assert.equal(f.names.length, 0); // nothing counted under an unkeyed name
});

test("a limit that is not a positive integer is misconfigured", async () => {
  const f = fakeNamespace();
  await quietly(() =>
    assert.rejects(limitDeviceCode({ DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY, DEVICE_POLL_PER_MINUTE: "lots" } as unknown as Env, code(1)), (e) => e instanceof ApiError && e.code === "misconfigured"),
  );
});

test("POST /v1/device answers 429 before any database call when the limit is spent", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY, DEVICE_START_PER_MINUTE: "1", DB_BACKEND: "local", EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;
  const call = () => worker.fetch(new Request("https://x/v1/device", { method: "POST", headers: { "cf-connecting-ip": "198.51.100.7", "content-type": "application/json" }, body: "{}" }), env);
  assert.equal((await call()).status, 400); // under the limit: the body is checked (and is malformed)
  const res = await call();
  assert.equal(res.status, 429);
  assert.deepEqual(await res.json(), { error: "rate_limited", message: "Too many requests from this address; wait a minute and try again." });
});

test("POST /v1/device/token is limited per device code before the database is asked", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, RATE_LIMIT_KEY, DEVICE_POLL_PER_MINUTE: "2", DB_BACKEND: "local", EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;
  let dbCalls = 0;
  (env as unknown as { TRACES: unknown }).TRACES = {
    get: async () => (dbCalls++, null),
    put: async () => null,
    list: async () => ({ objects: [], truncated: false }),
    head: async () => null,
  };
  const poll = (c: string) => worker.fetch(new Request("https://x/v1/device/token", { method: "POST", headers: { "cf-connecting-ip": "198.51.100.7", "content-type": "application/json" }, body: JSON.stringify({ device_code: c }) }), env);
  assert.notEqual((await poll(code(1))).status, 429);
  assert.notEqual((await poll(code(1))).status, 429);
  const before = dbCalls;
  const res = await poll(code(1));
  assert.equal(res.status, 429);
  assert.equal(((await res.json()) as { message: string }).message, "This sign-in is polled too often; wait a minute and try again.");
  assert.equal(dbCalls, before); // refused before any database read
  assert.notEqual((await poll(code(2))).status, 429); // the same address, another code: not limited
});

test("GET /v1/health answers ok without configuration", async () => {
  const res = await worker.fetch(new Request("https://x/v1/health"), {} as Env);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { status: "ok" });
});
