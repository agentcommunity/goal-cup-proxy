// The Worker's bindings, variables and secrets. See README.md for what each one is.

export interface Env {
  /** R2 bucket: raw chunks under traces/…; (local backend only) state under state/…. */
  TRACES: R2Bucket;

  /** Event that new runs belong to, e.g. "practice". */
  EVENT_ID: string;
  /** Run length in hours (default 168). */
  RUN_HOURS?: string;
  /** Tests only, DB_BACKEND=local only: run length in seconds, and seconds after the deadline until an unsealed run is abandoned (default 24 h). */
  RUN_SECONDS?: string;
  ABANDON_AFTER_SECONDS?: string;

  /** "supabase" (default) or "local" (wrangler dev and tests only). */
  DB_BACKEND?: string;

  /** Where the member approves a device request: the website's page, without a query string. */
  APPROVAL_URL?: string;
  /**
   * Secret, tests only: switches on `POST /v1/device/approve`, which approves a device request as
   * any member. Honoured only with DB_BACKEND=local or STAGING=on; set anywhere else, every call
   * answers 500 misconfigured. Leave it unset in production.
   */
  TEST_APPROVAL_KEY?: string;
  /** "on" marks a staging deployment (the only non-local place TEST_APPROVAL_KEY may be set). */
  STAGING?: string;

  /** "on" answers every call 503 + Retry-After. */
  MAINTENANCE?: string;
  MAINTENANCE_RETRY_AFTER?: string;

  /** "on" checks with the GitHub API that repo_url is public. Anything else: no check. */
  GITHUB_CHECK?: string;
  GITHUB_TOKEN?: string;

  /** Secret: Ed25519 seed, 64 hex characters. */
  RECEIPT_SIGNING_KEY: string;
  /** Secrets for the Supabase backend. */
  SUPABASE_URL?: string;
  SUPABASE_KEY?: string;
}
