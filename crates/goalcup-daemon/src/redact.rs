//! Redaction at capture (D14).
//!
//! The pattern set is public, it is applied before persistence, and every altered
//! span is recorded in the same hash-covered envelope as the bytes it describes,
//! "so hashes are computed over redacted bytes consistently". This module is the whole of
//! that: [`Redactor::apply`] takes a record's raw bytes and returns the bytes the chain
//! will hash. The raw bytes never leave this stage: the caller's raw record exists for this
//! call and enters no hash.
//!
//! **The order is fixed and cannot be reordered.** Redact, then hash, then persist (the V1
//! path also partitioned first and buffered last; D48 removed it). Redacting after hashing
//! would publish a digest of the secret; persisting before redacting would store it.
//!
//! **No interpretation.** A pattern match is a byte comparison against a regular
//! expression this file carries or a connector rule's form derives, never a judgement about what
//! the matched text means. Whether a removed span was in fact a live credential is not a
//! question this module asks or answers.
//!
//! **One [`Redactor`] for every record.** `apply` takes bytes and does not know which store
//! produced them.
//!
//! **Bounded memory.** `apply` holds the caller's borrowed record (the caller bounds its
//! size), one output buffer of the same order, and one interval per
//! match. It never holds a batch and never copies one.
//!
//! The replacement publishes only the pattern kind. The hash-covered audit publishes that
//! kind and the original byte offsets; it publishes no salt or secret-derived digest that
//! could serve as a confirmation oracle for a low-entropy credential.

use crate::DaemonError;
use goalcup_protocol::harness::{ExtraPattern, HarnessManifest};
use goalcup_protocol::redaction::RedactionAuditSpan;
use std::sync::OnceLock;

/// Protocol-v5 capture redaction: the same built-in pattern set, plus optionally a
/// connector's `extra_patterns` through [`compile_extra`], applied to ordered byte streams
/// with bounded state. It shares [`patterns`], [`candidates`], [`select`]
/// and [`marker`] with [`Redactor::apply`]; it has no pattern list of its own.
pub mod stream;

/// One named rule in the public set. `kind` is what reaches [`RedactionAuditSpan::kind`] and the
/// replacement marker, so it is the name a participant reads in the audit.
///
/// `regex::bytes::Regex`, not `regex::Regex`: a record's raw bytes are what is redacted,
/// and a JSONL line a harness wrote is not guaranteed to be valid UTF-8. Transcribing it
/// to a `str` first would either shift every offset (lossy replacement) or force
/// non-UTF-8 records to pass through unredacted — a privacy hole in the one place the
/// bytes are least inspectable. Under the crate's default Unicode mode every match is
/// still guaranteed to be valid UTF-8.
pub struct Pattern {
    pub kind: String,
    pub re: regex::bytes::Regex,
    /// Which part of a match is the span: `0` for the whole match, `1` for the first
    /// capture group.
    ///
    /// **A pattern may declare a capture group, and then the span is that group alone.**
    /// A rule that has to name its key in order to find its value would otherwise remove
    /// the key too: `{"api_key":"…"}` would become `{<redacted:…>}`, which is no longer JSON —
    /// and JSON is not only this stage's input shape but its output shape, since the chain
    /// and the verifier's parsers both read redacted bytes. With the value in group 1 the
    /// key, the colon and both quotes stay put and the record still parses. `start` and
    /// `end` name the group's bytes, so the audit describes exactly what left.
    ///
    /// Set from the compiled regex: a literal with at least one explicit group means group
    /// 1, otherwise the whole match. So a rule that wants the whole match writes its groups
    /// non-capturing, `(?:…)` — which is what the shell/TOML literal does, deliberately,
    /// because `spikes/redact.py` replaces `m.group(0)` and the committed corpus carries
    /// the markers that produced.
    pub span_group: usize,
    /// Soft: loses every overlap and keeps what the winner did not cover (see [`select`]).
    ///
    /// Set only by [`compile`], which builds the shared set, and only for the shared literal
    /// whose kind is [`SOFT_KIND`]. [`compile_extra`] always sets it `false`, so a connector's
    /// own pattern that reuses the name `masked_key` is an ordinary hard rule.
    pub soft: bool,
    /// Block: a match is one whole block that no claim inside it or across its edge may
    /// shadow (see [`select`] and [`BLOCK_KIND`]). Set by [`compile`] for the PEM rule and by
    /// [`RunSecrets`] for each exact value; [`compile_extra`] always sets it `false`.
    pub block: bool,
}

/// The kind of the one soft rule in the shared set. A soft rule loses every overlap, whatever
/// its position in the rule list, and keeps what the winner did not cover: adding it to a
/// policy can only add redaction, never change or remove what the other rules redact. Only
/// [`compile`] reads this name; see [`Pattern::soft`] and [`select`].
pub(crate) const SOFT_KIND: &str = "masked_key";

/// Never the regex: its source spells a run's exact secret value for a [`RunSecrets`] rule.
impl std::fmt::Debug for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pattern")
            .field("kind", &self.kind)
            .field("span_group", &self.span_group)
            .field("soft", &self.soft)
            .field("block", &self.block)
            .finish_non_exhaustive()
    }
}

/// The kind of the one block rule in the shared set (the run's exact values, [`RunSecrets`],
/// are blocks too, but are not in the shared set): a PEM private key, from its `BEGIN` line
/// to the first `END` line that starts within [`PEM_BODY_MAX`] bytes, else those bytes (a key
/// cut off before its `END`, or an `END` split across two strings), extended to the end of a
/// character they cut. [`candidates`] then keeps only the parts of that range inside JSON
/// strings ([`string_pieces`]). Only [`compile`] and [`candidates`] read this name. A block
/// claim absorbs every hard claim it contains or that crosses its edge, so a key is never left
/// raw because something inside it, or touching it, was claimed first; see [`select`] and
/// [`Pattern::block`].
pub(crate) const BLOCK_KIND: &str = "private_key";

