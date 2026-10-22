// LocalDb: the database for `wrangler dev` and tests ONLY. It keeps its state as JSON objects in
// the same R2 bucket, under state/…, and makes every change atomic with a conditional put
// (the object's etag must still be the one that was read). Production uses SupabaseDb.
//
// Layout:
//   state/runs/{run_id}.json     the run and its entries, with receipts and chunks (a few hundred bytes each)
//   state/open/{member}.json     {"run_id": …}: the member's latest competition run, the one a new start replaces
//                                (only competition replaces; test and practice runs have no slot)
//   state/devices/{sha256}.json  a device request, by the hash of its device code
//   state/usercodes/{sha256}.json  {"device_sha256": …}: the request a user code belongs to
//   state/credentials/{sha256}.json  a run credential, by its own hash
//   state/notices/{run_id}.{kind}.{key}.json  a queued notice (the outbox); `key` is the dedupe key, URL-encoded, "-" when empty
//
// Not built for load: a run's whole state is one object. Fine for a developer and for tests.

import type {
  AbandonResult,
  ApproveDeviceResult,
  ChunkRecord,
  ClaimedNotice,
  CommitChunkArgs,
  CommitChunkResult,
  ContinueRunArgs,
  ContinueRunResult,
  CreateDeviceArgs,
  Credential,
  Db,
  DevicePoll,
  EntryRow,
  FinalCommitDetail,
  HeartbeatArgs,
  HeartbeatResult,
  MachineFacts,
  NoticeKind,
  NoticeOutcome,
  NoticeArgs,
  OpenRunArgs,
  OpenRunResult,
  RunRow,
  RunStatus,
  SealArgs,
  SealResult,
  StoreReceiptResult,
  TickRun,
  Vitals,
} from "./db.ts";
import { CLOSE_AFTER_SECONDS, DISK_NOTICE_GAP_SECONDS, FIRST_SEQ, GENESIS, NO_FINAL_DETAIL, TICK_LOOKBACK_SECONDS, UNBOUND_CREDENTIAL_SECONDS, endsAt, isPastEnd, statusAt } from "./db.ts";
import type { StartReceipt } from "./sign.ts";
import { addSeconds, formatTime, sha256Hex, toHex } from "./util.ts";

interface StoredEntry extends EntryRow {
  harness_version: string;
  tracer_version: string;
  chunks: ChunkRecord[];
  /** Compressed bytes the entry's committed chunks hold in R2, and how many chunks (absent in a state file from before). */
  stored_bytes?: number;
  chunk_count?: number;
  /** The highest `model_requests` a heartbeat of this entry has sent; null until one does. */
  model_requests?: number | null;
  final_repo_head: string | null;
  last_heartbeat: {
    at: string;
    seq: number;
    head: string;
    offset_local: number;
    recording: string | null;
    vitals?: Vitals | null;
    client_ip: string | null;
    client_country: string | null;
  } | null;
  heartbeat_count: number;
}

/** The run and its entries (lowest segment first); `current_*` are worked out when it is read. */
interface StoredRun extends Omit<RunRow, "first_entry_id" | "current_entry_id" | "current_sealed_at" | "stored_bytes" | "chunk_count"> {
  machine: MachineFacts | null;
  client_ip: string | null;
  client_country: string | null;
  /** The latest pair, from a heartbeat or a continue; `client_*` stay the open call's. */
  last_client_ip?: string | null;
  last_client_country?: string | null;
  abandon_reason: string | null;
  sealed_at: string | null;
  // What the heartbeats and the deadline add, for the cron. (A state file from before has none of these.)
  last_heartbeat_at?: string | null;
  last_model_request_at?: string | null;
  flagged_at?: string | null;
  flag_reason?: "gaps" | "failed" | null;
  final_commit?: string | null;
  final_commit_at?: string | null;
  /** How the judged commit was found (`FinalCommitDetail`). */
  final_pushed_at?: string | null;
  final_branch?: string | null;
  final_late_push?: boolean | null;
  final_force_push?: boolean | null;
  final_head_fallback?: boolean | null;
  /** The latest vitals a heartbeat sent; the start of the running disk pause; the start of the latest pause owed a disk_full email. */
  last_vitals?: Vitals | null;
  disk_paused_at?: string | null;
  disk_notice_at?: string | null;
  entries: StoredEntry[];
}

