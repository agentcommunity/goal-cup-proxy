// Per-request context: parsed configuration, the database, the signer.

import type { Db } from "./db.ts";
import { LocalDb } from "./db-local.ts";
import { SupabaseDb } from "./db-supabase.ts";
import type { Env } from "./env.ts";
import { ApiError } from "./errors.ts";
import { Signer } from "./sign.ts";

export interface Config {
  eventId: string;
  runHours: number;
  graceHours: number;
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

function positiveNumber(name: string, raw: string | undefined, fallback: number, allowZero = false): number {
  if (raw === undefined || raw === "") return fallback;
  const n = Number(raw);
  if (!Number.isFinite(n) || n < 0 || (n === 0 && !allowZero) || n > 24 * 365) {
    throw misconfigured(`${name} must be a number of hours.`);
  }
  return n;
}

export function readConfig(env: Env): Config {
  // The event id becomes part of every R2 key, so it is restricted to a safe alphabet.
  if (typeof env.EVENT_ID !== "string" || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(env.EVENT_ID)) {
    throw misconfigured("EVENT_ID must be lowercase letters, digits and dashes.");
  }
  return {
    eventId: env.EVENT_ID,
    runHours: positiveNumber("RUN_HOURS", env.RUN_HOURS, 168),
    graceHours: positiveNumber("GRACE_HOURS", env.GRACE_HOURS, 48, true),
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
    if (backend === "local") return await LocalDb.create(env.TRACES, env.DEV_TOKENS);
  } catch (e) {
    // The detail goes to the Worker's log only: a caller without a token gets this answer too,
    // and a configuration error must not echo configuration (DEV_TOKENS holds tokens).
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