/// The most bytes the `private_key` rule takes after its `BEGIN` line: an `END` line must start
/// within them, and without one the block is them. Both arms of the regex literal spell it
/// (`{0,15872}`), and a test in `capture_redaction.rs` pins that.
///
/// Why 15.5 KiB: the longest key body to expect is RSA-16384, the largest RSA key OpenSSH
/// generates. Measured on two PKCS#8 files from `openssl genpkey` (12,628 and 12,632 bytes), the
/// bytes between the `BEGIN` and `END` lines are at most: 12,579 raw; 14,529 as Claude Code's
/// Read tool prints it (`N→` on each of its 196 lines) JSON-escaped; 14,724 escaped twice;
/// 15,114 escaped once with the arrow as `\u2192`; 15,504 escaped twice that way (Python's
/// default `json.dumps`, twice). Every one of these fits. Escaped three times that way it is
/// 16,284, which does not, and the tail past the bound leaks. The bound stays 512 bytes under
/// the stream's 16 KiB hold ([`stream::MAX_HELD_BYTES`]), which is room for a `BEGIN` and an
/// `END` line, so a block is decided below the hold as this regex decides it. The hold is not
/// raised: it is in "Keep as is".
pub const PEM_BODY_MAX: usize = 15_872;

/// What an invalid byte reads as when the rules match: one ASCII `0`. Every key body class in
/// the set takes it (it is the one character `[A-Za-z0-9]`, `[0-9A-Z]` and `[a-f0-9]` share)
/// and no shared rule's fixed prefix starts or completes with it. So a key with an invalid byte
/// inside its body is still one match, where the Unicode classes would end it at that byte and
/// leave the rest raw (H-20).
pub(crate) const INVALID_BYTE_AS: u8 = b'0';

/// The bytes the rules match on: `bytes`, with every byte that is not part of a valid UTF-8
/// sequence read as [`INVALID_BYTE_AS`]. One byte stands for one byte, so every offset is an
/// offset in `bytes`, and the replacement still copies the original bytes. Borrowed (no copy)
/// when `bytes` is valid UTF-8, which is nearly every record.
///
/// "Not valid" is per byte, the stream's `decode` rule: a lead byte whose sequence breaks is
/// one invalid byte, each continuation byte after it is another, and a sequence cut off at
/// the end of `bytes` is invalid bytes too. Called on a stream's held bytes, which start and
/// end on a decoded unit, it reads each byte as the whole stream would.
pub(crate) fn match_view(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let Err(mut error) = std::str::from_utf8(bytes) else {
        return std::borrow::Cow::Borrowed(bytes);
    };
    let mut view = bytes.to_vec();
    let mut at = 0;
    loop {
        let from = at + error.valid_up_to();
        let bad = error.error_len().unwrap_or(bytes.len() - from);
        view[from..from + bad].fill(INVALID_BYTE_AS);
        at = from + bad;
        match std::str::from_utf8(&bytes[at..]) {
            Ok(_) => return std::borrow::Cow::Owned(view),
            Err(next) => error = next,
        }
    }
}

