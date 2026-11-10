// H23, the Supabase key cut-over: the Worker sends its secret API key on `apikey` only, refuses any
// other key kind, and logs the key's fingerprint once (never the key). The static half (G-CODE) fails
// if a Bearer header or a legacy key name comes back into the Worker's Supabase path.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, test } from "node:test";
import { getDb } from "../src/context.ts";
import { SupabaseDb, keyFingerprint } from "../src/db-supabase.ts";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";

const realFetch = globalThis.fetch;
const realLog = console.log;
const realError = console.error;
afterEach(() => {
  globalThis.fetch = realFetch;
  console.log = realLog;
  console.error = realError;
});

/** Record each request's headers; answer every call with `null`. */
function stubFetch(): Record<string, string>[] {
  const headers: Record<string, string>[] = [];
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    headers.push(Object.fromEntries(new Headers(init?.headers).entries()));
    return new Response("null", { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  return headers;
}

/** Collect console.log lines for the duration of `fn`. */
async function logged(fn: () => Promise<void>): Promise<string[]> {
  const lines: string[] = [];
  console.log = (...args: unknown[]) => void lines.push(args.map(String).join(" "));
  await fn();
  return lines;
}

const fp12 = (key: string) => createHash("sha256").update(key).digest("hex").slice(0, 12);

test("H23: the secret key goes on apikey only, never as Authorization: Bearer", async () => {
  const headers = stubFetch();
  const key = "sb_secret_h23_headers";
  await new SupabaseDb("https://p.example", key).credential("ab".repeat(32), "2026-11-10T09:00:00Z");
  assert.equal(headers.length, 1);
  assert.equal(headers[0]!.apikey, key);
  assert.equal(headers[0]!.authorization, undefined);
  assert.ok(!Object.values(headers[0]!).some((v) => /bearer/i.test(v)), "no header value may carry a Bearer token");
});

test("H23: a legacy JWT, a publishable key, a bare prefix or garbage is refused at construction", () => {
  // A syntactically JWT-shaped placeholder: three base64url parts, not a real token.
  const legacyJwt = ["eyJhbGciOiJIUzI1NiJ9", "eyJyb2xlIjoic2VydmljZV9yb2xlIn0", "c2ln"].join(".");
  for (const key of [legacyJwt, "sb_publishable_abc", "sb_secret_", "SB_SECRET_abc", "key", " sb_secret_abc"]) {
    assert.throws(
      () => new SupabaseDb("https://p.example", key),
      (e: unknown) => e instanceof Error && /SUPABASE_KEY must be a Supabase secret API key/.test(e.message) && (key.length < 12 || !e.message.includes(key)),
      `accepted ${JSON.stringify(key.slice(0, 16))}`,
    );
  }
});

test("H23: a wrong key kind makes the Worker misconfigured (500) and the log does not echo the key", async () => {
  const errors: string[] = [];
  console.error = (...args: unknown[]) => void errors.push(args.map(String).join(" "));
  const key = "sb_publishable_should_not_be_logged";
  const env = { DB_BACKEND: "supabase", SUPABASE_URL: "https://p.example", SUPABASE_KEY: key } as unknown as Env;
  await assert.rejects(getDb(env), (e) => e instanceof ApiError && e.status === 500 && e.code === "misconfigured");
  assert.ok(errors.some((l) => /SUPABASE_KEY must be a Supabase secret API key/.test(l)), errors.join("\n"));
  assert.ok(!errors.some((l) => l.includes(key)), "the key value reached the log");
});

test("H23: key_fp is the first 12 hex of sha256, logged once per key with the kind, and never the key", async () => {
  stubFetch();
  const key = "sb_secret_h23_fingerprint_line";
  assert.equal(await keyFingerprint(key), fp12(key));
  const lines = await logged(async () => {
    const db = new SupabaseDb("https://p.example", key);
    await db.credential("ab".repeat(32), "t");
    await db.credential("cd".repeat(32), "t");
    await new SupabaseDb("https://p.example", key).credential("ef".repeat(32), "t"); // a second client, same isolate
  });
  assert.deepEqual(lines, [`key_switch holder=H23 key_fp=${fp12(key)} key_kind=sb_secret`]);
  assert.ok(!lines.some((l) => l.includes(key)));
});

// ---- G-CODE: static check over the Worker's source -----------------------------------------------

const SRC = join(import.meta.dirname, "..", "src");
const sources = readdirSync(SRC)
  .filter((f) => f.endsWith(".ts"))
  .map((f) => ({ file: f, text: readFileSync(join(SRC, f), "utf8") }));

/**
 * Files allowed to build an `Authorization: Bearer` header, each for a reason that is not the
 * Supabase key. Adding a file here needs the same kind of reason.
 */
const BEARER_ALLOWED: Record<string, string> = {
  "github.ts": "GitHub API token (GITHUB_TOKEN), not a Supabase key",
  "mail.ts": "Resend API key (RESEND_API_KEY), not a Supabase key",
  "auth.ts": "reads the member's run credential (gcr_) from the incoming request",
  "device.ts": "reads TEST_APPROVAL_KEY from the incoming staging request",
};

test("G-CODE H23: no Supabase key is sent as Bearer, and no legacy Supabase key name is read", () => {
  assert.ok(sources.length >= 10, `found only ${sources.length} source files: the scan is broken`);
  assert.ok(sources.some((s) => s.file === "db-supabase.ts"));
  const problems: string[] = [];
  for (const { file, text } of sources) {
    text.split("\n").forEach((line, i) => {
      if (/authorization.{0,20}bearer/i.test(line) && !(file in BEARER_ALLOWED)) problems.push(`${file}:${i + 1}: builds a Bearer header`);
      if (/SUPABASE_SERVICE_ROLE_KEY|SUPABASE_SERVICE_KEY|SUPABASE_ANON_KEY|NEXT_PUBLIC_SUPABASE_ANON_KEY|MAIN_SUPABASE_KEY/.test(line)) {
        problems.push(`${file}:${i + 1}: reads a legacy Supabase key name`);
      }
    });
  }
  // The file that holds SUPABASE_KEY must never put it in an Authorization header, allowed list or not.
  const db = sources.find((s) => s.file === "db-supabase.ts")!.text;
  if (/authorization\s*:/i.test(db)) problems.push("db-supabase.ts: sets an Authorization header");
  if (!db.includes("apikey: this.#key")) problems.push("db-supabase.ts: no longer sends the key on apikey");
  assert.deepEqual(problems, []);
});
