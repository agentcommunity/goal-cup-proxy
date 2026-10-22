// Per-request context: parsed configuration, the database, the signer.

import { CLOSE_AFTER_SECONDS, type Db, type StartWindow } from "./db.ts";
import { LocalDb } from "./db-local.ts";
import { SupabaseDb } from "./db-supabase.ts";
import type { Env } from "./env.ts";
import { ApiError } from "./errors.ts";
import { Signer } from "./sign.ts";
import { isTime, parseSafeUint } from "./util.ts";

/** The default per-run size limit: 64 GiB of compressed chunks. */
export const DEFAULT_RUN_MAX_COMPRESSED_BYTES = 68_719_476_736;

export interface Config {
  /**
   * The events a new competition run may belong to, in order (EVENT_ID, 1 to 4 ids). A start goes
   * to the first one whose start window (in the database) contains it. A run keeps its event.
   */
  eventIds: string[];
  /** The event of practice runs. */
  practiceEventId: string;
  /** The event of organizer test runs (TEST_EVENT_ID); undefined: no test runs here. */
  testEventId: string | undefined;
  /** deadline_at = official_start + this. RUN_HOURS * 3600, or RUN_SECONDS (local backend only). */
  runSeconds: number;
  /** The same for a practice run: PRACTICE_RUN_MINUTES * 60. */
  practiceRunSeconds: number;
  /** A run is closed this long after it ends. 24 h, or CLOSE_AFTER_SECONDS (local backend only). */
  closeAfterSeconds: number;
  /** The most compressed bytes one run may hold in R2 (RUN_MAX_COMPRESSED_BYTES). */
  runMaxCompressedBytes: number;
  /** Each competition event's start window, by id (local backend only); undefined: the database's, or always open. */
  startWindows: Record<string, StartWindow> | undefined;
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

/** RUN_MAX_COMPRESSED_BYTES: a positive whole number of bytes, or the default when unset. */
function byteLimit(raw: string | undefined): number {
  if (raw === undefined || raw === "") return DEFAULT_RUN_MAX_COMPRESSED_BYTES;
  const n = parseSafeUint(raw);
  if (n === null || n === 0) throw misconfigured("RUN_MAX_COMPRESSED_BYTES must be a positive whole number of bytes.");
  return n;
}

/**
 * RUN_SECONDS, CLOSE_AFTER_SECONDS and the START_WINDOW_* edges exist so tests can run a whole
 * lifecycle in seconds. They are honoured only with DB_BACKEND=local; anywhere else their
 * presence is a mistake worth stopping for, not a value to ignore.
 */
function testOnly(env: Env, name: "RUN_SECONDS" | "CLOSE_AFTER_SECONDS" | "START_WINDOW_OPENS" | "START_WINDOW_CLOSES"): string | undefined {
  const raw = env[name];
  if (raw === undefined || raw === "") return undefined;
  if (env.DB_BACKEND !== "local") throw misconfigured(`${name} is for DB_BACKEND=local (tests) only.`);
  return raw;
}

const testOnlySeconds = (env: Env, name: "RUN_SECONDS" | "CLOSE_AFTER_SECONDS"): number | undefined => {
  const raw = testOnly(env, name);
  return raw === undefined ? undefined : positiveNumber(name, raw, 0, "seconds", 365 * 24 * 3600);
};

/**
 * One edge of the start window for each event in `eventIds`, in order. One time is every event's
 * edge; a comma-separated list gives one per event (an empty item is an open edge).
 */
function testOnlyTimes(env: Env, name: "START_WINDOW_OPENS" | "START_WINDOW_CLOSES", eventIds: string[]): Array<string | null> {
  const raw = testOnly(env, name);
  if (raw === undefined) return eventIds.map(() => null);
  const items = raw.split(",");
  if (items.length !== 1 && items.length !== eventIds.length) throw misconfigured(`${name} must be one time, or one per EVENT_ID event.`);
  for (const item of items) {
    if (item !== "" && !isTime(item)) throw misconfigured(`${name} must be an RFC 3339 time like 2026-11-06T09:00:00Z.`);
  }
  return eventIds.map((_, i) => {
    const item = items.length === 1 ? items[0]! : items[i]!;
    return item === "" ? null : item;
  });
}

/** The event id becomes part of every R2 key, so it is restricted to a safe alphabet. */
const EVENT_ID_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/;
/** EVENT_ID lists at most this many events: the dry run and the cup need two. */
const MAX_EVENT_IDS = 4;

/** EVENT_ID: one event id, or up to 4 different ones separated by commas, in the order to try. */
function eventIds(raw: unknown): string[] {
  const ids = typeof raw === "string" ? raw.split(",") : [];
  if (ids.length === 0 || ids.length > MAX_EVENT_IDS || !ids.every((id) => EVENT_ID_PATTERN.test(id)) || new Set(ids).size !== ids.length) {
    throw misconfigured(
      `EVENT_ID must be one event id, or up to ${MAX_EVENT_IDS} different ids separated by commas (no spaces); each is lowercase letters, digits and dashes.`,
    );
  }
  return ids;
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
  const ids = eventIds(env.EVENT_ID);
  if (typeof env.PRACTICE_EVENT_ID !== "string" || !EVENT_ID_PATTERN.test(env.PRACTICE_EVENT_ID)) {
    throw misconfigured("PRACTICE_EVENT_ID must be lowercase letters, digits and dashes.");
  }
  if (env.TEST_EVENT_ID !== undefined && !EVENT_ID_PATTERN.test(env.TEST_EVENT_ID)) {
    throw misconfigured("TEST_EVENT_ID must be lowercase letters, digits and dashes.");
  }
  const opens = testOnlyTimes(env, "START_WINDOW_OPENS", ids);
  const closes = testOnlyTimes(env, "START_WINDOW_CLOSES", ids);
  const windowed = [...opens, ...closes].some((t) => t !== null);
  return {
    eventIds: ids,
    practiceEventId: env.PRACTICE_EVENT_ID,
    testEventId: env.TEST_EVENT_ID === "" ? undefined : env.TEST_EVENT_ID,
    runSeconds: testOnlySeconds(env, "RUN_SECONDS") ?? positiveNumber("RUN_HOURS", env.RUN_HOURS, 168, "hours", 24 * 365) * 3600,
    practiceRunSeconds: positiveNumber("PRACTICE_RUN_MINUTES", env.PRACTICE_RUN_MINUTES, 10, "minutes", 24 * 60) * 60,
    closeAfterSeconds: testOnlySeconds(env, "CLOSE_AFTER_SECONDS") ?? CLOSE_AFTER_SECONDS,
    runMaxCompressedBytes: byteLimit(env.RUN_MAX_COMPRESSED_BYTES),
    startWindows: windowed ? Object.fromEntries(ids.map((id, i) => [id, { opens: opens[i]!, closes: closes[i]! }])) : undefined,
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
    if (backend === "local") {
      const cfg = readConfig(env);
      return LocalDb.create(env.TRACES, { closeAfterSeconds: cfg.closeAfterSeconds, startWindows: cfg.startWindows });
    }
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