/// The shared set: `spikes/redact.py`'s ten original rules in its order, plus `masked_key`
/// (listed right after `openai_key` here and last in the script; a soft rule never wins an
/// overlap, so its position does not matter, see [`select`]), then seven fixed vendor
/// prefixes, then the three forms of the tracer's own `proxy_credential`. The committed
/// fixtures were redacted with that script and the verifier's parsers read those bytes, so
/// the ten keep the script's spelling and their place in its list. The script carries the
/// three `proxy_credential` literals too, after its ten and before its last rule.
///
/// Two deliberate deltas from the script. `generic_secret_assignment` is one kind spelled as
/// two literals, and the seven vendor prefixes are new: shapes the script predates.
/// `tests/it/redaction.rs` holds the whole comparison (every script regex against this list,
/// with the differences written out) so that either side drifting fails a test.
///
/// The split exists because the script's rule requires `[=:]` immediately after the key
/// word, so it never fires on a quoted JSON key — and JSON is the only shape this stage is
/// ever handed. `{"api_key":"…"}` would produce zero spans. Widening the script's value class
/// to `[A-Za-z0-9_\-+/=.]{16,}` cannot reach a quoted key and does reach into ordinary
/// source: `let token = entry.keys.signing_key` would become a span. So:
///
/// 1. the JSON form matches `"<key>"\s*:\s*"<value>"`, where the value is sixteen or more
///    bytes carrying no quote and no backslash. The value's own closing quote bounds the
///    match, which is what keeps it off dotted code. **The value is a capture group, and
///    that group is the span** ([`Pattern::span_group`]): the key, the colon and the
///    quotes stay, so `{"api_key":"<redacted:…>"}` is still JSON for every reader
///    downstream of this stage;
/// 2. the shell/TOML form keeps the script's shape and the script's original narrow value
///    class, `[A-Za-z0-9_\-]{16,}`, and — also deliberately — the script's whole-match
///    span: its groups are non-capturing, because `redact.py` replaces `m.group(0)` and the
///    committed corpus the verifier's parsers read carries the markers that produced. It is a
///    shell export or a TOML line, not a JSON field; there is no structure to keep intact.
///
/// Both carry the kind `generic_secret_assignment`, so a participant reads one rule in the
/// log whichever literal fired.
const SHARED: &[(&str, &str)] = &[
    // ── spikes/redact.py's PATTERNS, in its order ──────────────────────────────────────
    ("anthropic_key", r"sk-ant-[A-Za-z0-9_\-]{20,}"),
    ("openai_key", r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}"),
    // A provider key echoed back masked: `key[:8]…key[-4:]` in a request dump or a 401 body
    // (`sk-ab***wxyz`, OpenAI's `sk-R7aMV` + 39 stars + `oKYo`, Hermes' `sk-0...cdef`).
    // Every full-key rule above needs a long contiguous value and misses it. Not in
    // `redact.py`'s original ten: the script has listed it since D41, last (that script
    // applies its rules in sequence, so last is how it is soft there). Here it sits after
    // `openai_key`, which is only a place in the list.
    //
    // SOFT (see [`SOFT_KIND`] and [`select`]): it never wins an overlap against another rule,
    // so a longer secret glued to a mask-shaped substring stays redacted whole, as it was
    // before this rule existed; only the part of a mask no other rule covers is added.
    // No word boundary at either edge: a mask after a newline, a tab, a `_` or an ANSI colour
    // code is still a mask, and being soft the rule cannot shadow anything for matching
    // mid-token, so an anchor would only cost coverage. The right edge is open too, so a mask
    // glued to more word characters still loses the characters it names.
    (
        "masked_key",
        r"(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}|…|\\u2026|\*{3,64})[A-Za-z0-9]{3,4}",
    ),
    // The fine-grained form: `github_pat_`, then 22 characters, `_`, 59 more. Same kind, so
    // nothing downstream sees a new label. It goes first: a body can hold `ghp_` (or `gho_`...)
    // inside it, and the first rule to claim wins the overlap, so the classic rule listed
    // first would take that inner part and leave `github_pat_` and the start of the body raw.
    ("github_token", r"github_pat_[A-Za-z0-9_]{22,}"),
    ("github_token", r"gh[pousr]_[A-Za-z0-9]{20,}"),
    ("aws_access_key", r"AKIA[0-9A-Z]{16}"),
    ("bearer", r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}"),
    (
        "jwt",
        r"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
    ),
    // Bounded, because a capture stream is one text: an unbounded block with no END ran to the
    // end of the stream, a whole session. The bound is [`PEM_BODY_MAX`] bytes, any byte: a body
    // may carry `→` or an invalid byte, and a bounded Unicode `[\s\S]` compiles past the regex
    // crate's 10 MiB limit. `candidates` extends a match that ends inside a character to that
    // character's end, so the marker never leaves half of one behind.
    //
    // Any bytes, in both arms, so a key printed with line numbers, escaped, or streamed as many
    // JSON string deltas is still one block, and so is one cut off before its END or whose END
    // line is split across two deltas (the second arm takes the bound). `candidates` then cuts
    // the range at each unescaped quote and keeps only the string interiors (H-20): every
    // fragment of the key goes, and the JSON between the strings stays, so a lone BEGIN (an
    // agent writing a PEM parser, `grep` output) no longer breaks the records after it.
    (
        "private_key",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?:(?-u:[\x00-\xFF]){0,15872}?-----END [A-Z ]*PRIVATE KEY-----|(?-u:[\x00-\xFF]){0,15872})",
    ),
    ("slack_token", r"xox[baprs]-[A-Za-z0-9\-]{10,}"),
    // (i) the JSON form: a quoted key, a quoted value. The key word may sit inside an
    // `_`/`-`-joined compound (`refresh_token`, `client_secret`, `x-api-key`) but must be a
    // whole segment of it, so `tokenizer` is not a token. Each side of the key word is at most
    // 1,000 units, so a key is at most 2,008: a base64url run after a quote (a Codex
    // `encrypted_content` value) is then not a key the stream must hold to its end. Each
    // quote may follow up to seven backslashes: tool output and file contents reach every
    // native transcript and request body as JSON escaped inside a string (`\"api_key\": \"…\"`),
    // and a command's JSON argument inside that once more (`\\\"api_key\\\"`).
    (
        "generic_secret_assignment",
        r#"(?i)\\{0,7}"(?:[A-Za-z0-9_\-]{0,999}[_\-])?(?:api[_-]?key|secret|token|password)(?:[_\-][A-Za-z0-9_\-]{0,999})?\\{0,7}"\s*:\s*\\{0,7}"([^"\\]{16,})\\{0,7}""#,
    ),
    // (ii) the shell/TOML form, with redact.py's own value class. Widening this one is what
    // put source lines in spans; the JSON form above is what catches base64 secrets. The
    // opening quote may follow up to seven backslashes (`password: \"…\"` in an escaped
    // string); the span takes them, so no backslash is left dangling.
    (
        "generic_secret_assignment",
        r#"(?i)(?:api[_-]?key|secret|token|password)\s*[=:]\s*(?:\\{0,7}['"])?(?:[A-Za-z0-9_\-]{16,})"#,
    ),
    // The local part is at most 1,024 units, for the same reason as the JSON key: a bare token
    // run is not an address the stream must hold to its end. Not RFC 5321's 64: a URL
    // credential (`https://user:<token>@host`, a `git remote -v` line) is a local part here,
    // and its tokens run to about 200 characters.
    (
        "email",
        r"[A-Za-z0-9._%+\-]{1,1024}@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
    ),
    // ── then the seven fixed vendor prefixes ───────────────────────────────────────────
    ("google_key", r"AIza[A-Za-z0-9_\-]{30,}"),
    ("huggingface_token", r"hf_[A-Za-z0-9]{20,}"),
    ("npm_token", r"npm_[A-Za-z0-9]{20,}"),
    ("xai_key", r"xai-[A-Za-z0-9]{20,}"),
    ("stripe_key", r"[sr]k_live_[A-Za-z0-9]{16,}"),
    ("gitlab_token", r"glpat-[A-Za-z0-9_\-]{20,}"),
    ("digitalocean_token", r"dop_v1_[a-f0-9]{32,}"),
    // ── then the tracer's own proxy credential, in the three forms a harness holds it ──
    // The capture launch hands the harness `HTTPS_PROXY=http://goalcup:<32 lowercase
    // hex>@127.0.0.1:<port>` (`goalcup-cli` `ProxyCredential::proxy_url`). An agent that runs
    // `env`, or a harness that uploads its environment, puts that per-run secret in a
    // recorded body. Narrow on purpose: the fixed user `goalcup` and the exact secret shape,
    // so nothing else is touched. The span is `goalcup:<secret>`, so a URL keeps its `@host`.
    // (i) the userinfo, or the decoded Basic credential, in plain or JSON text;
    ("proxy_credential", r"goalcup:[a-f0-9]{32}"),
    // (ii) the same, percent-encoded once (`http%3A%2F%2Fgoalcup%3A<secret>%40…`);
    ("proxy_credential", r"goalcup%3[Aa][a-f0-9]{32}"),
    // (iii) `base64("goalcup:<secret>")`, the `Proxy-Authorization: Basic` token the harness
    // computes from the URL: `Z29hbGN1cD` encodes `goalcu` and the `p:`, `o` or `p` the top
    // bits of the first hex digit, and the 43 characters after it end the last secret byte.
    // The `==` pad carries nothing and stays.
    ("proxy_credential", r"Z29hbGN1cD[op][A-Za-z0-9+/]{43}"),
];

