// SupabaseDb: the production database. Each method is one call to a Postgres function in schema
// `goalcup`, through Supabase's PostgREST RPC endpoint:
//
//   POST {SUPABASE_URL}/rest/v1/rpc/<function>     Content-Profile: goalcup
//
// The functions are proposed in sql/proposed-goalcup-functions.sql (the website repo owns the
// schema). The Worker's key belongs to a dedicated role that may execute only those functions.
// Tokens: Supabase stores only the sha256 of a token, so the Worker hashes the bearer token
// and sends the hash.

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
import { ApiError } from "./errors.ts";
import type { StartReceipt } from "./sign.ts";

const SCHEMA = "goalcup";
const TIMEOUT_MS = 10_000;

export class SupabaseDb implements Db {
  readonly #base: string;
  readonly #key: string;

  constructor(url: string | undefined, key: string | undefined) {
    if (!url || !key) throw new Error("DB_BACKEND=supabase needs the SUPABASE_URL and SUPABASE_KEY secrets");
    this.#base = url.replace(/\/+$/, "");
    this.#key = key;
  }

  /** Call one function. Any failure becomes a 502, which the tracer retries with backoff. */
  async #rpc(fn: string, args: Record<string, unknown>): Promise<unknown> {
    let res: Response;
    try {
      res = await fetch(`${this.#base}/rest/v1/rpc/${fn}`, {
        method: "POST",
        headers: {
          apikey: this.#key,
          authorization: `Bearer ${this.#key}`,
          "content-type": "application/json",
          accept: "application/json",
          "content-profile": SCHEMA,
          "accept-profile": SCHEMA,
        },
        body: JSON.stringify(args),
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
    } catch (e) {
      console.error(`supabase ${fn}: request failed: ${String(e)}`);
      throw new ApiError(502, "upstream_error", "The database could not be reached.");
    }
    if (!res.ok) {
      // Log the status and PostgREST's error text (no request data, no keys); tell the client nothing more.
      console.error(`supabase ${fn}: HTTP ${res.status}: ${(await res.text()).slice(0, 300)}`);
      throw new ApiError(502, "upstream_error", "The database refused the request.");
    }
    return res.json();
  }

  /** The functions return a fixed set of words; anything else means the SQL and this file disagree. */
  #word<T extends string>(fn: string, v: unknown, allowed: readonly T[]): T {
    if (typeof v === "string" && (allowed as readonly string[]).includes(v)) return v as T;
    console.error(`supabase ${fn}: unexpected result ${JSON.stringify(v)?.slice(0, 200)}`);
    throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
  }

  async memberForToken(tokenSha256: string): Promise<string | null> {
    const v = await this.#rpc("upload_lookup_token", { p_token_hash: tokenSha256 });
    return typeof v === "string" && v !== "" ? v : null;
  }

  async openRun(a: OpenRunArgs): Promise<OpenRunResult> {
    const v = (await this.#rpc("upload_open_run", {
      p_member_id: a.member_id,
      p_event_id: a.event_id,
      p_run_id: a.run_id,
      p_entry_id: a.entry_id,
      p_official_start: a.official_start,
      p_deadline_at: a.deadline_at,
      p_connector_id: a.connector_id,
      p_manifest_sha256: a.manifest_sha256,
      p_connector_dir_sha256: a.connector_dir_sha256,
      p_workspace: a.workspace,
      p_session_id: a.session_id,
      p_harness_version: a.harness_version,
      p_tracer_version: a.tracer_version,
      p_repo_url: a.repo_url,
    })) as { result?: unknown; run?: RunRow; message?: string } | null;
    const result = this.#word("upload_open_run", v?.result, ["created", "existing", "conflict", "forbidden"] as const);
    if (result === "conflict") return { kind: "conflict" };
    if (result === "forbidden") return { kind: "forbidden", message: v?.message ?? "You may not start a run now." };
    if (!v?.run) throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
    return { kind: result, run: v.run };
  }

  async getRun(memberId: string, runId: string): Promise<RunRow | null> {
    const v = await this.#rpc("upload_run_position", { p_member_id: memberId, p_run_id: runId });
    return (v as RunRow | null) ?? null;
  }

  async getChunk(memberId: string, runId: string, offset: number): Promise<ChunkRecord | null> {
    const v = await this.#rpc("upload_get_chunk", { p_member_id: memberId, p_run_id: runId, p_offset: offset });
    return (v as ChunkRecord | null) ?? null;
  }

  async storeReceipt(memberId: string, runId: string, chainHead: string, receipt: StartReceipt): Promise<StoreReceiptResult> {
    const v = (await this.#rpc("upload_issue_receipt", {
      p_member_id: memberId,
      p_run_id: runId,
      p_chain_head: chainHead,
      p_receipt: receipt,
    })) as { result?: unknown; receipt?: StartReceipt } | null;
    const result = this.#word("upload_issue_receipt", v?.result, ["stored", "existing", "conflict", "closed"] as const);
    if (result === "conflict" || result === "closed") return { kind: result };
    if (!v?.receipt) throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
    return { kind: result, receipt: v.receipt };
  }

  async abandonRun(memberId: string, runId: string, reason: string, at: string): Promise<AbandonResult> {
    const v = await this.#rpc("upload_abandon_run", { p_member_id: memberId, p_run_id: runId, p_reason: reason, p_at: at });
    return this.#word("upload_abandon_run", v, ["abandoned", "has_chunks", "closed", "not_found"] as const);
  }

  async commitChunk(a: CommitChunkArgs): Promise<CommitChunkResult> {
    const v = await this.#rpc("upload_commit_chunk", { p_member_id: a.member_id, p_run_id: a.run_id, p_chunk: a.chunk });
    return this.#word("upload_commit_chunk", v, ["committed", "conflict", "closed", "not_found"] as const);
  }

  async sealRun(a: SealArgs): Promise<SealResult> {
    const v = await this.#rpc("upload_seal_run", {
      p_member_id: a.member_id,
      p_run_id: a.run_id,
      p_entry_id: a.entry_id,
      p_seal_sha256: a.seal_sha256,
      p_baseline_sha256: a.baseline_sha256,
      p_seal_key: a.seal_key,
      p_baseline_key: a.baseline_key,
      p_sealed_at: a.sealed_at,
    });
    return this.#word("upload_seal_run", v, ["sealed", "already_sealed", "conflict", "closed", "not_found"] as const);
  }
}
