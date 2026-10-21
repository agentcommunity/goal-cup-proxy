// SupabaseDb: the production database. Each method is one call to a Postgres function through
// Supabase's PostgREST RPC endpoint:
//
//   POST {SUPABASE_URL}/rest/v1/rpc/goal_cup_<function>
//
// The functions are public.goal_cup_upload_* in the website repo's migration
// supabase/migrations/20261015120000_goal_cup_tracer_upload.sql (copied, for reference, to
// sql/goal-cup-tracer-upload.sql); their tables live in the private schema goal_cup_tracer, which
// PostgREST does not expose. The Worker's key is a Supabase secret API key whose role may execute
// only those functions.
// Credentials and device codes: Supabase stores only their sha256, so the Worker hashes the
// bearer credential (or the device code) and sends the hash. The one plain value that comes
// back is a freshly minted credential, once, from upload_poll_device.

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
  HeartbeatArgs,
  HeartbeatResult,
  NoticeArgs,
  NoticeOutcome,
  OpenRunArgs,
  OpenRunResult,
  RunRow,
  RunStatus,
  SealArgs,
  SealResult,
  StoreReceiptResult,
  TickRun,
} from "./db.ts";
import { ApiError } from "./errors.ts";
import type { StartReceipt } from "./sign.ts";
import { RUN_ID, isEntryId, isHex64, isTime } from "./util.ts";

const STATUS_WORDS = ["open", "sealed", "incomplete", "abandoned"] as const;

// What a row coming back must look like. The SQL and this file are written apart, so a row is
// checked field by field before anything trusts it.
type Check = (v: unknown) => boolean;
const isObject: Check = (v) => typeof v === "object" && v !== null && !Array.isArray(v);
const text: Check = (v) => typeof v === "string";
const uint: Check = (v) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const hex64: Check = isHex64;
/** A field a database without the latest migration does not send yet; when sent it must pass `check`. */
const optional =
  (check: Check): Check =>
  (v) =>
    v === undefined || check(v);
const nullable =
  (check: Check): Check =>
  (v) =>
    v === null || check(v);
const oneOf =
  (...words: string[]): Check =>
  (v) =>
    typeof v === "string" && words.includes(v);
const END: Check = (v) => isObject(v) && uint((v as Record<string, unknown>).seq) && hex64((v as Record<string, unknown>).head);

const RUN_SHAPE: Record<string, Check> = {
  run_id: (v) => typeof v === "string" && RUN_ID.test(v),
  event_id: text,
  member_id: text,
  mode: oneOf("competition", "practice", "test"),
  status: oneOf(...STATUS_WORDS),
  official_start: isTime,
  deadline_at: isTime,
  replaced_at: nullable(isTime),
  repo_url: text,
  first_entry_id: isEntryId,
  current_entry_id: isEntryId,
  current_sealed_at: nullable(isTime),
  // Website migration 20261021110000 adds these. Before it they are absent, and chunks.ts refuses
  // uploads (502, retried) instead of letting a run past the size limit; every other call still works.
  stored_bytes: optional(uint),
  chunk_count: optional(uint),
};
const TICK_SHAPE: Record<string, Check> = {
  ...RUN_SHAPE,
  last_heartbeat_at: nullable(isTime),
  last_model_request_at: nullable(isTime),
  flagged_at: nullable(isTime),
  flag_reason: nullable(oneOf("gaps", "failed")),
  final_commit: nullable((v) => typeof v === "string" && /^([0-9a-f]{40}|none)$/.test(v)),
};
const ENTRY_SHAPE: Record<string, Check> = {
  entry_id: isEntryId,
  run_id: RUN_SHAPE.run_id!,
  segment: uint,
  connector_id: text,
  manifest_sha256: hex64,
  connector_dir_sha256: hex64,
  workspace: text,
  session_id: text,
  offset: uint,
  next_seq: uint,
  head: hex64,
  last_ack: nullable(isObject),
  final: nullable(END),
  receipt: nullable(isObject),
  seal_sha256: nullable(hex64),
  baseline_sha256: nullable(hex64),
  sealed_at: nullable(isTime),
};
const NOTICE_SHAPE: Record<string, Check> = {
  id: uint,
  kind: oneOf("started", "silence", "idle_3h", "idle_6h", "recording", "ended"),
  run_id: RUN_SHAPE.run_id!,
  to: nullable(text),
  subject: text,
  text: text,
};
const CHUNK_SHAPE: Record<string, Check> = {
  entry_id: isEntryId,
  offset: uint,
  length: uint,
  sha256: hex64,
  first_seq: uint,
  last_seq: uint,
  prev: hex64,
  head: hex64,
  r2_key: text,
  received_at: isTime,
  ack: isObject,
};

