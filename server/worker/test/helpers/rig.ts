// The Worker in this process on a clock the test moves, and the calls a lifecycle test makes
// again and again: a credential, an open, a heartbeat, a continue, and an entry the server holds whole.

import assert from "node:assert/strict";
import { test } from "node:test";
import { Client, GENESIS, credentialFor, openBody, sha256 } from "./client.ts";
import { type InProcess, inProcess } from "./inproc.ts";

export const T0 = "2026-11-06T09:00:00Z";
/** A start window from 08:00 to 10:00 on the day of T0. */
export const WINDOW = { START_WINDOW_OPENS: "2026-11-06T08:00:00Z", START_WINDOW_CLOSES: "2026-11-06T10:00:00Z" };

export interface Opened {
  run_id: string;
  entry_id: string;
  official_start: string;
  deadline_at: string;
}

/** The Worker, a clock, and the calls a test makes again and again. */
export function rig(vars: Record<string, string> = WINDOW, start = T0, cf?: Record<string, unknown>) {
  const p: InProcess = inProcess(start, vars, cf);
  const api: Client = p.api;
  const cred = (member: string, mode: "competition" | "practice" = "competition") => credentialFor(api, member, mode);
  const open = (token: string, session: string, extra: Record<string, unknown> = {}, headers: Record<string, string> = {}) =>
    api.call("POST", "/v1/runs", { token, json: openBody({ session_id: session, ...extra }), headers });
  const start_ = async (token: string, session: string): Promise<Opened> => {
    const r = await open(token, session);
    assert.equal(r.status, 201, JSON.stringify(r.body));
    return r.body as Opened;
  };
  const beat = (token: string, run: string, entry: string, over: Record<string, unknown> = {}, headers: Record<string, string> = {}) =>
    api.call("POST", `/v1/runs/${run}/heartbeat`, {
      token,
      headers,
      json: { entry_id: entry, seq: 0, head: GENESIS, offset_local: 0, final: false, repo_head: null, ...over },
    });
  const get = (token: string, run: string, entry?: string) => api.call("GET", `/v1/runs/${run}${entry === undefined ? "" : `?entry_id=${entry}`}`, { token });
  const cont = (token: string, run: string, previous: string, session: string, extra: Record<string, unknown> = {}, headers: Record<string, string> = {}) =>
    api.call("POST", `/v1/runs/${run}/continue`, {
      token,
      headers,
      json: {
        previous_entry_id: previous,
        connector_id: "other-cli",
        connector_sha256: "1".repeat(64),
        workspace: "/w/two",
        session_id: session,
        harness_version: "9",
        proxy_version: "0.2.0",
        ...extra,
      },
    });
  /** The server holds records 0..4 of this entry (put in through the database, as a chunk upload would). */
  const hold = async (member: string, run: string, entry: string): Promise<string> => {
    const head = sha256(`${entry}:4`);
    const chunk = {
      entry_id: entry, offset: 0, length: 10, sha256: "1".repeat(64), first_seq: 0, last_seq: 4, prev: GENESIS, head, r2_key: "k",
      received_at: T0,
    };
    assert.equal(await p.db().commitChunk({ member_id: member, run_id: run, chunk, stored_bytes: 10 }), "committed");
    return head;
  };
  /** Commit the end of an entry that `hold` filled: the server then holds it whole and seals it. The answer. */
  const finish = async (token: string, run: string, entry: string, head: string) => {
    const f = await beat(token, run, entry, { seq: 4, head, final: true });
    assert.equal(f.status, 200, JSON.stringify(f.body));
    return f.body as { stop: boolean; sealed: boolean };
  };
  const storedRun = (run: string) => JSON.parse(p.bucket.map.get(`state/runs/${run}.json`)!.text) as Record<string, unknown>;
  return { ...p, cred, open, start: start_, beat, get, cont, hold, finish, storedRun };
}
export type Rig = ReturnType<typeof rig>;

export function lifecycle(name: string, fn: (r: Rig) => Promise<void>, vars?: Record<string, string>, start?: string, cf?: Record<string, unknown>) {
  test(name, async () => {
    const r = rig(vars, start, cf);
    try {
      await fn(r);
    } finally {
      r.stop();
    }
  });
}
