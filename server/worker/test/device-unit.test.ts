// The pure parts of the device flow: codes, constant-time comparison, credential lifetimes.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { Ctx } from "../src/context.ts";
import { boundCredentialExpiry, type CreateDeviceArgs } from "../src/db.ts";
import { USER_CODE_ALPHABET, createDevice, newDeviceCode, newUserCode, normalizeUserCode } from "../src/device.ts";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import { constantTimeEqual, proxyVersionField } from "../src/util.ts";

test("a user code is 8 letters of the agreed alphabet, shown as XXXX-XXXX", () => {
  assert.equal(USER_CODE_ALPHABET, "BCDFGHJKLMNPQRSTVWXZ");
  const seen = new Set<string>();
  for (let i = 0; i < 2000; i++) {
    const code = newUserCode();
    assert.match(code, /^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/);
    for (const ch of code.replace("-", "")) seen.add(ch);
  }
  assert.equal(seen.size, 20, "every letter of the alphabet turns up");
});

test("a device code is 64 lowercase hex digits and never repeats", () => {
  const a = newDeviceCode();
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.notEqual(a, newDeviceCode());
});

test("approval input is case-insensitive, with or without the dash", () => {
  assert.equal(normalizeUserCode("bcdf-ghjk"), "BCDFGHJK");
  assert.equal(normalizeUserCode("BCDFGHJK"), "BCDFGHJK");
  assert.equal(normalizeUserCode("BCDF-GHJK"), "BCDFGHJK");
  for (const bad of ["", "BCDF-GHJ", "BCDF-GHJKL", "AAAA-AAAA", "BCDF GHJK", "bcdf--ghjk", "1234-5678"]) assert.equal(normalizeUserCode(bad), null, bad);
});

test("constantTimeEqual compares strings of any length", async () => {
  assert.equal(await constantTimeEqual("abc", "abc"), true);
  assert.equal(await constantTimeEqual("abc", "abd"), false);
  assert.equal(await constantTimeEqual("abc", "abcd"), false);
  assert.equal(await constantTimeEqual("", ""), true);
});

test("a bound credential lasts until 24 hours after the deadline", () => {
  assert.equal(boundCredentialExpiry("2026-11-13T09:00:00Z"), "2026-11-14T09:00:00Z");
});

const isMalformed = (e: unknown) => e instanceof ApiError && e.status === 400 && e.code === "malformed";

test("Goal Cup Proxy's version is proxy_version or tracer_version; both must agree", () => {
  assert.equal(proxyVersionField({ proxy_version: "0.7.0" }), "0.7.0");
  assert.equal(proxyVersionField({ tracer_version: "0.6.0" }), "0.6.0");
  assert.equal(proxyVersionField({ proxy_version: "0.7.0", tracer_version: "0.7.0" }), "0.7.0");
  assert.equal(proxyVersionField({ proxy_version: "x".repeat(64) }), "x".repeat(64));
  for (const bad of [
    {},
    { proxy_version: "0.7.0", tracer_version: "0.6.0" },
    { proxy_version: "" },
    { tracer_version: "" },
    { proxy_version: "x".repeat(65) },
    { tracer_version: "x".repeat(65) },
    { proxy_version: 7 },
    { proxy_version: null },
    { proxy_version: null, tracer_version: "0.6.0" },
    { proxy_version: "0.7.0", tracer_version: "x".repeat(65) },
  ]) assert.throws(() => proxyVersionField(bad), isMalformed, JSON.stringify(bad));
});

test("POST /v1/device stores proxy_version as the device's tracer_version", async () => {
  const stored: CreateDeviceArgs[] = [];
  const ctx = {
    env: { DB_BACKEND: "local", APPROVAL_URL: "https://example.org/approve" } as unknown as Env,
    db: { createDevice: async (a: CreateDeviceArgs) => void stored.push(a) },
    now: () => new Date("2026-10-21T10:00:00Z"),
  } as unknown as Ctx;
  const post = (body: Record<string, unknown>) =>
    createDevice(ctx, new Request("https://w/v1/device", { method: "POST", body: JSON.stringify({ purpose: "run", mode: "practice", connector_id: "c", ...body }) }));
  assert.equal((await post({ proxy_version: "0.7.0" })).status, 200);
  assert.equal((await post({ tracer_version: "0.6.0" })).status, 200);
  assert.equal((await post({ proxy_version: "0.7.1", tracer_version: "0.7.1" })).status, 200);
  assert.deepEqual(stored.map((d) => d.tracer_version), ["0.7.0", "0.6.0", "0.7.1"]);
  await assert.rejects(post({ proxy_version: "0.7.0", tracer_version: "0.6.0" }), isMalformed);
  await assert.rejects(post({}), isMalformed);
  assert.equal(stored.length, 3, "a refused body stores nothing");
});
