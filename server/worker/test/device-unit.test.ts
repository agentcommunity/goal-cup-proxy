// The pure parts of the device flow: codes, constant-time comparison, credential lifetimes.

import assert from "node:assert/strict";
import { test } from "node:test";
import { boundCredentialExpiry } from "../src/db.ts";
import { USER_CODE_ALPHABET, newDeviceCode, newUserCode, normalizeUserCode } from "../src/device.ts";
import { constantTimeEqual } from "../src/util.ts";

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
