// LocalDb: the database for `wrangler dev` and tests ONLY. It keeps its state as JSON objects in
// the same R2 bucket, under state/…, and makes every change atomic with a conditional put
// (the object's etag must still be the one that was read). Production uses SupabaseDb.
//
// Layout:
//   state/runs/{run_id}.json     the run, its receipt, its chunks (a few hundred bytes each)
//   state/open/{member}.json     {"run_id": …}: which run holds the member's one-open-run slot
//   state/devices/{sha256}.json  a device request, by the hash of its device code
//   state/usercodes/{sha256}.json  {"device_sha256": …}: the request a user code belongs to
//   state/credentials/{sha256}.json  a run credential, by its own hash
//
// Not built for load: a run's whole state is one object. Fine for a developer and for tests.

import type {
  AbandonResult,
  ApproveDeviceResult,
  ChunkRecord,
  CommitChunkArgs,
  CommitChunkResult,
  CreateDeviceArgs,
  Credential,
  Db,
  DevicePoll,
  HeartbeatArgs,
  HeartbeatResult,
  OpenRunArgs,
  OpenRunResult,
  RunRow,
  SealArgs,
  SealResult,
  StoreReceiptResult,
} from "./db.ts";
import { ABANDON_AFTER_SECONDS, FIRST_SEQ, GENESIS, UNBOUND_CREDENTIAL_SECONDS, isAbandonable } from "./db.ts";
import type { StartReceipt } from "./sign.ts";
import { addSeconds, formatTime, parseTime, sha256Hex, toHex } from "./util.ts";

interface StoredRun extends RunRow {
  harness_version: string;
  tracer_version: string;
  abandon_reason: string | null;
  sealed_at: string | null;
  chunks: ChunkRecord[];
  final_repo_head: string | null;
  last_heartbeat: { at: string; seq: number; head: string; offset_local: number } | null;
  heartbeat_count: number;
}

interface StoredDevice extends CreateDeviceArgs {
  status: "pending" | "approved" | "denied";
  member_id: string | null;
  decided_at: string | null;
  picked_up: boolean;
}

const MAX_CAS_TRIES = 10;

export class LocalDb implements Db {
  readonly #bucket: R2Bucket;
  readonly #abandonAfterSeconds: number;

  private constructor(bucket: R2Bucket, abandonAfterSeconds: number) {
    this.#bucket = bucket;
    this.#abandonAfterSeconds = abandonAfterSeconds;
  }

