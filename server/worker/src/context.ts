// Per-request context: parsed configuration, the database, the signer.

import { ABANDON_AFTER_SECONDS, type Db } from "./db.ts";
import { LocalDb } from "./db-local.ts";
import { SupabaseDb } from "./db-supabase.ts";
import type { Env } from "./env.ts";
import { ApiError } from "./errors.ts";
import { Signer } from "./sign.ts";

export interface Config {
  eventId: string;
  /** deadline_at = official_start + this. RUN_HOURS * 3600, or RUN_SECONDS (local backend only). */
  runSeconds: number;
  /** Unsealed this long after deadline_at -> abandoned. 24 h, or ABANDON_AFTER_SECONDS (local backend only). */
  abandonAfterSeconds: number;
  githubCheck: boolean;
}

export interface Ctx {
  env: Env;
  db: Db;
  signer: Signer;
  cfg: Config;
  now: () => Date;
}

const misconfigured = (message: string) => new ApiError(500, "misconfigured", message);

function positiveNumber(name: string, raw: string | undefined, fallback: number, unit: string, max: number): number {
  if (raw === undefined || raw === "") return fallback;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0 || n > max) throw misconfigured(`${name} must be a positive number of ${unit}.`);
  return n;
}

/**
 * RUN_SECONDS and ABANDON_AFTER_SECONDS exist so tests can run a whole deadline in seconds.
 * They are honoured only with DB_BACKEND=local; anywhere else their presence is a mistake worth
 * stopping for, not a value to ignore.
 */
function testOnlySeconds(env: Env, name: "RUN_SECONDS" | "ABANDON_AFTER_SECONDS"): number | undefined {
  const raw = env[name];
  if (raw === undefined || raw === "") return undefined;
  if (env.DB_BACKEND !== "local") throw misconfigured(`${name} is for DB_BACKEND=local (tests) only.`);
  return positiveNumber(name, raw, 0, "seconds", 365 * 24 * 3600);
}

/**
 * TEST_APPROVAL_KEY switches on a route that approves a device request as any member, so it is
 * honoured only where that is meant: DB_BACKEND=local (tests, `wrangler dev`) or STAGING=on.
 * Anywhere else its presence is a mistake worth stopping every call for.
 */
function checkTestApprovalKey(env: Env): void {
  if (env.TEST_APPROVAL_KEY === undefined || env.TEST_APPROVAL_KEY === "") return;
  if (env.DB_BACKEND !== "local" && env.STAGING !== "on") {
    throw misconfigured("TEST_APPROVAL_KEY is for DB_BACKEND=local (tests) or STAGING=on only.");
  }
}

export function readConfig(env: Env): Config {
  checkTestApprovalKey(env);
  // The event id becomes part of every R2 key, so it is restricted to a safe alphabet.
  if (typeof env.EVENT_ID !== "string" || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(env.EVENT_ID)) {
    throw misconfigured("EVENT_ID must be lowercase letters, digits and dashes.");
  }
  return {
    eventId: env.EVENT_ID,
    runSeconds: testOnlySeconds(env, "RUN_SECONDS") ?? positiveNumber("RUN_HOURS", env.RUN_HOURS, 168, "hours", 24 * 365) * 3600,
    abandonAfterSeconds: testOnlySeconds(env, "ABANDON_AFTER_SECONDS") ?? ABANDON_AFTER_SECONDS,
    githubCheck: env.GITHUB_CHECK === "on",
  };
}

let cachedSigner: { seed: string; signer: Signer } | undefined;

async function getSigner(env: Env): Promise<Signer> {
  if (typeof env.RECEIPT_SIGNING_KEY !== "string") throw misconfigured("RECEIPT_SIGNING_KEY is not set.");
  if (cachedSigner?.seed === env.RECEIPT_SIGNING_KEY) return cachedSigner.signer;
  try {
    const signer = await Signer.fromSeedHex(env.RECEIPT_SIGNING_KEY);
    cachedSigner = { seed: env.RECEIPT_SIGNING_KEY, signer };
    return signer;
  } catch {
    throw misconfigured("RECEIPT_SIGNING_KEY is not a 64-hex-character Ed25519 seed.");
  }
}

export async function getDb(env: Env): Promise<Db> {
  // Production is the default. `local` must be asked for by name.
  const backend = env.DB_BACKEND ?? "supabase";
  try {
    if (backend === "supabase") return new SupabaseDb(env.SUPABASE_URL, env.SUPABASE_KEY);
    if (backend === "local") return LocalDb.create(env.TRACES, readConfig(env).abandonAfterSeconds);
  } catch (e) {
    // The detail goes to the Worker's log only: a caller without a credential gets this answer too,
    // and a configuration error must not echo configuration.
    console.error(`misconfigured: ${e instanceof Error ? e.message : String(e)}`);
    throw misconfigured("The database is not configured correctly; the Worker's log says how.");
  }
  console.error(`misconfigured: DB_BACKEND must be "supabase" or "local", not "${backend}"`);
  throw misconfigured("The database is not configured correctly; the Worker's log says how.");
}

export async function makeCtx(env: Env): Promise<Ctx> {
  const cfg = readConfig(env);
  const signer = await getSigner(env);
  const db = await getDb(env);
  return { env, db, signer, cfg, now: () => new Date(Math.floor(Date.now() / 1000) * 1000) };
}
