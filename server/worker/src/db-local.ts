// LocalDb: the database for `wrangler dev` and tests ONLY. It keeps its state as JSON objects in
// the same R2 bucket, under state/…, and makes every change atomic with a conditional put
// (the object's etag must still be the one that was read). Production uses SupabaseDb.
//
// Layout:
//   state/runs/{run_id}.json     the run, its receipt, its chunks (a few hundred bytes each)
//   state/open/{member}.json     {"run_id": …}: which run holds the member's one-open-run slot
//
// Not built for load: a run's whole state is one object. Fine for a developer and for tests.

import type {
  AbandonResult,
  ChunkRecord,
  CommitChunkArgs,
  CommitChunkResult,
  Db,
  OpenRunArgs,
  OpenRunResult,
  RunRow,
  SealArgs,
  SealResult,
  StoreReceiptResult,
} from "./db.ts";
import { FIRST_SEQ, GENESIS } from "./db.ts";
import type { StartReceipt } from "./sign.ts";
import { sha256Hex } from "./util.ts";

interface StoredRun extends RunRow {
  harness_version: string;
  tracer_version: string;
  abandon_reason: string | null;
  sealed_at: string | null;
  chunks: ChunkRecord[];
}

const MAX_CAS_TRIES = 10;

export class LocalDb implements Db {
  readonly #bucket: R2Bucket;
  readonly #tokens: Map<string, string>; // sha256(token) -> member

  private constructor(bucket: R2Bucket, tokens: Map<string, string>) {
    this.#bucket = bucket;
    this.#tokens = tokens;
  }

  /** Parses DEV_TOKENS ("token1:member1,token2:member2"). Refuses to exist without it. */
  static async create(bucket: R2Bucket, devTokens: string | undefined): Promise<LocalDb> {
    if (!devTokens || devTokens.trim() === "") {
      throw new Error("DB_BACKEND=local needs DEV_TOKENS (\"token1:member1,token2:member2\"); refusing to start without it");
    }
    const tokens = new Map<string, string>();
    for (const pair of devTokens.split(",")) {
      const at = pair.indexOf(":");
      const token = pair.slice(0, at).trim();
      const member = pair.slice(at + 1).trim();
      if (at < 1 || token === "" || member === "") throw new Error(`DEV_TOKENS entry "${pair}" is not token:member`);
      tokens.set(await sha256Hex(token), member);
    }
    return new LocalDb(bucket, tokens);
  }

  async memberForToken(tokenSha256: string): Promise<string | null> {
    return this.#tokens.get(tokenSha256) ?? null;
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

  // ---- Db --------------------------------------------------------------------------------

  async openRun(a: OpenRunArgs): Promise<OpenRunResult> {
    const openKey = this.#openKey(a.member_id);
    for (let i = 0; i < MAX_CAS_TRIES; i++) {
      const marker = await this.#bucket.get(openKey);
      if (marker !== null) {
        const heldId = ((await marker.json()) as { run_id: string }).run_id;
        const held = await this.#readRun(heldId);
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
  const { harness_version: _h, tracer_version: _t, abandon_reason: _a, sealed_at: _s, chunks: _c, ...row } = r;
  return row;
}
