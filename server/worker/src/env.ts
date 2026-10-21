// The Worker's bindings, variables and secrets. See README.md for what each one is.

export interface Env {
  /** R2 bucket: raw chunks under traces/…; (local backend only) state under state/…. */
  TRACES: R2Bucket;

  /** Event that new competition runs belong to, e.g. "cup". */
  EVENT_ID: string;
  /** Event that practice runs belong to, e.g. "practice". */
  PRACTICE_EVENT_ID: string;
  /** Event that organizer test runs belong to (mode `test`); unset: this server opens no test run. */
  TEST_EVENT_ID?: string;
  /** Run length in hours (default 168). */
  RUN_HOURS?: string;
  /** Practice run length in minutes (default 10). */
  PRACTICE_RUN_MINUTES?: string;
  /**
   * The most compressed bytes one run may hold in R2, over all its entries (default 68719476736,
   * 64 GiB). A chunk that would take the run past it is refused with 413 quota_exceeded and nothing
   * is stored; goalcup keeps the data and retries, so raising this resumes the upload.
   */
  RUN_MAX_COMPRESSED_BYTES?: string;
  /**
   * Tests only, DB_BACKEND=local only: run length in seconds; seconds after a run ends until it is
   * closed (default 24 h); and the competition start window (RFC 3339, `2026-11-06T09:00:00Z`;
   * an unset edge is open). In production the window is the event's row in the database.
   */
  RUN_SECONDS?: string;
  CLOSE_AFTER_SECONDS?: string;
  START_WINDOW_OPENS?: string;
  START_WINDOW_CLOSES?: string;

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

  /**
   * Durable Object namespace (class DeviceLimiter) that rate-limits POST /v1/device and
   * /v1/device/token per client address (src/ratelimit.ts). Unbound: no limit with
   * DB_BACKEND=local; every device call answers 500 misconfigured on the Supabase backend.
   */
  DEVICE_LIMITER?: DurableObjectNamespace;
  /**
   * Calls a minute: starts per client address (default 120), polls per device code (default 30),
   * and polls per client address, an abuse cap only (default 1200).
   */
  DEVICE_START_PER_MINUTE?: string;
  DEVICE_POLL_PER_MINUTE?: string;
  DEVICE_TOKEN_PER_MINUTE?: string;
  /**
   * Secret, at least 32 characters: the HMAC key of every rate-limit key. Required with
   * DEVICE_LIMITER (500 misconfigured without it), except with DB_BACKEND=local.
   */
  RATE_LIMIT_KEY?: string;

  /** "on" answers every call 503 + Retry-After. */
  MAINTENANCE?: string;
  MAINTENANCE_RETRY_AFTER?: string;

  /** "on" checks with the GitHub API that repo_url is public. Anything else: no check. */
  GITHUB_CHECK?: string;
  GITHUB_TOKEN?: string;

  /**
   * The run emails (src/mail.ts). Mail is off while RESEND_API_KEY (a secret) is unset. NOTICE_FROM
   * is the sender, e.g. `Agent Community <updates@member.agentcommunity.org>`. With
   * NOTICE_RECIPIENT_OVERRIDE set every notice goes to that one address instead of the member; a
   * staging Worker refuses to send without it.
   */
  RESEND_API_KEY?: string;
  NOTICE_FROM?: string;
  NOTICE_RECIPIENT_OVERRIDE?: string;

  /** Secret: Ed25519 seed, 64 hex characters. */
  RECEIPT_SIGNING_KEY: string;
  /** Secrets for the Supabase backend. */
  SUPABASE_URL?: string;
  SUPABASE_KEY?: string;
}