/** Every function the Worker calls is `public.goal_cup_<name>`, the website's naming convention. */
const PREFIX = "goal_cup_";
const TIMEOUT_MS = 10_000;

/** PostgREST's `code` and `message` out of an error reply, shortened; nothing else it said. */
async function errorSummary(res: Response): Promise<string> {
  try {
    const e = (await res.json()) as { code?: unknown; message?: unknown };
    const part = (v: unknown) => (typeof v === "string" ? v.slice(0, 200) : "");
    return `${part(e.code)} ${part(e.message)}`.trim();
  } catch {
    return "(no readable error)";
  }
}

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
      res = await fetch(`${this.#base}/rest/v1/rpc/${PREFIX}${fn}`, {
        method: "POST",
        headers: {
          apikey: this.#key,
          authorization: `Bearer ${this.#key}`,
          "content-type": "application/json",
          accept: "application/json",
        },
        body: JSON.stringify(args),
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
    } catch (e) {
      console.error(`supabase ${fn}: request failed: ${String(e)}`);
      throw new ApiError(502, "upstream_error", "The database could not be reached.");
    }
    if (!res.ok) {
      // Log the status and PostgREST's error `code` and `message` only: its `details` and `hint` can
      // quote the row (machine, client_ip). Tell the client nothing more.
      console.error(`supabase ${fn}: HTTP ${res.status}: ${await errorSummary(res)}`);
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

  /** A function that answers true or false. */
  #flag(fn: string, v: unknown): boolean {
    if (typeof v === "boolean") return v;
    console.error(`supabase ${fn}: unexpected result ${JSON.stringify(v)?.slice(0, 200)}`);
    throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
  }

  /** A row that has every field of `shape` in the right form, or a 502 that names the fields (never their values). */
  #row<T>(fn: string, v: unknown, what: string, shape: Record<string, Check>): T {
    const bad = isObject(v) ? Object.keys(shape).filter((k) => !shape[k]!((v as Record<string, unknown>)[k])) : ["(not an object)"];
    if (bad.length === 0) return v as T;
    console.error(`supabase ${fn}: the ${what} has fields that do not look right: ${bad.join(", ")}`);
    throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
  }

  /** A run and an entry together, as the position and the open and continue calls answer them. */
  #pair(fn: string, run: unknown, entry: unknown): { run: RunRow; entry: EntryRow } {
    return { run: this.#row<RunRow>(fn, run, "run", RUN_SHAPE), entry: this.#row<EntryRow>(fn, entry, "entry", ENTRY_SHAPE) };
  }

  async credential(credentialSha256: string, now: string): Promise<Credential | null> {
    const v = await this.#rpc("upload_credential", { p_credential_hash: credentialSha256, p_now: now });
    return (v as Credential | null) ?? null;
  }

  async createDevice(a: CreateDeviceArgs): Promise<void> {
    const v = await this.#rpc("upload_create_device", {
      p_device_hash: a.device_sha256,
      p_user_hash: a.user_sha256,
      p_purpose: a.purpose,
      p_mode: a.mode,
      p_connector_id: a.connector_id,
      p_tracer_version: a.tracer_version,
      p_created_at: a.created_at,
      p_expires_at: a.expires_at,
    });
    this.#word("upload_create_device", v, ["created"] as const);
  }

  async pollDevice(deviceSha256: string, now: string): Promise<DevicePoll> {
    const v = (await this.#rpc("upload_poll_device", { p_device_hash: deviceSha256, p_now: now })) as {
      result?: unknown;
      purpose?: "login" | "run";
      credential?: string;
      expires_at?: string;
      handle?: string;
    } | null;
    const result = this.#word("upload_poll_device", v?.result, ["pending", "approved", "denied", "expired"] as const);
    if (result !== "approved") return { kind: result };
    if (v?.purpose === "login" && typeof v.handle === "string") return { kind: "approved", purpose: "login", handle: v.handle };
    if (v?.purpose === "run" && typeof v.credential === "string" && typeof v.expires_at === "string") {
      return { kind: "approved", purpose: "run", credential_plain: v.credential, expires_at: v.expires_at };
    }
    throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
  }

  async approveDevice(userCode: string, memberId: string, approve: boolean, now: string): Promise<ApproveDeviceResult> {
    const v = await this.#rpc("upload_approve_device", { p_user_code: userCode, p_member_id: memberId, p_approve: approve, p_now: now });
    return this.#word("upload_approve_device", v, ["ok", "not_found", "expired", "already_decided", "forbidden"] as const);
  }

  async openRun(a: OpenRunArgs): Promise<OpenRunResult> {
    const v = (await this.#rpc("upload_open_run", {
      p_member_id: a.member_id,
      p_event_id: a.event_id,
      p_mode: a.mode,
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
      p_machine: a.machine,
      p_client_ip: a.client_ip,
      p_client_country: a.client_country,
      p_credential_hash: a.credential,
      p_credential_expires_at: a.credential_expires_at,
    })) as { result?: unknown; run?: unknown; entry?: unknown; message?: string } | null;
    const result = this.#word("upload_open_run", v?.result, ["created", "existing", "conflict", "forbidden"] as const);
    if (result === "conflict") return { kind: "conflict" };
    if (result === "forbidden") return { kind: "forbidden", message: v?.message ?? "You may not start a run now." };
    return { kind: result, ...this.#pair("upload_open_run", v?.run, v?.entry) };
  }

  async continueRun(a: ContinueRunArgs): Promise<ContinueRunResult> {
    const v = (await this.#rpc("upload_continue_run", {
      p_member_id: a.member_id,
      p_run_id: a.run_id,
      p_previous_entry_id: a.previous_entry_id,
      p_entry_id: a.entry_id,
      p_connector_id: a.connector_id,
      p_manifest_sha256: a.manifest_sha256,
      p_connector_dir_sha256: a.connector_dir_sha256,
      p_workspace: a.workspace,
      p_session_id: a.session_id,
      p_harness_version: a.harness_version,
      p_tracer_version: a.tracer_version,
      p_client_ip: a.client_ip,
      p_client_country: a.client_country,
      p_at: a.at,
    })) as { result?: unknown; run?: unknown; entry?: unknown } | null;
    const result = this.#word("upload_continue_run", v?.result, ["created", "existing", "conflict", "closed", "not_found"] as const);
    if (result === "created" || result === "existing") return { kind: result, ...this.#pair("upload_continue_run", v?.run, v?.entry) };
    return { kind: result };
  }

  async getRun(memberId: string, runId: string, entryId: string | null): Promise<{ run: RunRow; entry: EntryRow | null } | null> {
    const v = (await this.#rpc("upload_run_position", { p_member_id: memberId, p_run_id: runId, p_entry_id: entryId })) as { run?: unknown; entry?: unknown } | null;
    if (v === null) return null;
    const run = this.#row<RunRow>("upload_run_position", v.run, "run", RUN_SHAPE);
    return { run, entry: v.entry === null ? null : this.#row<EntryRow>("upload_run_position", v.entry, "entry", ENTRY_SHAPE) };
  }

  async getChunk(memberId: string, runId: string, entryId: string, offset: number): Promise<ChunkRecord | null> {
    const v = await this.#rpc("upload_get_chunk", { p_member_id: memberId, p_run_id: runId, p_entry_id: entryId, p_offset: offset });
    return v === null ? null : this.#row<ChunkRecord>("upload_get_chunk", v, "chunk", CHUNK_SHAPE);
  }

  async storeReceipt(memberId: string, runId: string, entryId: string, chainHead: string, receipt: StartReceipt): Promise<StoreReceiptResult> {
    const v = (await this.#rpc("upload_issue_receipt", {
      p_member_id: memberId,
      p_run_id: runId,
      p_entry_id: entryId,
      p_chain_head: chainHead,
      p_receipt: receipt,
    })) as { result?: unknown; receipt?: StartReceipt } | null;
    const result = this.#word("upload_issue_receipt", v?.result, ["stored", "existing", "conflict", "closed", "not_found"] as const);
    if (result === "conflict" || result === "closed" || result === "not_found") return { kind: result };
    if (!v?.receipt) throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
    return { kind: result, receipt: v.receipt };
  }

  async abandonRun(memberId: string, runId: string, reason: string, at: string): Promise<AbandonResult> {
    const v = await this.#rpc("upload_abandon_run", { p_member_id: memberId, p_run_id: runId, p_reason: reason, p_at: at });
    return this.#word("upload_abandon_run", v, ["abandoned", "has_chunks", "closed", "not_found"] as const);
  }

  async commitChunk(a: CommitChunkArgs): Promise<CommitChunkResult> {
    const v = await this.#rpc("upload_commit_chunk", { p_member_id: a.member_id, p_run_id: a.run_id, p_chunk: a.chunk, p_stored_bytes: a.stored_bytes });
    return this.#word("upload_commit_chunk", v, ["committed", "conflict", "beyond_final", "closed", "not_found"] as const);
  }

  async recordHeartbeat(a: HeartbeatArgs): Promise<HeartbeatResult> {
    const v = await this.#rpc("upload_heartbeat", {
      p_member_id: a.member_id,
      p_run_id: a.run_id,
      p_entry_id: a.entry_id,
      p_seq: a.seq,
      p_head: a.head,
      p_offset_local: a.offset_local,
      p_final: a.final,
      p_repo_head: a.repo_head,
      p_recording: a.recording,
      p_model_requests: a.model_requests,
      p_client_ip: a.client_ip,
      p_client_country: a.client_country,
      p_at: a.at,
    });
    return this.#word("upload_heartbeat", v, ["recorded", "final_committed", "final_existing", "final_conflict", "closed", "not_found"] as const);
  }

  async closeRun(memberId: string, runId: string, at: string): Promise<RunStatus | null> {
    const v = await this.#rpc("upload_close_run", { p_member_id: memberId, p_run_id: runId, p_at: at });
    return v === null ? null : this.#word("upload_close_run", v, STATUS_WORDS);
  }

  async runsForTick(now: string): Promise<TickRun[]> {
    const v = await this.#rpc("upload_runs_for_tick", { p_at: now });
    if (!Array.isArray(v)) {
      console.error(`supabase upload_runs_for_tick: expected a list of runs`);
      throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
    }
    return v.map((row) => this.#row<TickRun>("upload_runs_for_tick", row, "run", TICK_SHAPE));
  }

  async queueNotices(notices: NoticeArgs[]): Promise<number> {
    if (notices.length === 0) return 0;
    const v = await this.#rpc("upload_queue_notices", { p_notices: notices });
    if (typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= notices.length) return v;
    console.error(`supabase upload_queue_notices: unexpected result ${JSON.stringify(v)?.slice(0, 200)}`);
    throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
  }

  async claimNotices(limit: number, now: string): Promise<ClaimedNotice[]> {
    const v = await this.#rpc("upload_claim_notices", { p_limit: limit, p_now: now });
    if (!Array.isArray(v)) {
      console.error(`supabase upload_claim_notices: expected a list of notices`);
      throw new ApiError(502, "upstream_error", "The database answered with something unexpected.");
    }
    return v.map((row) => this.#row<ClaimedNotice>("upload_claim_notices", row, "notice", NOTICE_SHAPE));
  }

  async finishNotice(id: number, outcome: NoticeOutcome, to: string | null, detail: string, now: string): Promise<void> {
    const v = await this.#rpc("upload_finish_notice", { p_id: id, p_outcome: outcome, p_to: to, p_detail: detail, p_now: now });
    this.#word("upload_finish_notice", v, ["ok", "not_found"] as const);
  }

  async recordFinalCommit(memberId: string, runId: string, commit: string, at: string): Promise<void> {
    const v = await this.#rpc("upload_record_final_commit", { p_member_id: memberId, p_run_id: runId, p_commit: commit, p_at: at });
    this.#flag("upload_record_final_commit", v);
  }

  async sealRun(a: SealArgs): Promise<SealResult> {
    const v = (await this.#rpc("upload_seal_run", {
      p_member_id: a.member_id,
      p_run_id: a.run_id,
      p_entry_id: a.entry_id,
      p_seal_sha256: a.seal_sha256,
      p_baseline_sha256: a.baseline_sha256,
      p_seal_key: a.seal_key,
      p_baseline_key: a.baseline_key,
      p_sealed_at: a.sealed_at,
    })) as { result?: unknown; run_status?: unknown } | null;
    const result = this.#word("upload_seal_run", v?.result, ["sealed", "already_sealed", "conflict", "no_final", "chain_behind", "closed", "not_found"] as const);
    if (result === "sealed" || result === "already_sealed") {
      return { kind: result, run_status: this.#word("upload_seal_run", v?.run_status, STATUS_WORDS) };
    }
    return { kind: result };
  }
}