interface StoredDevice extends CreateDeviceArgs {
  status: "pending" | "approved" | "denied";
  member_id: string | null;
  decided_at: string | null;
  picked_up: boolean;
}

const MAX_CAS_TRIES = 10;

/** What `wrangler dev` and the tests can set; production's equivalents are in the database. */
export interface LocalOptions {
  /** Seconds after a run ends until it is closed (default 24 hours). Tests only. */
  closeAfterSeconds?: number;
  /** The competition start window; an unset edge is open. Unset: always open. */
  startWindow?: { opens: string | null; closes: string | null };
}

const BEFORE_WINDOW = "The start window has not opened yet.";
const AFTER_WINDOW = "after the start window you can only `goalcup continue` your run";

const entryOf = (run: StoredRun, entryId: string): StoredEntry | undefined => run.entries.find((e) => e.entry_id === entryId);
const currentOf = (run: StoredRun): StoredEntry => run.entries[run.entries.length - 1]!;
const withEntry = (run: StoredRun, entry: StoredEntry): StoredRun => ({
  ...run,
  entries: run.entries.map((e) => (e.entry_id === entry.entry_id ? entry : e)),
});

/** A queued notice as stored under state/notices/, with what the mailer adds. */
interface LocalNotice {
  run_id: string;
  member_id: string;
  kind: NoticeKind;
  dedupe_key: string;
  payload: Record<string, unknown>;
  created_at: string;
  sent_at: string | null;
  claimed_at?: string;
  sent_to?: string | null;
  provider_id?: string;
  skipped_at?: string;
  skipped_reason?: string;
  last_error?: string;
}

/** How long a mailer's claim on a notice lasts (the same 15 minutes as the database's). */
const NOTICE_CLAIM_SECONDS = 15 * 60;

/** A notice's id locally: a 32-bit FNV-1a hash of its key (Supabase has a real identity column). */
function noticeId(key: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < key.length; i++) h = Math.imul(h ^ key.charCodeAt(i), 0x01000193) >>> 0;
  return h;
}

export class LocalDb implements Db {
  readonly #bucket: R2Bucket;
  readonly #closeAfterSeconds: number;
  readonly #window: LocalOptions["startWindow"];

  private constructor(bucket: R2Bucket, options: LocalOptions) {
    this.#bucket = bucket;
    this.#closeAfterSeconds = options.closeAfterSeconds ?? CLOSE_AFTER_SECONDS;
    this.#window = options.startWindow;
  }

  static create(bucket: R2Bucket, options: LocalOptions = {}): LocalDb {
    return new LocalDb(bucket, options);
  }

  // ---- plumbing --------------------------------------------------------------------------

  #runKey = (runId: string) => `state/runs/${runId}.json`;
  #openKey = (member: string) => `state/open/${encodeURIComponent(member)}.json`;

