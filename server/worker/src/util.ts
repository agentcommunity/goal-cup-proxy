// Small pure helpers: hex, time, ids, hashing, field validation. No Worker-only APIs, so the
// Node unit tests can import this file directly.

import { malformed } from "./errors.ts";

const HEX64 = /^[0-9a-f]{64}$/;
const ENTRY_ID = /^e[0-9a-f]{16}$/;
export const RUN_ID = /^r[0-9a-f]{16}$/;
const TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;
// Exactly what the contract allows: a public GitHub repository URL, no trailing slash, no .git.
// docs/UPLOAD-API.md: the owner is ASCII letters, digits and `-`; the repository also `.` and
// `_`, and is never `.` or `..`.
export const REPO_URL = /^https:\/\/github\.com\/[A-Za-z0-9-]+\/(?!\.{1,2}$)[A-Za-z0-9._-]+$/;

export const isHex64 = (s: unknown): s is string => typeof s === "string" && HEX64.test(s);
export const isEntryId = (s: unknown): s is string => typeof s === "string" && ENTRY_ID.test(s);
export const isTime = (s: unknown): s is string => typeof s === "string" && TIME.test(s);

export function toHex(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

export function fromHex(hex: string): Uint8Array {
  if (hex.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(hex)) throw new Error("not hex");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export async function sha256Hex(data: string | Uint8Array): Promise<string> {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  return toHex(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes as BufferSource)));
}

/** Equal strings, without the time depending on where they first differ (both sides are hashed first). */
export async function constantTimeEqual(a: string, b: string): Promise<boolean> {
  const [x, y] = await Promise.all([sha256Hex(a), sha256Hex(b)]);
  let diff = 0;
  for (let i = 0; i < x.length; i++) diff |= x.charCodeAt(i) ^ y.charCodeAt(i);
  return diff === 0;
}

/** "YYYY-MM-DDTHH:MM:SSZ": RFC 3339 UTC to the second, the only time shape on the wire. */
export function formatTime(d: Date): string {
  return d.toISOString().slice(0, 19) + "Z";
}

export function addSeconds(d: Date, seconds: number): Date {
  return new Date(d.getTime() + Math.round(seconds) * 1000);
}

/** Prefix + 16 lowercase hex digits from the platform CSPRNG: "r…" run ids, "e…" entry ids. */
export function newId(prefix: "r" | "e"): string {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  return prefix + toHex(bytes);
}

// ---- JSON body field access ---------------------------------------------------------------

export function asObject(v: unknown, what: string): Record<string, unknown> {
  if (typeof v !== "object" || v === null || Array.isArray(v)) throw malformed(`${what} must be a JSON object.`);
  return v as Record<string, unknown>;
}

/** A string field, 1..max characters, well-formed Unicode (a lone surrogate cannot be signed). */
export function strField(o: Record<string, unknown>, name: string, max: number, allowEmpty = false): string {
  const v = o[name];
  if (typeof v !== "string") throw malformed(`${name} must be a string.`);
  if (!allowEmpty && v.length === 0) throw malformed(`${name} must not be empty.`);
  if (v.length > max) throw malformed(`${name} is longer than ${max} characters.`);
  if (!v.isWellFormed()) throw malformed(`${name} is not well-formed Unicode.`);
  return v;
}

/**
 * The Goal Cup Proxy's version from a device or run body: `proxy_version` (the current name) or
 * `tracer_version` (the name older releases send). Either is accepted; when both are sent they must
 * be equal. Null is absent, as for other optional fields. At most 64 characters. Stored as
 * `tracer_version` (the database's name, for now).
 */
export function proxyVersionField(o: Record<string, unknown>): string {
  const p = o.proxy_version == null ? null : strField(o, "proxy_version", 64);
  const t = o.tracer_version == null ? null : strField(o, "tracer_version", 64);
  if (p !== null && t !== null && p !== t) throw malformed("proxy_version and tracer_version differ; send one.");
  const version = p ?? t;
  if (version === null) throw malformed("proxy_version must be a string.");
  return version;
}

/** An optional boolean field: `fallback` when missing, 400 `malformed` for anything but true or false (null included). */
export function boolField(o: Record<string, unknown>, name: string, fallback: boolean): boolean {
  const v = o[name];
  if (v === undefined) return fallback;
  if (typeof v !== "boolean") throw malformed(`${name} must be true or false.`);
  return v;
}

/**
 * The version rule: the first `<digits>.<digits>.<digits>` in the text; whatever follows it
 * (`-dev`, `+build`, a fourth number) is ignored. Null when there is none, i.e. the version is unknown.
 */
export function versionTriple(text: string): [number, number, number] | null {
  const m = /(\d+)\.(\d+)\.(\d+)/.exec(text);
  if (m === null) return null;
  const v = [Number(m[1]), Number(m[2]), Number(m[3])] as [number, number, number];
  return v.every(Number.isSafeInteger) ? v : null;
}

export function hexField(o: Record<string, unknown>, name: string): string {
  const v = o[name];
  if (!isHex64(v)) throw malformed(`${name} must be 64 lowercase hex digits.`);
  return v;
}

/** Non-negative integer in canonical decimal form that is exactly representable as a JS number. */
export function parseSafeUint(s: string): number | null {
  if (!/^(0|[1-9][0-9]{0,15})$/.test(s)) return null;
  const n = Number(s);
  return Number.isSafeInteger(n) ? n : null;
}

/** Read a request body of unknown length without ever holding more than `max` bytes. */
export async function readBodyCapped(request: Request, max: number): Promise<Uint8Array | null> {
  if (!request.body) return new Uint8Array(0);
  const reader = request.body.getReader();
  const parts: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > max) {
      await reader.cancel();
      return null; // caller answers 413
    }
    parts.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.byteLength;
  }
  return out;
}
