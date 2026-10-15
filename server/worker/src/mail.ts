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

/** Send the notices that are due. Returns how many were sent by this call. */
export async function sendNotices(ctx: Ctx, now: Date): Promise<number> {
  const key = ctx.env.RESEND_API_KEY;
  if (key === undefined || key === "") return 0;
  const from = ctx.env.NOTICE_FROM ?? "";
  if (from === "") throw new Error("RESEND_API_KEY is set but NOTICE_FROM is not");
  const override = (ctx.env.NOTICE_RECIPIENT_OVERRIDE ?? "").trim() || null;
  if (ctx.env.STAGING === "on" && override === null) {
    throw new Error("a staging Worker sends notices only to NOTICE_RECIPIENT_OVERRIDE, which is not set");
  }

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
      res = await fetch(RESEND_URL, {
        method: "POST",
        headers: { authorization: `Bearer ${key}`, "content-type": "application/json", "idempotency-key": `goal-cup-notice/${n.id}` },
        body: JSON.stringify({ from, to: [to], subject: override === null ? n.subject : `[staging] ${n.subject}`, text: n.text }),
        signal: AbortSignal.timeout(10_000),
      });
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