static COMPILED: OnceLock<Vec<Pattern>> = OnceLock::new();

/// The twenty-three shared literals — nineteen kinds, `generic_secret_assignment` and
/// `github_token` twice, `proxy_credential` three times —
/// compiled once for the process.
///
/// A literal in [`SHARED`] that will not compile is skipped rather than unwrapped: this
/// crate raises no panic in `src/`, and the signature returns a slice and not a `Result`.
/// The gate against a silently missing rule is a test, not a runtime abort —
/// `tests/it/redaction.rs` pins both the count and the exact ordered kind list, so a broken
/// literal fails the test step rather than shipping a hole.
pub fn patterns() -> &'static [Pattern] {
    COMPILED.get_or_init(|| SHARED.iter().filter_map(|(k, r)| compile(k, r)).collect())
}

/// True when `start..end` is the interior of a JSON string — a `"` immediately before it
/// and a `"` (or one escaped by up to seven backslashes) immediately after — so a bare marker dropped in its place is
/// still a JSON string and the record still parses. The escaped close is a string escaped
/// inside another string (`\"api_key\": \"<redacted:…>\"`): a quoted marker there would end
/// the outer string.
///
/// When it is false the span was the whole value (`"credits_remaining": 17.5`,
/// `"rate_limits": {…}`), and there is no pair of quotes left behind to hold the marker.
/// A bare `<redacted:…>` in a value position is not JSON at all, so the marker has to
/// bring its own quotes and the number or object becomes a string. That is a type change
/// a reader can see and account for; an unparsable record is not.
///
/// Asked only of a span a pattern claimed through a capture group. A whole-match span is
/// the shell/TOML literal and the vendor prefixes, which are not JSON fields and whose
/// bare markers the committed corpus already carries.
fn span_is_a_json_string_interior(bytes: &[u8], start: usize, end: usize) -> bool {
    let slashes = bytes[end.min(bytes.len())..]
        .iter()
        .take(8)
        .take_while(|&&b| b == b'\\')
        .count();
    let closes = slashes <= 7 && bytes.get(end + slashes) == Some(&b'"');
    start > 0 && bytes[start - 1] == b'"' && closes
}

/// Group 1 when the pattern declares one, otherwise the whole match. See
/// [`Pattern::span_group`] for why a pattern would declare one.
fn span_group_of(re: &regex::bytes::Regex) -> usize {
    // `captures_len` counts the implicit whole-match group, so >1 means at least one
    // explicit `(…)`.
    usize::from(re.captures_len() > 1)
}

fn compile(kind: &str, re: &str) -> Option<Pattern> {
    match regex::bytes::Regex::new(re) {
        Ok(re) => Some(Pattern {
            kind: kind.to_string(),
            span_group: span_group_of(&re),
            soft: kind == SOFT_KIND,
            block: kind == BLOCK_KIND,
            re,
        }),
        Err(_) => None,
    }
}

/// The kind of every exact-value rule ([`RunSecrets`]): the run's own secret (the proxy
/// credential), named apart from the shared `proxy_credential` pattern rule.
pub const RUN_SECRET_KIND: &str = "run_secret";

/// This run's own exact secret values as literal rules (D60): every form
/// `goalcup_protocol::run_secret::encoded_forms` derives of the proxy credential the launch puts
/// in the harness's `HTTPS_PROXY`. Both redactors apply them
/// after the shared and the connector rules, each as a block: a literal match absorbs every
/// claim that it contains or that crosses its edge ([`select`]), so no other rule's earlier
/// claim can leave part of a value raw.
///
/// Held in memory for one entry and never written anywhere. `Debug` prints a count. Each form is
/// compiled once, when the run's values arrive (`goalcup_protocol::run_secret::wrapped_regex`:
/// a line break may come between any two of its characters), and every redactor of the entry
/// shares the compiled rules. The forms are overwritten when the last clone drops; the compiled
/// regexes keep copies of their own bytes, which is hygiene and not a boundary (D37).
#[derive(Clone, Default)]
pub struct RunSecrets {
    forms: std::sync::Arc<Forms>,
}

/// Each form's bytes and its compiled rule, in the order the rules take after the connector's.
#[derive(Default)]
struct Forms(Vec<(Vec<u8>, regex::bytes::Regex)>);

impl Drop for Forms {
    fn drop(&mut self) {
        for (form, _) in &mut self.0 {
            goalcup_protocol::run_secret::wipe(form);
        }
    }
}

impl std::fmt::Debug for RunSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunSecrets({} forms)", self.forms.0.len())
    }
}

impl RunSecrets {
    /// The forms of `values`, compiled. Refused, without echoing a value, when there are more
    /// than [`goalcup_protocol::run_secret::MAX_RUN_SECRETS`] or one is not a usable run secret.
    pub fn new(
        values: &[goalcup_protocol::run_secret::RunSecret],
    ) -> Result<RunSecrets, DaemonError> {
        use goalcup_protocol::run_secret::{
            MAX_RUN_SECRET_BYTES, MAX_RUN_SECRETS, MIN_RUN_SECRET_BYTES, wipe, wrapped_regex,
        };
        if values.len() > MAX_RUN_SECRETS || !values.iter().all(|v| v.is_usable()) {
            return Err(DaemonError::Refused(format!(
                "run_secrets: at most {MAX_RUN_SECRETS} values, each {MIN_RUN_SECRET_BYTES} to \
                 {MAX_RUN_SECRET_BYTES} bytes of printable ASCII without a space"
            )));
        }
        let mut forms: Vec<(Vec<u8>, regex::bytes::Regex)> = Vec::new();
        for value in values {
            for mut form in goalcup_protocol::run_secret::encoded_forms(value.expose()) {
                if forms.iter().any(|(known, _)| *known == form) {
                    wipe(&mut form);
                    continue;
                }
                let source = wrapped_regex(&form);
                let compiled = regex::bytes::Regex::new(&source);
                // The source spells the value: wiped once compiled (the regex keeps its own).
                wipe(&mut source.into_bytes());
                let Ok(re) = compiled else {
                    wipe(&mut form);
                    return Err(DaemonError::Refused(
                        "run_secrets: a form does not compile".to_string(),
                    ));
                };
                forms.push((form, re));
            }
        }
        Ok(RunSecrets {
            forms: std::sync::Arc::new(Forms(forms)),
        })
    }

