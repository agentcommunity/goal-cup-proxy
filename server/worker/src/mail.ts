// The run emails: each notice the cron queued is sent once, through Resend.
//
// The database (the website's migration) decides what each email says, who it goes to (the
// member's account address) and whether that address is suppressed; it hands this file
// claimed notices. This file only carries them to Resend and records the outcome:
//
//   claimNotices -> POST https://api.resend.com/emails (Idempotency-Key goal-cup-notice/<id>) -> finishNotice
//
// A claim lasts 15 minutes and the idempotency key holds for Resend's 24 hours, so a tick that
// dies half way never sends a notice twice. Mail is off while RESEND_API_KEY is unset. A staging
// Worker sends every notice to NOTICE_RECIPIENT_OVERRIDE and refuses to send without it, so a test
// approval "as any member" can never email a real member. Addresses are never logged.

import type { Ctx } from "./context.ts";
import { formatTime } from "./util.ts";

/** At most this many emails per tick (each is one subrequest, plus two database calls). */
export const NOTICES_PER_TICK = 20;
const RESEND_URL = "https://api.resend.com/emails";

const resend = (key: string, body: Record<string, unknown>, headers: Record<string, string> = {}) =>
  fetch(RESEND_URL, {
    method: "POST",
    headers: { authorization: `Bearer ${key}`, "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(10_000),
  });

/** Mail on (RESEND_API_KEY set): the key, the sender and the override. Throws when the sender is missing or a staging Worker has no override. */
function mailer(ctx: Ctx): { key: string; from: string; override: string | null } | null {
  const key = ctx.env.RESEND_API_KEY;
  if (key === undefined || key === "") return null;
  const from = ctx.env.NOTICE_FROM ?? "";
  if (from === "") throw new Error("RESEND_API_KEY is set but NOTICE_FROM is not");
  const override = (ctx.env.NOTICE_RECIPIENT_OVERRIDE ?? "").trim() || null;
  if (ctx.env.STAGING === "on" && override === null) {
    throw new Error("a staging Worker sends notices only to NOTICE_RECIPIENT_OVERRIDE, which is not set");
  }
  return { key, from, override };
}

/**
 * Mail the owner (ALERT_EMAIL; NOTICE_RECIPIENT_OVERRIDE when set, always on staging). Returns false
 * when mail is off or ALERT_EMAIL is unset; throws when the send fails. The address is never logged.
 */
export async function sendAlert(ctx: Ctx, subject: string, text: string): Promise<boolean> {
  const alertTo = (ctx.env.ALERT_EMAIL ?? "").trim();
  const m = alertTo === "" ? null : mailer(ctx);
  if (m === null) return false;
  const res = await resend(m.key, { from: m.from, to: [m.override ?? alertTo], subject: m.override === null ? subject : `[staging] ${subject}`, text });
  if (!res.ok) throw new Error(`alert: resend HTTP ${res.status}`);
  return true;
}

/** Send the notices that are due. Returns how many were sent by this call. */
export async function sendNotices(ctx: Ctx, now: Date): Promise<number> {
  const m = mailer(ctx);
  if (m === null) return 0;
  const { key, from, override } = m;

  const at = formatTime(now);
  let sent = 0;
  for (const n of await ctx.db.claimNotices(NOTICES_PER_TICK, at)) {
    // Staging: always the test address (the database gives staging no address anyway).
    const to = ctx.env.STAGING === "on" ? override : (override ?? n.to);
    if (to === null) {
      await ctx.db.finishNotice(n.id, "skipped", null, "no_address", at);
      continue;
    }
    let res: Response;
    try {
      res = await resend(key, { from, to: [to], subject: override === null ? n.subject : `[staging] ${n.subject}`, text: n.text }, { "idempotency-key": `goal-cup-notice/${n.id}` });
    } catch (e) {
      console.error(`mail notice ${n.id}: request failed: ${String(e).slice(0, 200)}`);
      await ctx.db.finishNotice(n.id, "failed", null, "request failed", at);
      continue;
    }
    if (!res.ok) {
      console.error(`mail notice ${n.id}: resend HTTP ${res.status}`);
      await ctx.db.finishNotice(n.id, "failed", null, `resend HTTP ${res.status}`, at);
      continue;
    }
    let id = "";
    try {
      const body = (await res.json()) as { id?: unknown };
      if (typeof body.id === "string") id = body.id.slice(0, 200);
    } catch {
      // sent all the same; the provider id is only for tracing
    }
    await ctx.db.finishNotice(n.id, "sent", to, id, at);
    sent++;
  }
  return sent;
}