  async #readRun(runId: string): Promise<{ run: StoredRun; etag: string } | null> {
    const obj = await this.#bucket.get(this.#runKey(runId));
    if (obj === null) return null;
    return { run: (await obj.json()) as StoredRun, etag: obj.etag };
  }

  /**
   * Read-modify-write one run atomically. `change` returns the new state plus the answer, or
   * just an answer (no write). If someone else wrote in between, the put is refused and we start over.
   */
  async #updateRun<T>(
    runId: string,
    memberId: string,
    notFound: T,
    change: (run: StoredRun) => { next?: StoredRun; answer: T },
  ): Promise<T> {
    for (let i = 0; i < MAX_CAS_TRIES; i++) {
      const cur = await this.#readRun(runId);
      if (cur === null || cur.run.member_id !== memberId) return notFound;
      const { next, answer } = change(cur.run);
      if (next === undefined) return answer;
      const put = await this.#bucket.put(this.#runKey(runId), JSON.stringify(next), {
        onlyIf: { etagMatches: cur.etag },
      });
      if (put !== null) return answer;
    }
    throw new Error("local state is too contended; try again");
  }

  /** Read-modify-write one JSON object atomically (the same etag rule as `#updateRun`). */
  async #updateObject<T extends object, A>(key: string, missing: A, change: (cur: T) => { next?: T; answer: A }): Promise<A> {
    for (let i = 0; i < MAX_CAS_TRIES; i++) {
      const obj = await this.#bucket.get(key);
      if (obj === null) return missing;
      const { next, answer } = change((await obj.json()) as T);
      if (next === undefined) return answer;
      if ((await this.#bucket.put(key, JSON.stringify(next), { onlyIf: { etagMatches: obj.etag } })) !== null) return answer;
    }
    throw new Error("local state is too contended; try again");
  }

  #deviceKey = (sha: string) => `state/devices/${sha}.json`;
  #userCodeKey = (sha: string) => `state/usercodes/${sha}.json`;
  #credentialKey = (sha: string) => `state/credentials/${sha}.json`;

  // ---- Db: device flow and credentials ---------------------------------------------------

  async createDevice(a: CreateDeviceArgs): Promise<void> {
    const dev: StoredDevice = { ...a, status: "pending", member_id: null, decided_at: null, picked_up: false };
    const create = { onlyIf: { etagDoesNotMatch: "*" } };
    if ((await this.#bucket.put(this.#deviceKey(a.device_sha256), JSON.stringify(dev), create)) === null) throw new Error("device code collision");
    if ((await this.#bucket.put(this.#userCodeKey(a.user_sha256), JSON.stringify({ device_sha256: a.device_sha256 }), create)) === null) {
      await this.#bucket.delete(this.#deviceKey(a.device_sha256));
      throw new Error("user code collision");
    }
  }

  async pollDevice(deviceSha256: string, now: string): Promise<DevicePoll> {
    const key = this.#deviceKey(deviceSha256);
    // Mark an approved request picked up first, with a conditional put, so only one poll gets the result.
    const won = await this.#updateObject<StoredDevice, StoredDevice | DevicePoll>(key, { kind: "expired" }, (dev) => {
      if (new Date(now).getTime() >= new Date(dev.expires_at).getTime()) return { answer: { kind: "expired" } };
      if (dev.status === "pending") return { answer: { kind: "pending" } };
      if (dev.status === "denied") return { answer: { kind: "denied" } };
      if (dev.picked_up) return { answer: { kind: "expired" } };
      return { next: { ...dev, picked_up: true }, answer: dev };
    });
    if (!("status" in won)) return won;
    if (won.purpose === "login") return { kind: "approved", purpose: "login", handle: won.member_id ?? "" };

    const plain = "gcr_" + toHex(crypto.getRandomValues(new Uint8Array(32)));
    const cred: Credential = {
      member_id: won.member_id ?? "",
      run_id: null,
      mode: won.mode,
      expires_at: formatTime(addSeconds(new Date(won.decided_at ?? now), UNBOUND_CREDENTIAL_SECONDS)),
    };
    await this.#bucket.put(this.#credentialKey(await sha256Hex(plain)), JSON.stringify(cred), { onlyIf: { etagDoesNotMatch: "*" } });
    return { kind: "approved", purpose: "run", credential_plain: plain, expires_at: cred.expires_at };
  }

  async approveDevice(userCode: string, memberId: string, approve: boolean, now: string): Promise<ApproveDeviceResult> {
    const link = await this.#bucket.get(this.#userCodeKey(await sha256Hex(userCode)));
    if (link === null) return "not_found";
    const { device_sha256 } = (await link.json()) as { device_sha256: string };
    return this.#updateObject<StoredDevice, ApproveDeviceResult>(this.#deviceKey(device_sha256), "not_found", (dev) => {
      if (new Date(now).getTime() >= new Date(dev.expires_at).getTime()) return { answer: "expired" };
      if (dev.status !== "pending") return { answer: "already_decided" };
      return { next: { ...dev, status: approve ? "approved" : "denied", member_id: memberId, decided_at: now }, answer: "ok" };
    });
  }

  async credential(credentialSha256: string, now: string): Promise<Credential | null> {
    const obj = await this.#bucket.get(this.#credentialKey(credentialSha256));
    if (obj === null) return null;
    const cred = (await obj.json()) as Credential;
    return new Date(now).getTime() < new Date(cred.expires_at).getTime() ? cred : null;
  }

  // ---- Db: runs --------------------------------------------------------------------------

  async openRun(a: OpenRunArgs): Promise<OpenRunResult> {
    // Times are RFC 3339 to the second, so they compare as text.
    const w = this.#window;
    if (a.mode === "competition" && w !== undefined) {
      if (w.opens !== null && a.official_start < w.opens) return { kind: "forbidden", message: BEFORE_WINDOW };
      if (w.closes !== null && a.official_start > w.closes) return { kind: "forbidden", message: AFTER_WINDOW };
    }
    // The credential binds first, so one that cannot bind opens no run and leaves nothing behind.
    const bound = await this.#updateObject<Credential, string | null | true>(this.#credentialKey(a.credential), null, (cred) =>
      cred.run_id !== null ? { answer: cred.run_id } : { next: { ...cred, run_id: a.run_id, expires_at: a.credential_expires_at }, answer: true },
    );
    if (bound !== true) {
      // Bound to a run already. If that run's first entry has this session, this is a retry that
      // raced the original open: the same run.
      const held = bound === null ? null : await this.#readRun(bound);
      const first = held?.run.entries[0];
      if (held?.run.member_id === a.member_id && first?.session_id === a.session_id) {
        return { kind: "existing", run: toRun(held.run), entry: toEntry(first) };
      }
      return { kind: "conflict" };
    }

    const run: StoredRun = {
      run_id: a.run_id,
      event_id: a.event_id,
      member_id: a.member_id,
      mode: a.mode,
      status: "open",
      official_start: a.official_start,
      deadline_at: a.deadline_at,
      replaced_at: null,
      repo_url: a.repo_url,
      machine: a.machine,
      client_ip: a.client_ip,
      client_country: a.client_country,
      abandon_reason: null,
      sealed_at: null,
      last_client_ip: null,
      last_client_country: null,
      last_heartbeat_at: null,
      last_model_request_at: null,
      flagged_at: null,
      flag_reason: null,
      final_commit: null,
      final_commit_at: null,
      last_vitals: null,
      disk_paused_at: null,
      disk_notice_at: null,
      entries: [newEntry(a.entry_id, a.run_id, 0, a)],
    };
    await this.#keep(run);
    return { kind: "created", run: toRun(run), entry: toEntry(currentOf(run)) };
  }

  /**
   * Store a new run. A competition run first marks the account's latest competition run replaced,
   * then takes the account's slot with a conditional put; if someone else took it meanwhile, go
   * round again (that run is replaced in turn). Only competition replaces: a test run or a practice
   * run is just stored, so an operator's test runs run side by side.
   */
  async #keep(run: StoredRun): Promise<void> {
    const create = { onlyIf: { etagDoesNotMatch: "*" } };
    if (run.mode !== "competition") {
      await this.#bucket.put(this.#runKey(run.run_id), JSON.stringify(run), create);
      return;
    }
    const openKey = this.#openKey(run.member_id);
    for (let i = 0; i < MAX_CAS_TRIES; i++) {
      const marker = await this.#bucket.get(openKey);
      if (marker !== null) {
        const heldId = ((await marker.json()) as { run_id: string }).run_id;
        await this.#updateRun<null>(heldId, run.member_id, null, (held) =>
          held.mode === "competition" && held.status === "open" && held.replaced_at === null
            ? { next: { ...held, replaced_at: held.official_start > run.official_start ? held.official_start : run.official_start }, answer: null }
            : { answer: null },
        );
      }
      await this.#bucket.put(this.#runKey(run.run_id), JSON.stringify(run), create); // refused (and fine) on a second round
      const took = await this.#bucket.put(
        openKey,
        JSON.stringify({ run_id: run.run_id }),
        marker === null ? create : { onlyIf: { etagMatches: marker.etag } },
      );
      if (took !== null) return;
    }
    throw new Error("local state is too contended; try again");
  }

  continueRun(a: ContinueRunArgs): Promise<ContinueRunResult> {
    return this.#updateRun<ContinueRunResult>(a.run_id, a.member_id, { kind: "not_found" }, (run) => {
      const cur = currentOf(run);
      const previous = run.entries[run.entries.length - 2];
      // The same call again: the current entry came from this session, after that entry.
      if (cur.segment > 0 && cur.session_id === a.session_id && previous?.entry_id === a.previous_entry_id) {
        return { answer: { kind: "existing", run: toRun(run), entry: toEntry(cur) } };
      }
      if (run.status !== "open" || isPastEnd(run, new Date(a.at))) return { answer: { kind: "closed" } };
      if (cur.entry_id !== a.previous_entry_id) return { answer: { kind: "conflict" } };
      const entry = newEntry(a.entry_id, run.run_id, cur.segment + 1, a);
      const next = {
        ...run,
        last_client_ip: a.client_ip ?? run.last_client_ip ?? null,
        last_client_country: a.client_country ?? run.last_client_country ?? null,
        entries: [...run.entries, entry],
      };
      return { next, answer: { kind: "created", run: toRun(next), entry: toEntry(entry) } };
    });
  }

  closeRun(memberId: string, runId: string, at: string): Promise<RunStatus | null> {
    return this.#updateRun<RunStatus | null>(runId, memberId, null, (run) => {
      const status = statusAt(toRun(run), new Date(at), this.#closeAfterSeconds);
      if (status === run.status) return { answer: status };
      return { next: { ...run, status, sealed_at: status === "sealed" ? currentOf(run).sealed_at : run.sealed_at }, answer: status };
    });
  }

  recordHeartbeat(a: HeartbeatArgs): Promise<HeartbeatResult> {
    return this.#updateRun<HeartbeatResult>(a.run_id, a.member_id, "not_found", (run) => {
      if (run.status === "abandoned" || run.status === "incomplete") return { answer: "closed" };
      const entry = entryOf(run, a.entry_id);
      if (entry === undefined) return { answer: "not_found" };
      const note = { at: a.at, seq: a.seq, head: a.head, offset_local: a.offset_local, recording: a.recording, vitals: a.vitals, client_ip: a.client_ip, client_country: a.client_country };
      if (!a.final) {
        // A sealed entry has nothing left to record.
        if (entry.sealed_at !== null) return { answer: "recorded" };
        const next = { ...entry, last_heartbeat: note, heartbeat_count: entry.heartbeat_count + 1 };
        return { next: withSignals(run, next, a), answer: "recorded" };
      }
      // The final heartbeat commits the end of the entry, once.
      if (entry.final !== null) {
        return { answer: entry.final.seq === a.seq && entry.final.head === a.head ? "final_existing" : "final_conflict" };
      }
      // It cannot contradict what the server already holds.
      const stored = entry.next_seq - 1; // -1 before any record
      if (stored > a.seq || (stored === a.seq && entry.head !== a.head)) return { answer: "final_conflict" };
      const next = { ...entry, final: { seq: a.seq, head: a.head }, final_repo_head: a.repo_head, last_heartbeat: note, heartbeat_count: entry.heartbeat_count + 1 };
      return { next: withSignals(run, next, a), answer: "final_committed" };
    });
  }

  async runsForTick(now: string): Promise<TickRun[]> {
    const cutoff = formatTime(addSeconds(new Date(now), -TICK_LOOKBACK_SECONDS));
    const runs: StoredRun[] = [];
    let cursor: string | undefined;
    do {
      const page = await this.#bucket.list({ prefix: "state/runs/", cursor });
      for (const { key } of page.objects) {
        const run = (await (await this.#bucket.get(key))?.json()) as StoredRun | undefined;
        if (run === undefined || run.status === "abandoned") continue;
        if (run.status === "open" || endsAt(run) >= cutoff) runs.push(run);
      }
      cursor = page.truncated ? page.cursor : undefined;
    } while (cursor !== undefined);
    return runs.sort((x, y) => (x.official_start + x.run_id).localeCompare(y.official_start + y.run_id)).map(toTick);
  }

  async queueNotices(notices: NoticeArgs[]): Promise<number> {
    let queued = 0;
    for (const n of notices) {
      const key = `state/notices/${n.run_id}.${n.kind}.${encodeURIComponent(n.dedupe_key) || "-"}.json`;
      const row = { run_id: n.run_id, member_id: n.member_id, kind: n.kind, dedupe_key: n.dedupe_key, payload: n.payload, created_at: n.at, sent_at: null };
      if ((await this.#bucket.put(key, JSON.stringify(row), { onlyIf: { etagDoesNotMatch: "*" } })) !== null) queued++;
    }
    return queued;
  }

  /** Local only: no accounts here, so `to` is always null (the mailer uses its test address or skips). */
  async claimNotices(limit: number, now: string): Promise<ClaimedNotice[]> {
    const claimed: ClaimedNotice[] = [];
    const stale = formatTime(addSeconds(new Date(now), -NOTICE_CLAIM_SECONDS));
    let cursor: string | undefined;
    do {
      const page = await this.#bucket.list({ prefix: "state/notices/", cursor });
      for (const { key } of page.objects) {
        if (claimed.length >= limit) break;
        const obj = await this.#bucket.get(key);
        if (obj === null) continue;
        const row = (await obj.json()) as LocalNotice;
        if (row.sent_at != null || row.skipped_at != null || (row.claimed_at != null && row.claimed_at >= stale)) continue;
        const next = { ...row, claimed_at: now };
        if ((await this.#bucket.put(key, JSON.stringify(next), { onlyIf: { etagMatches: obj.etag } })) === null) continue;
        claimed.push({ id: noticeId(key), kind: row.kind, run_id: row.run_id, to: null, subject: `Goal Cup: ${row.kind}`, text: JSON.stringify(row.payload) });
      }
      cursor = page.truncated && claimed.length < limit ? page.cursor : undefined;
    } while (cursor !== undefined);
    return claimed;
  }

  async finishNotice(id: number, outcome: NoticeOutcome, to: string | null, detail: string, now: string): Promise<void> {
    let cursor: string | undefined;
    do {
      const page = await this.#bucket.list({ prefix: "state/notices/", cursor });
      for (const { key } of page.objects) {
        if (noticeId(key) !== id) continue;
        await this.#updateObject<LocalNotice, void>(key, undefined, (row) => {
          if (row.sent_at != null || row.skipped_at != null) return { answer: undefined };
          if (outcome === "sent") return { next: { ...row, sent_at: now, sent_to: to, provider_id: detail }, answer: undefined };
          if (outcome === "skipped") return { next: { ...row, skipped_at: now, skipped_reason: detail }, answer: undefined };
          return { next: { ...row, last_error: detail }, answer: undefined };
        });
        return;
      }
      cursor = page.truncated ? page.cursor : undefined;
    } while (cursor !== undefined);
  }

  recordFinalCommit(memberId: string, runId: string, commit: string, at: string, d: FinalCommitDetail = NO_FINAL_DETAIL): Promise<void> {
    return this.#updateRun<void>(runId, memberId, undefined, (run) =>
      run.final_commit != null
        ? { answer: undefined }
        : {
            next: {
              ...run,
              final_commit: commit,
              final_commit_at: at,
              final_pushed_at: d.pushed_at,
              final_branch: d.branch,
              final_late_push: d.late_push,
              final_force_push: d.force_push,
              final_head_fallback: d.head_fallback,
            },
            answer: undefined,
          },
    );
  }

  async getRun(memberId: string, runId: string, entryId: string | null): Promise<{ run: RunRow; entry: EntryRow | null } | null> {
    const cur = await this.#readRun(runId);
    if (cur === null || cur.run.member_id !== memberId) return null;
    const entry = entryId === null ? currentOf(cur.run) : entryOf(cur.run, entryId);
    return { run: toRun(cur.run), entry: entry === undefined ? null : toEntry(entry) };
  }

  async getChunk(memberId: string, runId: string, entryId: string, offset: number): Promise<ChunkRecord | null> {
    const cur = await this.#readRun(runId);
    if (cur === null || cur.run.member_id !== memberId) return null;
    return entryOf(cur.run, entryId)?.chunks.find((c) => c.offset === offset) ?? null;
  }

  storeReceipt(memberId: string, runId: string, entryId: string, chainHead: string, receipt: StartReceipt): Promise<StoreReceiptResult> {
    return this.#updateRun<StoreReceiptResult>(runId, memberId, { kind: "not_found" }, (run) => {
      const entry = entryOf(run, entryId);
      if (entry === undefined) return { answer: { kind: "not_found" } };
      if (run.status === "abandoned" || run.status === "incomplete" || entry.sealed_at !== null) return { answer: { kind: "closed" } };
      if (entry.receipt !== null) {
        return entry.receipt.claims.chain_head === chainHead
          ? { answer: { kind: "existing", receipt: entry.receipt } }
          : { answer: { kind: "conflict" } };
      }
      return { next: withEntry(run, { ...entry, receipt }), answer: { kind: "stored", receipt } };
    });
  }

  abandonRun(memberId: string, runId: string, reason: string, at: string): Promise<AbandonResult> {
    return this.#updateRun<AbandonResult>(runId, memberId, "not_found", (run) => {
      if (run.status !== "open") return { answer: "closed" };
      if (run.entries.some((e) => e.chunks.length > 0)) return { answer: "has_chunks" };
      return { next: { ...run, status: "abandoned", abandon_reason: reason, sealed_at: at }, answer: "abandoned" };
    });
  }

  async commitChunk(a: CommitChunkArgs): Promise<CommitChunkResult> {
    const c = a.chunk;
    // The database raises on this too (goal_cup_upload_commit_chunk): a negative size is a Worker bug.
    if (!Number.isSafeInteger(a.stored_bytes) || a.stored_bytes < 0) throw new Error(`commitChunk: stored_bytes ${a.stored_bytes} is not a size`);
    return this.#updateRun<CommitChunkResult>(a.run_id, a.member_id, "not_found", (run) => {
      const entry = entryOf(run, c.entry_id);
      if (entry === undefined) return { answer: "not_found" };
      if (run.status === "abandoned" || run.status === "incomplete" || entry.sealed_at !== null) return { answer: "closed" };
      // Nothing past the committed end, whatever the Worker saw a moment ago.
      if (entry.final !== null && (c.last_seq > entry.final.seq || (c.last_seq === entry.final.seq && c.head !== entry.final.head))) {
        return { answer: "beyond_final" };
      }
      // The point of the whole function: advance only if the entry still stands where the caller saw it.
      if (entry.offset !== c.offset || entry.head !== c.prev || entry.next_seq !== c.first_seq) return { answer: "conflict" };
      const next = {
        ...entry,
        offset: c.offset + c.length,
        next_seq: c.last_seq + 1,
        head: c.head,
        last_ack: c.ack,
        chunks: [...entry.chunks, c],
        stored_bytes: (entry.stored_bytes ?? 0) + a.stored_bytes,
        chunk_count: (entry.chunk_count ?? 0) + 1,
      };
      return { next: withEntry(run, next), answer: "committed" };
    });
  }

  sealRun(a: SealArgs): Promise<SealResult> {
    return this.#updateRun<SealResult>(a.run_id, a.member_id, { kind: "not_found" }, (run) => {
      const entry = entryOf(run, a.entry_id);
      if (entry === undefined) return { answer: { kind: "not_found" } };
      const status = statusAt(toRun(run), new Date(a.sealed_at), this.#closeAfterSeconds);
      if (entry.sealed_at !== null) {
        const same = entry.seal_sha256 === a.seal_sha256 && entry.baseline_sha256 === a.baseline_sha256;
        return { answer: same ? { kind: "already_sealed", run_status: status } : { kind: "conflict" } };
      }
      if (run.status === "abandoned" || run.status === "incomplete") return { answer: { kind: "closed" } };
      // The seal needs the end the final heartbeat committed, and the chain held all the way to it.
      if (entry.final === null) return { answer: { kind: "no_final" } };
      if (entry.next_seq - 1 !== entry.final.seq || entry.head !== entry.final.head) return { answer: { kind: "chain_behind" } };
      const sealed = withEntry(run, { ...entry, seal_sha256: a.seal_sha256, baseline_sha256: a.baseline_sha256, sealed_at: a.sealed_at });
      // The run is sealed when its current entry is, once it is over.
      const after = statusAt(toRun(sealed), new Date(a.sealed_at), this.#closeAfterSeconds);
      return { next: { ...sealed, status: after, sealed_at: after === "sealed" ? a.sealed_at : sealed.sealed_at }, answer: { kind: "sealed", run_status: after } };
    });
  }
}