    /// Whether this run named no secret.
    pub fn is_empty(&self) -> bool {
        self.forms.0.is_empty()
    }

    /// Each form's bytes, in a fixed order: the order its rules take after the connector's.
    pub(crate) fn forms(&self) -> impl Iterator<Item = &[u8]> {
        self.forms.0.iter().map(|(form, _)| form.as_slice())
    }

    /// Each form as a rule, in [`Self::forms`] order, sharing the regex compiled once.
    pub(crate) fn patterns(&self) -> Vec<Pattern> {
        self.forms
            .0
            .iter()
            .map(|(_, re)| Pattern {
                kind: RUN_SECRET_KIND.to_string(),
                span_group: 0,
                soft: false,
                block: true,
                re: re.clone(),
            })
            .collect()
    }
}

/// A connector's `[redaction].extra_patterns`, in declaration order, as the rules both
/// redactors compile. Empty when `harness.toml` has no `[redaction]` table.
pub fn extra_patterns_of(manifest: &HarnessManifest) -> Vec<ExtraPattern> {
    manifest
        .redaction
        .as_ref()
        .map(|redaction| redaction.extra_patterns.clone())
        .unwrap_or_default()
}

/// The one compiler for a connector's `redaction.extra_patterns[]` rule, shared by
/// [`Redactor::for_patterns`] and [`stream::StreamRedactor::with_extra_patterns`]: it compiles
/// the regex the rule's form stands for ([`ExtraPattern::regex`]), so both redactors run the
/// same rule. The error is the reason, without the connector's name.
pub(crate) fn compile_extra(p: &ExtraPattern) -> Result<Pattern, String> {
    let re = regex::bytes::Regex::new(&p.regex()).map_err(|source| source.to_string())?;
    Ok(Pattern {
        kind: p.kind().to_string(),
        span_group: span_group_of(&re),
        soft: false,
        block: false,
        re,
    })
}

/// One record's redaction: the bytes the chain will hash and the audit spans against the
/// bytes it was handed.
#[derive(Debug)]
pub struct Redacted {
    /// The redacted payload. This — and never the input — is what leaves this stage.
    pub bytes: Vec<u8>,
    /// In positional order, indexing the **pre**-redaction bytes: every span names bytes of
    /// the input this stage was handed. (`redact.py`'s log is in a different frame: its
    /// in-pass entries are offsets in the text as the earlier rules left it, and only its
    /// residual `masked_key` entries are original offsets.) Empty when nothing matched, in
    /// which case `bytes` is the input byte for byte and the caller omits the inline audit.
    pub spans: Vec<RedactionAuditSpan>,
}

/// The shared set, plus whatever the connector's `harness.toml` adds.
#[derive(Debug)]
pub struct Redactor {
    /// The connector's own `redaction.extra_patterns[]`, applied after the shared set, then
    /// the run's exact values ([`RunSecrets`]), applied last.
    /// The shared literals are not copied in: they live in [`patterns`], compiled once.
    extra: Vec<Pattern>,
}

impl Default for Redactor {
    fn default() -> Self {
        Redactor::new()
    }
}

impl Redactor {
    /// The shared set and nothing else.
    pub fn new() -> Redactor {
        Redactor { extra: Vec::new() }
    }

    /// The shared set plus a connector's `redaction.extra_patterns[]` (see
    /// [`extra_patterns_of`]) — how Codex's `plan_type` and credit fields, measured fifteen
    /// times in a ten-minute session, and Claude's billing header block are stripped at
    /// capture rather than at publication (D26 strips them before publication, these before
    /// the upload). `id` names the connector in the error.
    ///
    /// The capture path streams through [`stream::StreamRedactor`]; this whole-record form is
    /// the independent oracle its tests compare against.
    ///
    /// A pattern that will not compile is an error and not a skip. A rule a participant
    /// can read in `harness.toml` but that never fires is a privacy hole with a paper trail
    /// claiming otherwise.
    pub fn for_patterns(id: &str, patterns: &[ExtraPattern]) -> Result<Redactor, DaemonError> {
        let mut extra = Vec::with_capacity(patterns.len());
        for p in patterns {
            extra.push(compile_extra(p).map_err(|reason| {
                DaemonError::Config(format!(
                    "connector {id} redaction pattern {}: {reason}",
                    p.kind()
                ))
            })?);
        }
        Ok(Redactor { extra })
    }

    /// [`Self::for_patterns`] plus the run's exact values, applied after the connector's rules
    /// (D60): the oracle of [`stream::StreamRedactor::for_run`].
    pub fn for_run(
        id: &str,
        patterns: &[ExtraPattern],
        secrets: &RunSecrets,
    ) -> Result<Redactor, DaemonError> {
        let mut redactor = Redactor::for_patterns(id, patterns)?;
        redactor.extra.extend(secrets.patterns());
        Ok(redactor)
    }

    /// Every kind this redactor can attribute a span to: the shared set, then the
    /// connector's.
    ///
    /// One entry per literal, not per kind, so a kind spelled as two literals is listed
    /// twice — `generic_secret_assignment` is, because a quoted JSON key and a shell/TOML
    /// `key = value` need different shapes to catch the same thing.
    pub fn kinds(&self) -> Vec<&str> {
        patterns()
            .iter()
            .chain(self.extra.iter())
            .map(|p| p.kind.as_str())
            .collect()
    }