  static create(
    bucket: R2Bucket,
    // Tests only (ABANDON_AFTER_SECONDS, honoured only by this local backend): production is 24 hours.
    abandonAfterSeconds: number = ABANDON_AFTER_SECONDS,
  ): LocalDb {
    return new LocalDb(bucket, abandonAfterSeconds);
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
      if (parseTime(now).getTime() >= parseTime(dev.expires_at).getTime()) return { answer: { kind: "expired" } };
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
      expires_at: formatTime(addSeconds(parseTime(won.decided_at ?? now), UNBOUND_CREDENTIAL_SECONDS)),
    };
    await this.#bucket.put(this.#credentialKey(await sha256Hex(plain)), JSON.stringify(cred), { onlyIf: { etagDoesNotMatch: "*" } });
    return { kind: "approved", purpose: "run", credential_plain: plain, expires_at: cred.expires_at };
  }

  async approveDevice(userCode: string, memberId: string, approve: boolean, now: string): Promise<ApproveDeviceResult> {
    const link = await this.#bucket.get(this.#userCodeKey(await sha256Hex(userCode)));
    if (link === null) return "not_found";
    const { device_sha256 } = (await link.json()) as { device_sha256: string };
    return this.#updateObject<StoredDevice, ApproveDeviceResult>(this.#deviceKey(device_sha256), "not_found", (dev) => {
      if (parseTime(now).getTime() >= parseTime(dev.expires_at).getTime()) return { answer: "expired" };
      if (dev.status !== "pending") return { answer: "already_decided" };
      return { next: { ...dev, status: approve ? "approved" : "denied", member_id: memberId, decided_at: now }, answer: "ok" };
    });
  }

  async credential(credentialSha256: string, now: string): Promise<Credential | null> {
    const obj = await this.#bucket.get(this.#credentialKey(credentialSha256));
    if (obj === null) return null;
    const cred = (await obj.json()) as Credential;
    return parseTime(now).getTime() < parseTime(cred.expires_at).getTime() ? cred : null;
  }

  async bindCredential(credentialSha256: string, runId: string, expiresAt: string): Promise<void> {
    await this.#updateObject<Credential, void>(this.#credentialKey(credentialSha256), undefined, (cred) => ({
      next: { ...cred, run_id: runId, expires_at: expiresAt },
      answer: undefined,
    }));
  }

  // ---- Db: runs --------------------------------------------------------------------------

  async openRun(a: OpenRunArgs): Promise<OpenRunResult> {
    const openKey = this.#openKey(a.member_id);
    for (let i = 0; i < MAX_CAS_TRIES; i++) {
      const marker = await this.#bucket.get(openKey);
      if (marker !== null) {
        const heldId = ((await marker.json()) as { run_id: string }).run_id;
        let held = await this.#readRun(heldId);
        // An unsealed run 24 hours past its deadline is abandoned first, which frees the slot.
        // (`official_start` is the caller's "now": the Worker's clock.)
        if (held !== null && held.run.status === "open" && isAbandonable(held.run.deadline_at, parseTime(a.official_start), this.#abandonAfterSeconds)) {
          await this.expireRun(held.run.member_id, held.run.run_id, a.official_start);
          held = await this.#readRun(heldId);
        }
        if (held !== null && held.run.status === "open") {
          if (held.run.session_id === a.session_id && held.run.receipt === null) {
            return { kind: "existing", run: toRow(held.run) };
          }
          return { kind: "conflict" };
        }
        // The marker points at a finished (or missing) run: the slot is free.
      }

      const run: StoredRun = {
        run_id: a.run_id,
        entry_id: a.entry_id,
        event_id: a.event_id,
        member_id: a.member_id,
        status: "open",
        official_start: a.official_start,
        deadline_at: a.deadline_at,
        connector_id: a.connector_id,
        manifest_sha256: a.manifest_sha256,
        connector_dir_sha256: a.connector_dir_sha256,
        workspace: a.workspace,
        session_id: a.session_id,
        repo_url: a.repo_url,
        harness_version: a.harness_version,
        tracer_version: a.tracer_version,
        offset: 0,
        next_seq: FIRST_SEQ,
        head: GENESIS,
        last_ack: null,
        receipt: null,
        seal_sha256: null,
        baseline_sha256: null,
        abandon_reason: null,
        sealed_at: null,
        chunks: [],
        final: null,
        final_repo_head: null,
        last_heartbeat: null,
        heartbeat_count: 0,
      };
      // Write the run first, then take the slot; if the slot is lost, remove the run again.
      await this.#bucket.put(this.#runKey(a.run_id), JSON.stringify(run), { onlyIf: { etagDoesNotMatch: "*" } });
      const body = JSON.stringify({ run_id: a.run_id });
      const took = await this.#bucket.put(
        openKey,
        body,
        marker === null ? { onlyIf: { etagDoesNotMatch: "*" } } : { onlyIf: { etagMatches: marker.etag } },
      );
      if (took !== null) return { kind: "created", run: toRow(run) };
      await this.#bucket.delete(this.#runKey(a.run_id));
    }
    throw new Error("local state is too contended; try again");
  }

  /** Mark an open, unsealed run abandoned because its time ran out. */
  expireRun(memberId: string, runId: string, at: string): Promise<boolean> {
    return this.#updateRun<boolean>(runId, memberId, false, (run) => {
      if (run.status !== "open" || run.seal_sha256 !== null) return { answer: false };
      if (!isAbandonable(run.deadline_at, parseTime(at), this.#abandonAfterSeconds)) return { answer: false };
      return {
        next: { ...run, status: "abandoned", abandon_reason: "expired: not sealed 24 hours after the deadline", sealed_at: at },
        answer: true,
      };
    });
  }

  recordHeartbeat(a: HeartbeatArgs): Promise<HeartbeatResult> {
    return this.#updateRun<HeartbeatResult>(a.run_id, a.member_id, "not_found", (run) => {
      if (run.status === "abandoned") return { answer: "closed" };
      const note = { at: a.at, seq: a.seq, head: a.head, offset_local: a.offset_local };
      if (!a.final) {
        // A sealed run has nothing left to record.
        if (run.status !== "open") return { answer: "recorded" };
        return { next: { ...run, last_heartbeat: note, heartbeat_count: run.heartbeat_count + 1 }, answer: "recorded" };
      }
      // The final heartbeat commits the end of the run, once.
      if (run.final !== null) {
        return { answer: run.final.seq === a.seq && run.final.head === a.head ? "final_existing" : "final_conflict" };
      }
      // It cannot contradict what the server already holds.
      const stored = run.next_seq - 1; // -1 before any record
      if (stored > a.seq || (stored === a.seq && run.head !== a.head)) return { answer: "final_conflict" };
      return {
        next: {
          ...run,
          final: { seq: a.seq, head: a.head },
          final_repo_head: a.repo_head,
          last_heartbeat: note,
          heartbeat_count: run.heartbeat_count + 1,
        },
        answer: "final_committed",
      };
    });
  }

  async getRun(memberId: string, runId: string): Promise<RunRow | null> {
    const cur = await this.#readRun(runId);
    return cur !== null && cur.run.member_id === memberId ? toRow(cur.run) : null;
  }

  async getChunk(memberId: string, runId: string, offset: number): Promise<ChunkRecord | null> {
    const cur = await this.#readRun(runId);
    if (cur === null || cur.run.member_id !== memberId) return null;
    return cur.run.chunks.find((c) => c.offset === offset) ?? null;
  }

  storeReceipt(memberId: string, runId: string, chainHead: string, receipt: StartReceipt): Promise<StoreReceiptResult> {
    return this.#updateRun<StoreReceiptResult>(runId, memberId, { kind: "closed" }, (run) => {
      if (run.status !== "open") return { answer: { kind: "closed" } };
      if (run.receipt !== null) {
        return run.receipt.claims.chain_head === chainHead
          ? { answer: { kind: "existing", receipt: run.receipt } }
          : { answer: { kind: "conflict" } };
      }
      return { next: { ...run, receipt }, answer: { kind: "stored", receipt } };
    });
  }

  abandonRun(memberId: string, runId: string, reason: string, at: string): Promise<AbandonResult> {
    return this.#updateRun<AbandonResult>(runId, memberId, "not_found", (run) => {
      if (run.status !== "open") return { answer: "closed" };
      if (run.chunks.length > 0) return { answer: "has_chunks" };
      return { next: { ...run, status: "abandoned", abandon_reason: reason, sealed_at: at }, answer: "abandoned" };
    });
  }

  commitChunk(a: CommitChunkArgs): Promise<CommitChunkResult> {
    const c = a.chunk;
    return this.#updateRun<CommitChunkResult>(a.run_id, a.member_id, "not_found", (run) => {
      if (run.status !== "open") return { answer: "closed" };
      // Nothing past the committed end, whatever the Worker saw a moment ago.
      if (run.final !== null && (c.last_seq > run.final.seq || (c.last_seq === run.final.seq && c.head !== run.final.head))) {
        return { answer: "beyond_final" };
      }
      // The point of the whole function: advance only if the run still stands where the caller saw it.
      if (run.offset !== c.offset || run.head !== c.prev || run.next_seq !== c.first_seq) return { answer: "conflict" };
      return {
        next: {
          ...run,
          offset: c.offset + c.length,
          next_seq: c.last_seq + 1,
          head: c.head,
          last_ack: c.ack,
          chunks: [...run.chunks, c],
        },
        answer: "committed",
      };
    });
  }

  sealRun(a: SealArgs): Promise<SealResult> {
    return this.#updateRun<SealResult>(a.run_id, a.member_id, "not_found", (run) => {
      if (run.status === "sealed") {
        const same = run.seal_sha256 === a.seal_sha256 && run.baseline_sha256 === a.baseline_sha256;
        return { answer: same ? "already_sealed" : "conflict" };
      }
      if (run.status !== "open") return { answer: "closed" };
      return {
        next: { ...run, status: "sealed", seal_sha256: a.seal_sha256, baseline_sha256: a.baseline_sha256, sealed_at: a.sealed_at },
        answer: "sealed",
      };
    });
  }
}

/** The public shape of a run: drop the bookkeeping that only the local store keeps. */
function toRow(r: StoredRun): RunRow {
  const {
    harness_version: _h, tracer_version: _t, abandon_reason: _a, sealed_at: _s, chunks: _c,
    final_repo_head: _f, last_heartbeat: _l, heartbeat_count: _n, ...row
  } = r;
  return row;
}
