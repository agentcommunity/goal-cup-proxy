// The run emails (src/mail.ts) against a fake database and a stubbed Resend, and SupabaseDb's two
// mailer calls against a stubbed fetch. No Resend and no Supabase here.

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import type { Ctx } from "../src/context.ts";
import type { ClaimedNotice, NoticeOutcome } from "../src/db.ts";
import { LocalDb } from "../src/db-local.ts";
import { SupabaseDb } from "../src/db-supabase.ts";
import type { Env } from "../src/env.ts";
import { sendNotices } from "../src/mail.ts";
import { FakeBucket, asR2 } from "./helpers/fake-r2.ts";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

const NOW = new Date("2026-10-15T12:00:00Z");
const notice = (id: number, to: string | null): ClaimedNotice => ({ id, kind: "started", run_id: "r0000000000000001", to, subject: "Goal Cup: your agent started", text: "body" });

function rig(env: Partial<Env>, claimed: ClaimedNotice[]) {
  const finished: { id: number; outcome: NoticeOutcome; to: string | null; detail: string }[] = [];
  let claims = 0;
  const db = {
    claimNotices: async (limit: number) => {
      claims++;
      assert.ok(limit > 0);
      return claimed;
    },
    finishNotice: async (id: number, outcome: NoticeOutcome, to: string | null, detail: string) => void finished.push({ id, outcome, to, detail }),
  };
  const ctx = { env: env as Env, db, now: () => NOW } as unknown as Ctx;
  return { ctx, finished, claims: () => claims };
}

function stubResend(reply: (body: Record<string, unknown>) => Response) {
  const sent: { headers: Record<string, string>; body: Record<string, unknown> }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    assert.equal(String(input), "https://api.resend.com/emails");
    const body = JSON.parse(init?.body as string) as Record<string, unknown>;
    sent.push({ headers: init?.headers as Record<string, string>, body });
    return reply(body);
  }) as typeof fetch;
  return sent;
}

test("mail is off without RESEND_API_KEY: nothing is claimed", async () => {
  const r = rig({}, [notice(1, "a@example.test")]);
  assert.equal(await sendNotices(r.ctx, NOW), 0);
  assert.equal(r.claims(), 0);
});

test("a staging Worker refuses to send without a recipient override", async () => {
  const r = rig({ RESEND_API_KEY: "re_test", NOTICE_FROM: "AC <a@example.test>", STAGING: "on" }, [notice(1, "member@example.test")]);
  await assert.rejects(sendNotices(r.ctx, NOW), /NOTICE_RECIPIENT_OVERRIDE/);
  assert.equal(r.claims(), 0);
});

test("each claimed notice goes once to the override, keyed for Resend's idempotency, and is recorded", async () => {
  const r = rig({ RESEND_API_KEY: "re_test", NOTICE_FROM: "AC <a@example.test>", STAGING: "on", NOTICE_RECIPIENT_OVERRIDE: "delivered@resend.dev" }, [notice(7, "member@example.test"), notice(8, null)]);
  const sent = stubResend(() => new Response(JSON.stringify({ id: "msg_1" }), { status: 200 }));
  assert.equal(await sendNotices(r.ctx, NOW), 2);
  assert.deepEqual(sent.map((s) => s.body.to), [["delivered@resend.dev"], ["delivered@resend.dev"]]);
  assert.equal(sent[0]!.headers["idempotency-key"], "goal-cup-notice/7");
  assert.equal(sent[0]!.body.subject, "[staging] Goal Cup: your agent started");
  assert.deepEqual(r.finished, [
    { id: 7, outcome: "sent", to: "delivered@resend.dev", detail: "msg_1" },
    { id: 8, outcome: "sent", to: "delivered@resend.dev", detail: "msg_1" },
  ]);
});

test("without an override a notice goes to its member; no address is skipped; a refusal is a failure to retry", async () => {
  const r = rig({ RESEND_API_KEY: "re_test", NOTICE_FROM: "AC <a@example.test>" }, [notice(1, "member@example.test"), notice(2, null), notice(3, "other@example.test")]);
  const sent = stubResend((body) => ((body.to as string[])[0] === "other@example.test" ? new Response("{}", { status: 500 }) : new Response('{"id":"m"}', { status: 200 })));
  assert.equal(await sendNotices(r.ctx, NOW), 1);
  assert.deepEqual(sent.map((s) => [s.body.to, s.body.subject]), [[["member@example.test"], "Goal Cup: your agent started"], [["other@example.test"], "Goal Cup: your agent started"]]);
  assert.deepEqual(r.finished.map((f) => [f.id, f.outcome, f.detail]), [[1, "sent", "m"], [2, "skipped", "no_address"], [3, "failed", "resend HTTP 500"]]);
});

test("SupabaseDb: claimNotices -> goal_cup_upload_claim_notices, finishNotice -> goal_cup_upload_finish_notice", async () => {
  const calls: { url: string; body: unknown }[] = [];
  const replies: unknown[] = [[notice(5, "m@example.test")], "ok", [{ id: 6 }]];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(input), body: JSON.parse(init?.body as string) });
    return new Response(JSON.stringify(replies.shift()), { status: 200 });
  }) as typeof fetch;
  const db = new SupabaseDb("https://p.example", "k");
  assert.deepEqual(await db.claimNotices(20, "2026-10-15T12:00:00Z"), [notice(5, "m@example.test")]);
  await db.finishNotice(5, "sent", "m@example.test", "msg", "2026-10-15T12:00:01Z");
  assert.deepEqual(calls.map((c) => c.url.slice("https://p.example/rest/v1/rpc/".length)), ["goal_cup_upload_claim_notices", "goal_cup_upload_finish_notice"]);
  assert.deepEqual(calls[1]!.body, { p_id: 5, p_outcome: "sent", p_to: "m@example.test", p_detail: "msg", p_now: "2026-10-15T12:00:01Z" });
  await assert.rejects(db.claimNotices(20, "2026-10-15T12:00:00Z")); // a row without its fields is a 502
});

test("LocalDb: a queued notice is claimed once, again after 15 minutes, and never after it is sent", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const at = "2026-10-15T12:00:00Z";
  await db.queueNotices([{ run_id: "r0000000000000001", member_id: "m", kind: "started", dedupe_key: "", payload: { repo_url: "x" }, at }]);
  const first = await db.claimNotices(10, at);
  assert.equal(first.length, 1);
  assert.equal(first[0]!.to, null);
  assert.deepEqual(await db.claimNotices(10, "2026-10-15T12:10:00Z"), []);
  const again = await db.claimNotices(10, "2026-10-15T12:16:00Z");
  assert.deepEqual(again.map((n) => n.id), [first[0]!.id]);
  await db.finishNotice(first[0]!.id, "sent", "delivered@resend.dev", "msg", "2026-10-15T12:16:01Z");
  assert.deepEqual(await db.claimNotices(10, "2026-10-15T13:00:00Z"), []);
});