    /// Redact one record.
    ///
    /// Every rule finds its matches on the original bytes, and [`select`] decides which
    /// ones are claimed: among the hard rules (the shared set, then the connector's, in that
    /// order) the earlier rule wins an overlap, which is how `bearer` and `openai_key` both
    /// decline the `sk-ant-…` that `anthropic_key` took; the one soft rule never wins and
    /// keeps only the parts of its matches that no hard rule claimed. `select` states the
    /// rule in full. A claimed span is the whole match, or the pattern's first capture group
    /// when it declares one — see [`Pattern::span_group`], which is what keeps a redacted
    /// JSON field a JSON field. Each claimed span is replaced by `<redacted:KIND>`.
    ///
    /// **A capture-group span whose value was not already a JSON string gets a quoted
    /// marker**, `"<redacted:KIND>"`. A pattern declares a group in order to keep
    /// the record parseable, and for `"credits_remaining": 17.5` or `"rate_limits": {…}`
    /// there are no quotes left behind to hold a bare marker — the field would stop being
    /// a JSON value. The span and offsets still name the value's own bytes;
    /// only the marker wears the quotes. Whole-match spans never do: those are the
    /// shell/TOML literal and the vendor prefixes, whose bare markers the committed corpus
    /// already carries.
    pub fn apply(&mut self, bytes: &[u8]) -> Redacted {
        self.apply_filtered(bytes, None)
    }

    /// [`Self::apply`] with every rule of kind `kind` left out: the policy as it was before
    /// that rule existed. A test oracle, so that "this rule only ever adds redaction" is
    /// checked against the real policy and not a copy of it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn apply_without_kind(&mut self, bytes: &[u8], kind: &str) -> Redacted {
        self.apply_filtered(bytes, Some(kind))
    }

    /// [`Self::apply`] with first-wins selection alone, the blocks not merged with what they
    /// touch: the policy before a block absorbed anything. A test oracle, so that "a block
    /// only adds redaction" is checked against the real selection and not a copy of it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn apply_first_wins_only(&mut self, bytes: &[u8]) -> Redacted {
        self.apply_selected(bytes, None, false)
    }

    fn apply_filtered(&mut self, bytes: &[u8], skip_kind: Option<&str>) -> Redacted {
        self.apply_selected(bytes, skip_kind, true)
    }

    fn apply_selected(
        &mut self,
        bytes: &[u8],
        skip_kind: Option<&str>,
        merge_blocks: bool,
    ) -> Redacted {
        let claimed = select_with(
            candidates(
                patterns()
                    .iter()
                    .chain(self.extra.iter())
                    .filter(|p| skip_kind.is_none_or(|k| p.kind != k)),
                bytes,
            ),
            merge_blocks,
        );

        let mut out = Vec::with_capacity(bytes.len());
        let mut spans = Vec::with_capacity(claimed.len());
        let mut cut = 0usize;
        for claim in claimed {
            out.extend_from_slice(&bytes[cut..claim.start]);
            out.extend_from_slice(marker(claim.kind, claim.quote).as_bytes());
            spans.push(RedactionAuditSpan {
                kind: claim.kind.to_string(),
                start: claim.start as u64,
                end: claim.end as u64,
            });
            cut = claim.end;
        }
        out.extend_from_slice(&bytes[cut..]);

        Redacted { bytes: out, spans }
    }
}

/// One interval a rule matched: a candidate before priority resolution, or a claimed span
/// after [`select`]. `priority` is the rule's position in the set it was found with (the
/// shared literals first). Among hard claims a lower number wins an overlap; a soft claim
/// never wins one whatever its number, see [`select`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Claim<'a> {
    pub(crate) priority: usize,
    /// The replaced span: the whole match, or the rule's capture group.
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// The whole match, which contains the span. A cut through it would change what the
    /// rule matches on either side.
    pub(crate) whole_start: usize,
    pub(crate) whole_end: usize,
    pub(crate) kind: &'a str,
    /// The marker wears its own quotes; see [`Redactor::apply`].
    pub(crate) quote: bool,
    /// A soft claim never shadows a hard one; see [`select`] and [`SOFT_KIND`].
    pub(crate) soft: bool,
    /// A block claim absorbs the hard claims it contains or touches; see [`select`] and
    /// [`BLOCK_KIND`].
    pub(crate) block: bool,
}

/// Every match of every rule in `set`, in the rule's span form ([`Pattern::span_group`]).
/// Rules are independent here; overlaps are resolved by [`select`].
///
/// The rules match on [`match_view`]`(bytes)`, so an invalid byte inside a key body does not
/// end the key. The offsets are offsets in `bytes` either way.
///
/// A `private_key` match is cut into the pieces [`string_pieces`] names, each its own block
/// claim. Every piece keeps the whole match as its `whole_start..whole_end`, so the stream
/// holds and releases the match as one.
pub(crate) fn candidates<'a>(
    set: impl Iterator<Item = &'a Pattern>,
    bytes: &[u8],
) -> Vec<Claim<'a>> {
    let view = match_view(bytes);
    let bytes: &[u8] = &view;
    let mut found_all = Vec::new();
    for (priority, p) in set.enumerate() {
        // `find_iter` on the common path: a pattern that declares no capture group has
        // nothing to ask `captures_iter` for, and that engine is the slower one.
        // `or_else(|| c.get(0))` covers an optional group that did not participate in
        // the match: the span then widens back to the whole match rather than
        // disappearing, because a rule that fired and removed nothing would leave the
        // secret in the chain.
        type Found = ((usize, usize), (usize, usize));
        let found: Box<dyn Iterator<Item = Found> + '_> = if p.span_group == 0 {
            Box::new(p.re.find_iter(bytes).map(|m| {
                let span = (m.start(), m.end());
                (span, span)
            }))
        } else {
            let g = p.span_group;
            Box::new(p.re.captures_iter(bytes).filter_map(move |c| {
                let whole = c.get(0)?;
                let span = c.get(g).unwrap_or(whole);
                Some(((span.start(), span.end()), (whole.start(), whole.end())))
            }))
        };
        for ((start, mut end), (whole_start, mut whole_end)) in found {
            if p.block {
                end = char_end(bytes, end);
                whole_end = whole_end.max(end);
            }
            if p.block && p.kind == BLOCK_KIND {
                for (start, end) in string_pieces(bytes, start, end) {
                    found_all.push(Claim {
                        priority,
                        start,
                        end,
                        whole_start,
                        whole_end,
                        kind: p.kind.as_str(),
                        quote: false,
                        soft: false,
                        block: true,
                    });
                }
                continue;
            }
            let quote = p.span_group != 0 && !span_is_a_json_string_interior(bytes, start, end);
            found_all.push(Claim {
                priority,
                start,
                end,
                whole_start,
                whole_end,
                kind: p.kind.as_str(),
                quote,
                soft: p.soft,
                block: p.block,
            });
        }
    }
    found_all
}

