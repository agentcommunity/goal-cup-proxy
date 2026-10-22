// The public live totals (docs/UPLOAD-API.md, `GET /v1/live.json`). The 5-minute cron asks the
// database once for the event's community totals and writes them to R2; the GET serves that copy and
// never calls the database. Design: goal-cup-proxy PR #241 (live stats), step (a).
//
// What reaches R2 is rebuilt here key by key from what the database answered, so a field the SQL
// might add later (or a mistake that put a run id in it) is never published: only the keys below exist.

import type { Ctx } from "./context.ts";
import { LIVE_MIN_GROUP, type LiveNumber, type LiveSnapshot } from "./db.ts";
import { ApiError } from "./errors.ts";
import { formatTime, isTime } from "./util.ts";

/** Where the cron keeps the event's current snapshot, in the TRACES bucket (outside traces/, so outside the bucket lock). */
export const liveKey = (eventId: string) => `live/${eventId}.json`;
/** Browsers and the website may keep a copy this long; the cron writes a new one every 5 minutes. */
export const LIVE_MAX_AGE = 60;

const ENTRY_KEYS = ["started", "running", "working", "quiet", "finished", "sealed", "incomplete"] as const;
const WORK_KEYS = ["agent_hours", "working_hours", "model_calls", "model_calls_last_hour", "restarts", "data_bytes"] as const;
const NOTICE_KEYS = ["silence", "idle_3h", "idle_6h", "disk_full"] as const;
const EVENT_KINDS = ["competition", "dry_run", "practice", "test"];
/** A connector id as a public bucket name; anything else is the database's mistake. */
const HARNESS_ID = /^[a-z0-9][a-z0-9-]{0,63}$/;

const obj = (v: unknown): Record<string, unknown> => {
  if (typeof v !== "object" || v === null || Array.isArray(v)) throw new Error("not an object");
  return v as Record<string, unknown>;
};

/** A count or sum, or null. Over LIVE_MIN_GROUP or null is checked where the number is made (the SQL, `LocalDb`). */
function num(v: unknown, what: string, whole = true): LiveNumber {
  if (v === null) return null;
  if (typeof v === "number" && Number.isFinite(v) && v >= 0 && (!whole || Number.isSafeInteger(v))) return v;
  throw new Error(`${what} is not a count`);
}

/** The published document: `raw` (what the database answered) rebuilt from the known keys only; throws on any wrong shape. */
export function publicSnapshot(raw: unknown): LiveSnapshot {
  const d = obj(raw);
  if (d.schema_version !== 1) throw new Error("schema_version is not 1");
  const ev = obj(d.event);
  if (typeof ev.id !== "string" || typeof ev.kind !== "string" || !EVENT_KINDS.includes(ev.kind)) throw new Error("event");
  for (const k of ["start_window_opens", "start_window_closes"]) if (ev[k] !== null && !isTime(ev[k])) throw new Error(`event.${k}`);
  if (!isTime(d.generated_at)) throw new Error("generated_at");
  const entries = obj(d.entries);
  const work = obj(d.work);
  const notices = obj(d.notices);
  let harnesses: LiveSnapshot["harnesses"] = null;
  if (d.harnesses !== null) {
    if (!Array.isArray(d.harnesses)) throw new Error("harnesses");
    harnesses = d.harnesses.map((h: unknown, i) => {
      const b = obj(h);
      if (typeof b.id !== "string" || !HARNESS_ID.test(b.id)) throw new Error(`harnesses[${i}].id`);
      const started = num(b.started, `harnesses[${i}].started`);
      if (started === null || started < LIVE_MIN_GROUP) throw new Error(`harnesses[${i}] is under the group size`);
      return { id: b.id, started, running: num(b.running, `harnesses[${i}].running`) };
    });
  }
  return {
    schema_version: 1,
    event: {
      id: ev.id,
      kind: ev.kind,
      start_window_opens: (ev.start_window_opens as string | null) ?? null,
      start_window_closes: (ev.start_window_closes as string | null) ?? null,
      run_hours: num(ev.run_hours ?? null, "event.run_hours", false), // a fraction only in tests (RUN_SECONDS)
    },
    generated_at: d.generated_at,
    min_group: LIVE_MIN_GROUP,
    entries: { signed_up: null, ...Object.fromEntries(ENTRY_KEYS.map((k) => [k, num(entries[k] ?? null, `entries.${k}`)])) } as LiveSnapshot["entries"],
    harnesses,
    work: Object.fromEntries(WORK_KEYS.map((k) => [k, num(work[k] ?? null, `work.${k}`, !k.endsWith("_hours"))])) as LiveSnapshot["work"],
    notices: Object.fromEntries(NOTICE_KEYS.map((k) => [k, num(notices[k] ?? null, `notices.${k}`)])) as LiveSnapshot["notices"],
    // Not built yet (later steps of the design): always null here, whatever the database said.
    survival: null,
    hourly: null,
    trace: null,
    github: null,
    server: null,
  };
}

/**
 * The cron's step: write the current event's snapshot to R2. Returns 1 when written, 0 when not. On
 * its own: a failure is one log line (`live-snapshot-failed`), and nothing else waits on it.
 */
export async function writeLiveSnapshot(ctx: Ctx, now: Date): Promise<number> {
  try {
    const raw = await ctx.db.liveSnapshot(ctx.cfg.eventId, formatTime(now));
    if (raw === null) {
      console.error(`live-snapshot-failed: the database has no event ${ctx.cfg.eventId} for this deployment`);
      return 0;
    }
    const doc = publicSnapshot(raw);
    await ctx.env.TRACES.put(liveKey(ctx.cfg.eventId), JSON.stringify(doc), { httpMetadata: { contentType: "application/json" } });
    return 1;
  } catch (e) {
    console.error(`live-snapshot-failed: ${e instanceof Error ? e.message : String(e)}`);
    return 0;
  }
}

/** Public, no credential: the snapshot as the cron last wrote it. Never a database call. */
export async function serveLive(ctx: Ctx, _request: Request): Promise<Response> {
  const cors = { "access-control-allow-origin": "*" };
  const stored = await ctx.env.TRACES.get(liveKey(ctx.cfg.eventId));
  if (stored === null) throw new ApiError(404, "not_ready", "No live totals yet; the server writes them every 5 minutes.", cors);
  const doc: unknown = await stored.json();
  return new Response(JSON.stringify(doc), {
    status: 200,
    headers: { "content-type": "application/json", "cache-control": `public, max-age=${LIVE_MAX_AGE}`, ...cors },
  });
}
