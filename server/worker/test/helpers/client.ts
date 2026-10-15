// A tiny HTTP client for the contract, and helpers to build chunks and check signatures.

import { createHash } from "node:crypto";
import { request as httpRequest } from "node:http";
import { zstdCompressSync } from "node:zlib";
import { ackSigningBytes, receiptSigningBytes, type Ack, type StartReceipt } from "../../src/sign.ts";
import { fromHex } from "../../src/util.ts";

export const GENESIS = "0".repeat(64);
export const sha256 = (b: Uint8Array | string) => createHash("sha256").update(b).digest("hex");

export interface Reply {
  status: number;
  headers: Headers;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  body: any;
}

/** How a request reaches the Worker: `fetch` over HTTP, or the Worker's own `fetch` in this process. */
export type Fetcher = (input: string, init: RequestInit) => Promise<Response>;

export class Client {
  readonly base: string;
  readonly #fetch: Fetcher;
  constructor(base: string, fetcher: Fetcher = fetch) {
    this.base = base;
    this.#fetch = fetcher;
  }

  async call(method: string, path: string, opts: { token?: string | null; json?: unknown; body?: Uint8Array; headers?: Record<string, string> } = {}): Promise<Reply> {
    const headers: Record<string, string> = { ...(opts.headers ?? {}) };
    if (opts.token) headers.authorization = `Bearer ${opts.token}`;
    let body: BodyInit | undefined;
    if (opts.json !== undefined) {
      headers["content-type"] = "application/json";
      body = JSON.stringify(opts.json);
    } else if (opts.body !== undefined) {
      body = opts.body as BodyInit;
    }
    const res = await this.#fetch(this.base + path, { method, headers, body });
    const text = await res.text();
    let parsed: unknown = text;
    try {
      parsed = JSON.parse(text);
    } catch {
      // leave as text
    }
    return { status: res.status, headers: res.headers, body: parsed };
  }

  /** Raw HTTP request, for the cases fetch will not let us send (no Content-Length, a lying one). */
  raw(method: string, path: string, headers: Record<string, string>, bodyChunks: Uint8Array[]): Promise<Reply> {
    const u = new URL(this.base + path);
    return new Promise((resolve, reject) => {
      const req = httpRequest({ host: u.hostname, port: u.port, path: u.pathname, method, headers }, (res) => {
        const parts: Buffer[] = [];
        res.on("data", (d) => parts.push(d));
        res.on("end", () => {
          const text = Buffer.concat(parts).toString();
          const h = new Headers();
          for (const [k, v] of Object.entries(res.headers)) if (typeof v === "string") h.set(k, v);
          let parsed: unknown = text;
          try {
            parsed = JSON.parse(text);
          } catch {
            // text
          }
          resolve({ status: res.statusCode ?? 0, headers: h, body: parsed });
        });
      });
      req.on("error", reject);
      for (const c of bodyChunks) req.write(c);
      req.end();
    });
  }
}

export const APPROVAL_KEY = "test-approval-key";

/** `POST /v1/device`, the way the tracer sends it. */
export const deviceBody = (overrides: Record<string, unknown> = {}) => ({
  purpose: "run",
  mode: "competition",
  connector_id: "codex-cli",
  tracer_version: "0.1.0",
  ...overrides,
});

/** The test-only approval of a user code by a member (`approve: false` denies). */
export const approve = (api: Client, user_code: string, member_id: string, approve = true, key: string | null = APPROVAL_KEY) =>
  api.call("POST", "/v1/device/approve", { token: key, json: { user_code, member_id, approve } });

/** The whole device flow in one go: a run credential (`gcr_` + 64 hex) for `member`, still unbound. */
export async function credentialFor(api: Client, member: string, mode: "competition" | "practice" | "test" = "competition"): Promise<string> {
  const d = await api.call("POST", "/v1/device", { json: deviceBody({ mode }) });
  if (d.status !== 200) throw new Error(`device: ${d.status} ${JSON.stringify(d.body)}`);
  const a = await approve(api, d.body.user_code, member);
  if (a.status !== 200) throw new Error(`approve: ${a.status} ${JSON.stringify(a.body)}`);
  const t = await api.call("POST", "/v1/device/token", { json: { device_code: d.body.device_code } });
  if (t.status !== 200) throw new Error(`token: ${t.status} ${JSON.stringify(t.body)}`);
  return t.body.credential as string;
}

export function openBody(overrides: Record<string, unknown> = {}) {
  return {
    connector_id: "codex-cli",
    manifest_sha256: "a".repeat(64),
    connector_dir_sha256: "c".repeat(64),
    workspace: "/Users/p/goal-1/run-x/workspace — ünï",
    session_id: "6f1c2d3e-0000-4000-8000-000000000001",
    harness_version: "0.153.0",
    tracer_version: "0.1.0",
    repo_url: "https://github.com/someone/goal-cup-entry",
    ...overrides,
  };
}

export interface TestChunk {
  body: Buffer; // zstd frame
  headers: Record<string, string>;
  length: number;
  head: string;
  lastSeq: number;
}

/** A syntactically valid chunk: a zstd frame of n JSON lines. The server checks headers, not chain content. */
export function makeChunk(entryId: string, firstSeq: number, count: number, prev: string, tag: string): TestChunk {
  const lines: string[] = [];
  for (let i = 0; i < count; i++) lines.push(JSON.stringify({ seq: firstSeq + i, tag, pad: "x".repeat(200) }));
  const raw = Buffer.from(lines.join("\n") + "\n");
  const body = zstdCompressSync(raw);
  const lastSeq = firstSeq + count - 1;
  const head = sha256(`${tag}:${lastSeq}`);
  return {
    body,
    length: raw.length,
    head,
    lastSeq,
    headers: {
      "content-type": "application/zstd",
      "goalcup-entry-id": entryId,
      "goalcup-length": String(raw.length),
      "goalcup-sha256": sha256(body),
      "goalcup-first-seq": String(firstSeq),
      "goalcup-last-seq": String(lastSeq),
      "goalcup-prev": prev,
      "goalcup-head": head,
    },
  };
}

async function verify(publicKeyHex: string, signatureHex: string, bytes: Uint8Array): Promise<boolean> {
  const key = await crypto.subtle.importKey("raw", fromHex(publicKeyHex) as BufferSource, { name: "Ed25519" }, false, ["verify"]);
  return crypto.subtle.verify({ name: "Ed25519" }, key, fromHex(signatureHex) as BufferSource, bytes as BufferSource);
}

export const verifyReceipt = (r: StartReceipt, publicKeyHex: string) => verify(publicKeyHex, r.signature, receiptSigningBytes(r.claims));
export const verifyAck = (a: Ack, publicKeyHex: string) => verify(publicKeyHex, a.server_signature, ackSigningBytes(a));
