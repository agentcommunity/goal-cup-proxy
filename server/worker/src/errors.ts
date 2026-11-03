// Error type and JSON response helpers. Every error body is {"error": "<code>", "message": "<sentence>"}
// exactly as docs/UPLOAD-API.md lists them.

export type ErrorCode =
  | "malformed" // 400
  | "unauthorized" // 401
  | "forbidden" // 403
  | "not_found" // 404
  | "method_not_allowed" // 405 (not in the contract; only for a wrong verb on a known path)
  | "conflict" // 409
  | "closed" // 410
  | "too_large" // 413
  | "digest_mismatch" // 422
  | "rate_limited" // 429: the device flow's per-address limit
  | "maintenance" // 503
  | "awaiting_final" // 503: past deadline_at, before the final heartbeat
  | "misconfigured" // 500: the deployment is wrong, not the request
  | "internal" // 500
  | "upstream_error"; // 502: Supabase, GitHub or R2 failed; goalcup retries with backoff

export class ApiError extends Error {
  readonly status: number;
  readonly code: ErrorCode;
  readonly headers: Record<string, string>;

  constructor(status: number, code: ErrorCode, message: string, headers: Record<string, string> = {}) {
    super(message);
    this.status = status;
    this.code = code;
    this.headers = headers;
  }
}

export const malformed = (message: string): ApiError => new ApiError(400, "malformed", message);
export const unauthorized = (message: string): ApiError => new ApiError(401, "unauthorized", message);
export const notFound = (message: string): ApiError => new ApiError(404, "not_found", message);
export const conflict = (message: string): ApiError => new ApiError(409, "conflict", message);
export const closed = (message: string): ApiError => new ApiError(410, "closed", message);
export const tooLarge = (message: string): ApiError => new ApiError(413, "too_large", message);

export function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store", ...headers },
  });
}

export function errorResponse(e: ApiError): Response {
  return json(e.status, { error: e.code, message: e.message }, e.headers);
}