/**
 * `entry` with the heartbeat noted, in `run`, and the run-level facts it adds: its time, the latest
 * connection (a missing one keeps the last), the first flag, an entry's first or higher
 * `model_requests` (each entry counts from zero), the latest vitals and the disk pause they report.
 */
function withSignals(run: StoredRun, entry: StoredEntry, a: HeartbeatArgs): StoredRun {
  const flag = a.recording === "gaps" || a.recording === "failed" ? a.recording : null;
  const grew = a.model_requests !== null && (entry.model_requests == null || a.model_requests > entry.model_requests);
  return {
    ...withEntry(run, grew ? { ...entry, model_requests: a.model_requests } : entry),
    last_heartbeat_at: a.at,
    last_client_ip: a.client_ip ?? run.last_client_ip ?? null,
    last_client_country: a.client_country ?? run.last_client_country ?? null,
    ...(flag !== null && run.flagged_at == null ? { flagged_at: a.at, flag_reason: flag } : {}),
    ...(grew ? { last_model_request_at: a.at } : {}),
    ...diskPause(run, a),
  };
}

/**
 * The run's vitals and disk pause after this heartbeat. Twin of the rule in
 * goal_cup_tracer.note_heartbeat (sql/pending/20261021100000_goal_cup_tracer_event_readiness.sql,
 * section 3): change both together.
 */