/// The parts of a PEM block's range `start..end` that lie inside JSON strings: the range is
/// cut at every unescaped `"` (one after an even run of backslashes), starting inside a string,
/// because the `BEGIN` that starts it is text. So a key streamed as many string deltas loses
/// every delta's text and keeps the JSON between them, and each record still parses. A piece
/// loses any backslashes it ends on, so an escaped quote at the bound (`\"`) is never split.
/// Text with no quote is one piece, the whole range. A piece left empty is dropped.
pub fn string_pieces(bytes: &[u8], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut pieces = Vec::new();
    let mut push = |from: usize, mut to: usize| {
        while to > from && bytes[to - 1] == b'\\' {
            to -= 1;
        }
        if to > from {
            pieces.push((from, to));
        }
    };
    let (mut inside, mut from, mut slashes) = (true, start, 0usize);
    for (at, &b) in bytes.iter().enumerate().take(end).skip(start) {
        if b == b'"' && slashes % 2 == 0 {
            if inside {
                push(from, at);
            } else {
                from = at + 1;
            }
            inside = !inside;
        }
        slashes = if b == b'\\' { slashes + 1 } else { 0 };
    }
    if inside {
        push(from, end);
    }
    pieces
}

/// The end of the UTF-8 character that `end` falls inside, or `end` on a boundary. The block
/// rule bounds its body in bytes, so a match may end inside a character; the stream's PEM
/// recognizer takes that character's unit whole, and so does the span.
fn char_end(bytes: &[u8], end: usize) -> usize {
    for p in end.saturating_sub(3)..end {
        let width = match bytes[p] {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => continue,
        };
        if p + width > end
            && bytes
                .get(p..p + width)
                .is_some_and(|c| std::str::from_utf8(c).is_ok())
        {
            return p + width;
        }
    }
    end
}

