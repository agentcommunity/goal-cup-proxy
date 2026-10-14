// The Worker's bindings, variables and secrets. See README.md for what each one is.

export interface Env {
  /** R2 bucket: raw chunks under traces/…; (local backend only) state under state/…. */
  TRACES: R2Bucket;

  /** Event that new runs belong to, e.g. "practice". */
  EVENT_ID: string;
  /** Run length in hours (default 168) and grace for late uploads after the deadline (default 48). */
  RUN_HOURS?: string;
  GRACE_HOURS?: string;

  /** "supabase" (default) or "local" (wrangler dev and tests only). */
  DB_BACKEND?: string;
  /** Local backend only: "token1:member1,token2:member2". */
  DEV_TOKENS?: string;

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