function diskPause(run: StoredRun, a: HeartbeatArgs): Pick<StoredRun, "last_vitals" | "disk_paused_at" | "disk_notice_at"> {
  if (a.vitals === null) return {}; // no vitals: nothing said about a pause
  if (a.vitals.agent_paused !== "disk") return { last_vitals: a.vitals, disk_paused_at: null };
  if (run.disk_paused_at != null) return { last_vitals: a.vitals }; // the same pause goes on
  const gap = formatTime(addSeconds(new Date(a.at), -DISK_NOTICE_GAP_SECONDS));
  const owed = run.disk_notice_at == null || run.disk_notice_at <= gap;
  return { last_vitals: a.vitals, disk_paused_at: a.at, ...(owed ? { disk_notice_at: a.at } : {}) };
}

const newEntry = (
  entry_id: string,
  run_id: string,
  segment: number,
  a: Pick<OpenRunArgs, "connector_id" | "manifest_sha256" | "connector_dir_sha256" | "workspace" | "session_id" | "harness_version" | "tracer_version">,
): StoredEntry => ({
  entry_id,
  run_id,
  segment,
  connector_id: a.connector_id,
  manifest_sha256: a.manifest_sha256,
  connector_dir_sha256: a.connector_dir_sha256,
  workspace: a.workspace,
  session_id: a.session_id,
  harness_version: a.harness_version,
  tracer_version: a.tracer_version,
  offset: 0,
  next_seq: FIRST_SEQ,
  head: GENESIS,
  last_ack: null,
  final: null,
  receipt: null,
  seal_sha256: null,
  baseline_sha256: null,
  sealed_at: null,
  chunks: [],
  stored_bytes: 0,
  chunk_count: 0,
  final_repo_head: null,
  last_heartbeat: null,
  heartbeat_count: 0,
  model_requests: null,
});