/// Resolve every rule's candidates into the claimed spans, in source order, never overlapping.
///
/// **The overlap rule.** Every rule finds its matches on the original bytes. The hard rules
/// are resolved first-wins, by list position and then by source order: a candidate that
/// overlaps a claim already made is dropped whole, which is how `bearer` and `openai_key`
/// both decline the `sk-ant-…` that `anthropic_key` took. That selection, [`select_hard`],
/// includes the one block rule (`private_key`, [`BLOCK_KIND`]) at its list position, and it is
/// never changed: whatever it claims stays claimed.
///
/// **A block then absorbs.** Every block is merged with every candidate that overlaps it,
/// selected or dropped, and with whatever overlaps those, until nothing more joins: the
/// claims inside a block vanish into it, and a claim across its edge extends it, so a block
/// is never shadowed and never leaves the outside part of a claim raw. The group becomes one
/// span from its earliest start to its latest end, named for the block. A block's match runs
/// from its `BEGIN` line to the first `END` line within [`PEM_BODY_MAX`] bytes, or those bytes
/// when no `END` follows (a key cut off by an output limit is still a key), and its claims are
/// the string interiors in that range ([`string_pieces`]). No rule is reordered, and
/// containment wins nowhere else. Because the merge starts from the first-wins result and
/// only joins what overlaps a block, what a block adds is redaction: the coverage is a
/// superset of first-wins alone (`apply_first_wins_only`, the test oracle).
///
/// Then the one soft rule (`masked_key`, [`SOFT_KIND`]) is cut against the hard claims, and
/// only its uncovered pieces are kept, each as its own span. So adding the soft rule can only
/// add redaction. That is [`select_blocks`] followed by the remainder pass below.
pub(crate) fn select(candidates: Vec<Claim<'_>>) -> Vec<Claim<'_>> {
    select_with(candidates, true)
}

/// [`select`], or with `merge_blocks` off the first-wins selection alone (the test oracle).
fn select_with(candidates: Vec<Claim<'_>>, merge_blocks: bool) -> Vec<Claim<'_>> {
    if !candidates.iter().any(|c| c.soft) {
        return select_blocks(candidates, merge_blocks);
    }
    let (mut soft, hard): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|c| c.soft);
    let mut claimed = select_blocks(hard, merge_blocks);
    // A soft candidate keeps every part of itself no hard claim covers. Soft candidates of
    // one rule do not overlap one another (a regex yields disjoint matches), so each is
    // cut against the hard claims alone.
    soft.sort_unstable_by_key(|c| (c.start, c.end));
    let mut remainders: Vec<Claim<'_>> = Vec::new();
    let mut from = 0;
    for candidate in soft {
        while from < claimed.len() && claimed[from].end <= candidate.start {
            from += 1;
        }
        let mut at = candidate.start;
        let mut next = from;
        while next < claimed.len() && claimed[next].start < candidate.end {
            if claimed[next].start > at {
                remainders.push(Claim {
                    start: at,
                    end: claimed[next].start,
                    quote: false,
                    ..candidate
                });
            }
            at = at.max(claimed[next].end);
            next += 1;
        }
        if at < candidate.end {
            remainders.push(Claim {
                start: at,
                end: candidate.end,
                quote: false,
                ..candidate
            });
        }
    }
    claimed.extend(remainders);
    claimed.sort_unstable_by_key(|c| (c.start, c.end));
    claimed
}

/// The hard half of [`select`] with the block rule applied: [`select_hard`], then
/// [`absorb_into_blocks`]. `merge_blocks` off is the first-wins selection alone.
fn select_blocks(candidates: Vec<Claim<'_>>, merge_blocks: bool) -> Vec<Claim<'_>> {
    if !merge_blocks || !candidates.iter().any(|c| c.block) {
        return select_hard(candidates);
    }
    let mut blocks: Vec<Claim<'_>> = candidates.iter().filter(|c| c.block).copied().collect();
    blocks.sort_unstable_by_key(|c| (c.start, c.end));
    // Every other candidate that overlaps a block, whether first-wins keeps it or not.
    let touching: Vec<Claim<'_>> = candidates
        .iter()
        .filter(|c| !c.block)
        .filter(|c| {
            let at = blocks.partition_point(|b| b.end <= c.start);
            blocks.get(at).is_some_and(|b| b.start < c.end)
        })
        .copied()
        .collect();
    let claimed = select_hard(candidates);
    absorb_into_blocks(blocks, touching, claimed)
}

/// Join every block with everything it overlaps, transitively: the `touching` candidates,
/// and the first-wins `claimed` ones those reach. Each connected group of two or more is
/// one claim from the earliest start to the latest end, with the block's kind and priority
/// and no quote (a block is a whole-match span). A claim connected to no block passes
/// through unchanged. `claimed` is disjoint, as [`select_hard`] returns it, and so are the
/// `blocks` (one rule's matches); two blocks joined by one claim across both become one.
fn absorb_into_blocks<'a>(
    blocks: Vec<Claim<'a>>,
    touching: Vec<Claim<'a>>,
    claimed: Vec<Claim<'a>>,
) -> Vec<Claim<'a>> {
    let mut all = Vec::with_capacity(blocks.len() + touching.len() + claimed.len());
    all.extend(blocks);
    all.extend(touching);
    all.extend(claimed.into_iter().filter(|c| !c.block));
    all.sort_unstable_by_key(|c| (c.start, c.end, !c.block));
    let mut out: Vec<Claim<'a>> = Vec::with_capacity(all.len());
    // The group being built, and how many claims are in it.
    let mut current: Option<(Claim<'a>, usize)> = None;
    for c in all {
        current = match current {
            Some((mut group, n)) if c.start < group.end => {
                if c.block && !group.block {
                    group.priority = c.priority;
                    group.kind = c.kind;
                    group.block = true;
                }
                group.end = group.end.max(c.end);
                group.whole_start = group.whole_start.min(c.whole_start);
                group.whole_end = group.whole_end.max(c.whole_end);
                group.quote = false;
                Some((group, n + 1))
            }
            Some((group, _)) => {
                out.push(group);
                Some((c, 1))
            }
            None => Some((c, 1)),
        };
    }
    out.extend(current.map(|(group, _)| group));
    out
}

/// The hard half of [`select`]: first-wins claiming among rules that are all hard.
///
/// The first sort key is the rule's fixed priority; within one rule regex yields source
/// order. One priority group at a time is selected against a source-sorted claimed set, then
/// the newly admitted spans are merged linearly. This avoids sorted-Vec insertion, whose
/// memmoves made a reverse-ordered high-match input quadratic in its number of matches.
fn select_hard(mut candidates: Vec<Claim<'_>>) -> Vec<Claim<'_>> {
    candidates.sort_unstable_by_key(|c| (c.priority, c.start, c.end));
    let mut claimed: Vec<Claim<'_>> = Vec::new();
    let mut group_start = 0;
    while group_start < candidates.len() {
        let priority = candidates[group_start].priority;
        let mut group_end = group_start + 1;
        while group_end < candidates.len() && candidates[group_end].priority == priority {
            group_end += 1;
        }

        let mut admitted: Vec<Claim<'_>> = Vec::new();
        let mut claimed_at = 0;
        for candidate in &candidates[group_start..group_end] {
            while claimed_at < claimed.len() && claimed[claimed_at].end <= candidate.start {
                claimed_at += 1;
            }
            let overlaps_claimed =
                claimed_at < claimed.len() && claimed[claimed_at].start < candidate.end;
            let overlaps_admitted = admitted
                .last()
                .is_some_and(|last| last.end > candidate.start);
            if !overlaps_claimed && !overlaps_admitted {
                admitted.push(*candidate);
            }
        }

        let mut merged = Vec::with_capacity(claimed.len() + admitted.len());
        let (mut left, mut right) = (0, 0);
        while left < claimed.len() || right < admitted.len() {
            if right == admitted.len()
                || (left < claimed.len() && claimed[left].start <= admitted[right].start)
            {
                merged.push(claimed[left]);
                left += 1;
            } else {
                merged.push(admitted[right]);
                right += 1;
            }
        }
        claimed = merged;
        group_start = group_end;
    }
    claimed
}

/// The replacement a claimed span leaves behind: `<redacted:KIND>`, or `"<redacted:KIND>"`
/// when the marker has to bring its own quotes.
pub(crate) fn marker(kind: &str, quote: bool) -> String {
    if quote {
        format!("\"<redacted:{kind}>\"")
    } else {
        format!("<redacted:{kind}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(
        priority: usize,
        start: usize,
        end: usize,
        kind: &'static str,
        block: bool,
    ) -> Claim<'static> {
        Claim {
            priority,
            start,
            end,
            whole_start: start,
            whole_end: end,
            kind,
            quote: false,
            soft: false,
            block,
        }
    }

    fn spans<'a>(claims: &[Claim<'a>]) -> Vec<(usize, usize, &'a str)> {
        claims.iter().map(|c| (c.start, c.end, c.kind)).collect()
    }

    /// Two blocks and the one claim across both edges become one span; a claim inside a block
    /// vanishes; a claim near no block is untouched; no block means plain first-wins.
    #[test]
    fn a_block_absorbs_what_it_contains_and_what_crosses_it() {
        let got = select(vec![
            claim(7, 0, 10, "private_key", true),
            claim(7, 20, 30, "private_key", true),
            claim(9, 8, 22, "email", false),
            claim(3, 24, 26, "github_token", false),
            claim(3, 40, 50, "github_token", false),
            claim(7, 60, 70, "private_key", true),
            claim(3, 62, 65, "github_token", false),
        ]);
        assert_eq!(
            spans(&got),
            vec![
                (0, 30, "private_key"),
                (40, 50, "github_token"),
                (60, 70, "private_key")
            ]
        );
        assert_eq!(got[0].priority, 7);
        assert!(!got[0].quote);

        // Touching is not overlapping: a claim that starts where the block ends stays.
        let got = select(vec![
            claim(7, 0, 10, "private_key", true),
            claim(3, 10, 20, "github_token", false),
        ]);
        assert_eq!(
            spans(&got),
            vec![(0, 10, "private_key"), (10, 20, "github_token")]
        );

        // A claim that lost to a higher one outside the block is still dropped whole.
        let got = select(vec![
            claim(3, 0, 10, "github_token", false),
            claim(9, 5, 15, "email", false),
        ]);
        assert_eq!(spans(&got), vec![(0, 10, "github_token")]);
    }
}
