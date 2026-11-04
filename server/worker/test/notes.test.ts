// Server notes (src/notes.ts): which rows of src/advisories.json an open or a continue answer carries,
// and the shape the committed file must keep.

import assert from "node:assert/strict";
import { test } from "node:test";
import advisories from "../src/advisories.json" with { type: "json" };
import { readConfig } from "../src/context.ts";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import { NOTES_MAX, NOTE_MAX_CHARS, pickNotes, validAdvisories } from "../src/notes.ts";

const rows = [
  { connector: "hermes-cli", version: "0.22.6", os: "*", note: "A" },
  { connector: "hermes-cli", version: "0.22", os: "linux", note: "B" },
  { connector: "*", version: "*", os: "*", note: "C" },
];

test("exact, prefix and any", () => {
  assert.deepEqual(pickNotes(rows, { connector: "hermes-cli", harnessVersion: "Hermes Agent v0.22.6", os: "macos" }), ["A", "C"]);
  assert.deepEqual(pickNotes(rows, { connector: "hermes-cli", harnessVersion: "0.22.60", os: "linux" }), ["B", "C"]);
  assert.deepEqual(pickNotes(rows, { connector: "hermes-cli", harnessVersion: "0.220.1", os: "linux" }), ["C"]);
  assert.deepEqual(pickNotes(rows, { connector: "codex-cli", harnessVersion: "unknown", os: "" }), ["C"]);
  assert.deepEqual(pickNotes(rows, { connector: "hermes-cli", harnessVersion: "0.22", os: "linux" }), ["B", "C"]);
});

test("at most five", () => assert.equal(pickNotes(Array(9).fill(rows[2]), { connector: "x", harnessVersion: "1", os: "" }).length, NOTES_MAX));

test("the committed file is valid", () => assert.equal(validAdvisories(advisories), null));

test("validAdvisories refuses ESC, bidi, 201 chars, a sixth key, version 'v1'", () => {
  const ok = { connector: "*", version: "*", os: "*", note: "fine" };
  assert.equal(validAdvisories([ok, { ...ok, note: "x".repeat(NOTE_MAX_CHARS) }, { ...ok, note: "\u{1F600}".repeat(150) }]), null);
  const bad: unknown[] = [
    [{ ...ok, note: "\u001b[31mred" }],
    [{ ...ok, note: "a\u202Eb" }],
    [{ ...ok, note: "a\u2067b" }],
    [{ ...ok, note: "a\u200Fb" }],
    [{ ...ok, os: "" }],
    [{ ...ok, connector: "" }],
    [{ ...ok, note: "x".repeat(NOTE_MAX_CHARS + 1) }],
    [{ ...ok, note: "" }],
    [{ ...ok, extra: "x" }],
    [{ connector: "*", version: "*", os: "*" }],
    [{ ...ok, version: "v1" }],
    [{ ...ok, os: 5 }],
    Array(51).fill(ok),
    {},
    [null],
  ];
  for (const b of bad) assert.equal(typeof validAdvisories(b), "string", JSON.stringify(b).slice(0, 80));
});

test("ADVISORIES_JSON is honoured only with the local backend, and must be valid", () => {
  const base = { EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", TRACES: {} } as unknown as Env;
  const json = JSON.stringify([{ connector: "*", version: "*", os: "*", note: "hi" }]);
  assert.deepEqual(readConfig({ ...base, DB_BACKEND: "local", ADVISORIES_JSON: json }).advisories, JSON.parse(json));
  assert.deepEqual(readConfig({ ...base, DB_BACKEND: "local" }).advisories, advisories);
  const misconfigured = (e: unknown) => e instanceof ApiError && e.code === "misconfigured";
  assert.throws(() => readConfig({ ...base, ADVISORIES_JSON: json }), misconfigured);
  assert.throws(() => readConfig({ ...base, DB_BACKEND: "local", ADVISORIES_JSON: "[{}]" }), misconfigured);
  assert.throws(() => readConfig({ ...base, DB_BACKEND: "local", ADVISORIES_JSON: "not json" }), misconfigured);
});