/** The public shape of a run: what it says about itself, and its current entry. */
function toRun(r: StoredRun): RunRow {
  const {
    machine: _m, client_ip: _i, client_country: _c, abandon_reason: _a, sealed_at: _s, entries: _e,
    last_client_ip: _p, last_client_country: _q,
    last_heartbeat_at: _h, last_model_request_at: _l, flagged_at: _f, flag_reason: _g, final_commit: _k, final_commit_at: _t,
    final_pushed_at: _pa, final_branch: _b, final_late_push: _lp, final_force_push: _fp, final_head_fallback: _hf,
    last_vitals: _v, disk_paused_at: _d, disk_notice_at: _n,
    ...row
  } = r;
  const cur = currentOf(r);
  return {
    ...row,
    first_entry_id: r.entries[0]!.entry_id,
    current_entry_id: cur.entry_id,
    current_sealed_at: cur.sealed_at,
    stored_bytes: r.entries.reduce((sum, e) => sum + (e.stored_bytes ?? 0), 0),
    chunk_count: r.entries.reduce((sum, e) => sum + (e.chunk_count ?? 0), 0),
  };
}

/** A run as the cron reads it. */
function toTick(r: StoredRun): TickRun {
  return {
    ...toRun(r),
    last_heartbeat_at: r.last_heartbeat_at ?? null,
    last_model_request_at: r.last_model_request_at ?? null,
    flagged_at: r.flagged_at ?? null,
    flag_reason: r.flag_reason ?? null,
    final_commit: r.final_commit ?? null,
    disk_notice_at: r.disk_notice_at ?? null,
  };
}

/** The public shape of an entry: drop the bookkeeping that only the local store keeps. */
function toEntry(e: StoredEntry): EntryRow {
  const {
    harness_version: _h, tracer_version: _t, chunks: _c, final_repo_head: _f, last_heartbeat: _l, heartbeat_count: _n, model_requests: _m,
    stored_bytes: _b, chunk_count: _k,
    ...row
  } = e;
  return row;
}
