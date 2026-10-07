//! Seal-time attribution (DECISIONS D11): every file in the sealed workspace
//! must be explained by a traced **edit, write, or shell command** — not by a call that
//! merely read it.
//!
//! **One attribution rule, in one module.** The rule below is the same one V9 applies to a
//! config file that changed underneath the capture, and it is written once:
//! `path_arg_attributes` and `shell_attributes` live here and `checks::v9_config` imports
//! them. Two implementations of "did this call write that file?" is how the two checks come
//! to disagree.
//!
//! **The baseline comes before the rule.** The daemon records a manifest of the
//! workspace as it was created. A sealed file whose path and sha256 both appear in the
//! baseline is `Baseline` and owes no traced write. Its inverse is
//! `BULK_ATTRIBUTION_THRESHOLD`, which `verdict::seal_findings` applies: one shell call is
//! allowed to explain a handful of files, not a tree.
//!
//! The rule:
//!
//!   * a **non-shell write-class** call whose path-typed argument (`file_path`, `path`,
//!     `filePath`, `notebook_path`, or an `apply_patch`-DSL target) **equals** the file's
//!     path; or
//!   * a **shell-class** call whose *command text* either redirects into the path, or
//!     passes the path as an argument to a write operator.
//!
//! **A tool in both classes gets both halves.** Every shipped connector puts its shell tool
//! in the write class too — `exec` (`connectors/codex-cli/manifest.toml`) and `Bash`
//! (`connectors/claude-code-cli/manifest.toml`) are each declared write and shell. It is
//! the right declaration: one tool's argument text carries a shell command, and for Codex
//! it carries the `apply_patch` DSL as a JS-source string instead. So a call that is both
//! tries the write attributions first and the shell ones when none matches. A tool in only
//! one class is judged by that class's rule alone.
//!
//! **What that reach costs.** Because `Bash` is in both classes, `path_arg_attributes` —
//! and through it `patch_target_paths` — walks its arguments as well, so a shell call whose
//! text merely *quotes* an `apply_patch` header attributes the path it names without having
//! written it. A `Write` whose `content` quotes a header does the same.
//! `patch_target_paths` is scoped to strings that also carry `*** Begin Patch`, which
//! raises the bar; that is mitigation, not closure, and `docs/CHEAT-CHECKLIST.md` carries
//! it as an accepted gap.
//!
//! **The redirect rule.** "Contains a write operator anywhere" is too loose: `cat
//! /ws/planted.txt 2>/dev/null` contains `>` and only reads the file. So:
//!
//!   * a plain `>`, `1>` or `>>` attributes only when **its own target token** — glued
//!     (`>out.txt`) or the next token (`> out.txt`) — equals the path. A redirect to some
//!     other target never attributes the path, however often the path is mentioned
//!     elsewhere in the command;
//!   * `2>` (and any other descriptor), `&>` and `>&` never attribute: a stderr or
//!     duplication redirect is not a write to the named file;
//!   * a word operator (`tee`, `truncate`, `dd`, `patch`, `install`, `sed -i`, and — for
//!     V9's question only — `rm`) attributes only when the path appears as an **argument
//!     token after** that command word **and before the next command separator** (`;`,
//!     `&&`, `||`, `|`, or a line break, **spaced or glued between two tokens**) — so
//!     `rm` inside "Performance", a path named before any operator, and both
//!     `rm tmp && cat /ws/planted.txt` and `rm tmp;cat /ws/planted.txt` are out. The
//!     redirect rule is bounded the same way, so a `>` cannot take the next line's first
//!     token as its target. `cp` and `mv` attribute only the segment's last token, their
//!     destination;
//!   * of the `key=value` argument forms only `dd`'s `of=` writes: `if=` reads, and an
//!     environment assignment (`FOO=/ws/planted.txt cmd`) writes nothing at all.
//!
//! Everything else — a read, a grep, a bare mention (`echo /path`), a basename-only
//! match, a path named inside an interpreter one-liner — does not attribute.
//!
//! **Exclusions are anchored at the workspace root.** `is_excluded` matches a
//! pattern against `rel` by equality or leading path prefix, and matches at depth only when
//! the pattern says `**/`. A nested `.git` is the agent's work and needs an explanation
//! like any other file.
//!
//! **One exclusion function.** The verifier, the daemon's seal walk and the CLI's
//! self-check all call `goalcup_protocol::seal::is_excluded(rel, patterns)`.
//!
//! **No absolute machine paths leave this module.** A `SealAttribution` carries the seal's
//! workspace-relative path, a `tool_call_id` and a tool name. The absolute spelling is
//! built only to compare against a tool call's arguments, and is never stored or reported.
//! `unexplained_seal_file` and `read_only_attribution` — both `Severity::Flag` (D35) —
//! reach the verdict's `review_queue` from exactly these fields.

use crate::parsers::ParsedLegs;
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::seal::{SealManifest, is_excluded};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Shell commands that write **every** path they are handed as an argument. `sed` is
/// handled separately, because only `sed -i` writes.
pub(crate) const SHELL_WRITE_OPERATORS: [&str; 5] = ["tee", "truncate", "dd", "patch", "install"];

/// Shell commands whose **last** operand is the destination and whose earlier operands are
/// reads. `cp /ws/planted.txt /tmp/backup` puts nothing in the workspace: it reads the file
/// the seal is asking about. Scanning every operand would attribute the source, which is
/// the "let the tracked agent discover the results" attack with one copy in front of it.
///
/// `install` is deliberately not here: it takes `-t DIR src…`, so its destination is not
/// positionally fixed, and over-attributing an `install` is the safe direction — it is a
/// write command in every form.
const SHELL_DESTINATION_OPERATORS: [&str; 2] = ["cp", "mv"];

/// Shell commands that remove the path they are handed.
///
/// A removal explains how a watched config file came to differ (V9's question, D6). It
/// never explains why a file is in the seal — the seal is the workspace as it stands at
/// the deadline, so `rm /ws/solution.py` and `solution.py` being sealed are two facts
/// about different files, and letting one silence `unexplained_seal_file` is a
/// one-command laundry for a planted file.
const SHELL_REMOVE_OPERATORS: [&str; 1] = ["rm"];

/// Which question a shell attribution is being asked. The rule is one definition
/// (`shell_attributes`); the operator lists differ because the two callers differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// V13's seal walk: which traced call put this file in the workspace. A removal is not
    /// an answer.
    Seal,
    /// V9's reconciliation: which traced call changed this watched file. A removal is an
    /// answer.
    ConfigChange,
}

/// How an operator's operands relate to the path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Operands {
    /// Every token after the command word is a target.
    Every,
    /// Only the segment's last token is a target; the rest are sources.
    LastOnly,
}

/// Argument keys tried, in order, for a non-shell tool's path-typed argument.
const PATH_ARG_KEYS: [&str; 4] = ["file_path", "path", "filePath", "notebook_path"];

/// Argument keys tried, in order, for a shell tool's command line. Every measured shell
/// tool puts it under one of these.
const COMMAND_KEYS: [&str; 4] = ["command", "cmd", "input", "script"];

/// A shell call's command line, or `None` when the arguments carry nothing that looks like
/// one — in which case nothing is attributed, because the alternative is matching the
/// whole serialised argument blob, which is how a `cat` or a `grep` gets counted as a
/// write.
pub(crate) fn shell_command_text(args: &Value) -> Option<&str> {
    COMMAND_KEYS
        .iter()
        .find_map(|k| args.get(*k).and_then(|v| v.as_str()))
}

/// Every target path an `apply_patch`-style unified-diff DSL header line names (`***
/// Add File: X`, `*** Update File: X`, `*** Delete File: X`), wherever a string value in
/// the arguments carries one. Walked structurally, not over a re-serialised blob: Codex's
/// `exec` wraps the DSL text inside a JS-source string inside the outer JSON, so a target
/// path can be followed by a real newline, a JS-escaped `\n` (a literal backslash then
/// `n`), or the JS string's closing quote — any of `\`, `\n` or `"` ends it.
///
/// Header lines are harvested only out of a string that also carries `*** Begin Patch`.
/// Every shipped connector's shell tool is in the write class, so this walks `Bash`
/// arguments too, and without that condition an `echo` of one header line would explain the
/// file it named. It is **mitigation, not closure**: a call that quotes a whole patch
/// envelope without applying it still attributes, and `docs/CHEAT-CHECKLIST.md` records
/// that as an accepted gap.
pub(crate) fn patch_target_paths(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) if s.contains("*** Begin Patch") => {
                for prefix in ["*** Add File: ", "*** Update File: ", "*** Delete File: "] {
                    let mut rest = s.as_str();
                    while let Some(idx) = rest.find(prefix) {
                        let after = &rest[idx + prefix.len()..];
                        let end = after.find(['\\', '\n', '"']).unwrap_or(after.len());
                        let target = after[..end].trim();
                        if !target.is_empty() {
                            out.push(target.to_string());
                        }
                        rest = &after[end..];
                    }
                }
            }
            Value::Object(m) => {
                for v in m.values() {
                    walk(v, out);
                }
            }
            Value::Array(a) => {
                for v in a {
                    walk(v, out);
                }
            }
            _ => {}
        }
    }
    walk(value, &mut out);
    out
}

/// A non-shell call's path-typed argument, or an `apply_patch`-DSL target, equals
/// `full_path` — never a basename-only or substring match.
pub(crate) fn path_arg_attributes(args: &Value, full_path: &str) -> bool {
    for key in PATH_ARG_KEYS {
        if args.get(key).and_then(|v| v.as_str()) == Some(full_path) {
            return true;
        }
    }
    patch_target_paths(args).iter().any(|p| p == full_path)
}

/// A shell **command** writes `full_path`: it redirects into it, or hands it to a write
/// operator as an argument.
///
/// `command` is the command text — `shell_command_text(args)` — and never the serialised
/// argument object: an operator that happens to appear in some other
/// argument is not a write, and the command line is the only place a shell tool's effect
/// is visible.
///
/// **Both halves of the rule are segment-scoped.** The command text is cut into lines, each
/// line into command segments, and each segment is scanned on its own. Doing the cut
/// *before* tokenising is the point: a separator needs no whitespace around it, so
/// `rm tmp;cat /ws/planted.txt` is two commands. The same cut bounds the redirect scan:
/// `echo a >` on one line and `/ws/planted.txt` on the next do not join across the break
/// into a write.
///
/// **A line ends at a JS-escaped newline too.** See `logical_lines`.
pub(crate) fn shell_attributes(command: &str, full_path: &str, purpose: Purpose) -> bool {
    logical_lines(command)
        .into_iter()
        .flat_map(command_segments)
        .any(|segment| {
            redirect_target_is(segment, full_path)
                || word_operator_takes(segment, full_path, purpose)
        })
}

/// One command text's lines: the text between line breaks, where a break is a real `\n`
/// or `\r` **or a JS-escaped one — a literal backslash followed by `n` or `r`**.
///
/// The escaped form is not a nicety. Codex 0.153.0 emits its shell calls in *code mode*:
/// the command lives inside a JS-source string inside the outer JSON, so `printf %s hello >
/// hello.txt` followed by `wc -c hello.txt` arrives as one `arguments.input` string whose
/// separator is a backslash and the letter `n`, never a control character. Splitting on
/// real newlines alone would leave the whole envelope as one segment, and the redirect's
/// target token would come out `hello.txt\nwc`. `patch_target_paths` ends a target path at
/// `\`, a real newline or the JS string's closing quote for this same envelope; this is the
/// same knowledge, applied to the other half of the rule.
///
/// **A literal `\n` inside a single-quoted shell string is not a line break to the shell**
/// — `printf 'a\nb' > out.txt` passes two characters to `printf` — and this function
/// breaks there anyway. That is deliberate, and it is the safe direction. Both halves of
/// the rule are *bounded* by the cut: more cuts can only shorten an operator's operand
/// list and shorten a bare `>`'s reach for its target, never lengthen either. Under-
/// segmenting is what admits a bypass (`rm tmp;cat /ws/planted.txt` read as one command
/// attributes a file the second command merely reads); over-segmenting can at worst lose
/// an attribution on a contrived command, and a lost attribution is a Flag an organizer
/// reads, not a Fail (D35). The redirect in `printf 'a\nb' > out.txt` still attributes,
/// because the cut leaves `b' > out.txt` and the redirect is wholly inside it.
///
/// Byte indexing is safe: every character matched here is ASCII, and no UTF-8
/// continuation byte can equal one.
fn logical_lines(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        let width = match bytes[i] {
            b'\n' | b'\r' => 1,
            b'\\' if matches!(bytes.get(i + 1), Some(b'n' | b'r')) => 2,
            _ => {
                i += 1;
                continue;
            }
        };
        out.push(&command[start..i]);
        i += width;
        start = i;
    }
    out.push(&command[start..]);
    out
}

/// One line's command segments: the text between shell command separators — `;`, `|`,
/// `||`, `&`, `&&`. A run of separators simply yields empty segments, which scan to
/// nothing, so `&&` needs no special case and neither does a leading or trailing one.
///
/// `&` separates only when it is not part of a redirect. `&>` (both streams) and `>&`
/// (descriptor duplication, `2>&1`) are redirect syntax, and cutting them in half would
/// hand the redirect scan a bare `>` — turning `cmd &>/dev/null` into a plain stdout
/// write to `/dev/null`, which is exactly the class of bypass this module exists to close.
fn command_segments(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in line.char_indices() {
        let separates = match c {
            ';' | '|' => true,
            '&' => bytes[..i].last() != Some(&b'>') && bytes.get(i + 1) != Some(&b'>'),
            _ => false,
        };
        if separates {
            out.push(&line[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&line[start..]);
    out
}

/// Strip one layer of matching shell quoting, so `"out.txt"` and `'out.txt'` compare equal
/// to `out.txt`. Backticks are deliberately not stripped: `` `long.py` `` in a markdown
/// table is prose, not a filename.
///
/// **And the token ends at an inner `"`.** This is `patch_target_paths`'s third terminator,
/// applied where the shell rule tokenises: in Codex's code-mode envelope the last command
/// of the JS string runs straight into the string's closing quote and whatever follows it,
/// so a redirect's target token arrives as `hello.txt",max_output_tokens:1000}));`.
/// Trimming the ends does not reach that quote — the token does not end there — so the scan
/// is cut at the first `"` that survives the trim. A file name containing a double quote is
/// truncated by this and goes unattributed; that is a Flag, not a Fail, and the alternative
/// is a real write nothing explains.
fn unquote(token: &str) -> &str {
    let token = token.trim_matches(['"', '\'']);
    match token.find('"') {
        Some(end) => &token[..end],
        None => token,
    }
}

/// The path is the target of a plain stdout redirect **in one command segment**.
///
/// `2>path`, `&>path` and `>&path` are excluded: a stderr redirect, a both-streams
/// redirect and a descriptor duplication are not writes to a file the seal is asking
/// about — and `cat path 2>/dev/null` is exactly the bypass this rule closes.
///
/// The segment bound is what stops a bare `>` from reaching across a line break or a
/// command separator for its target: `echo a >\n/ws/planted.txt` redirects into nothing
/// this scan can name, and it must not read as a write.
fn redirect_target_is(segment: &str, full_path: &str) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        let Some(pos) = token.find('>') else {
            continue;
        };
        let before = &token[..pos];
        // `&>` redirects both streams; it is not a plain write to the named file.
        if before.ends_with('&') {
            continue;
        }
        // An explicit descriptor: `1>` is stdout and attributes; `2>` and anything else
        // is another stream and never does.
        let descriptor: String = before
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect();
        if !descriptor.is_empty() && descriptor != "1" {
            continue;
        }
        let arrows = token[pos..].chars().take_while(|c| *c == '>').count();
        if arrows > 2 {
            continue;
        }
        let rest = &token[pos + arrows..];
        // `>&2` duplicates a descriptor; it names no file.
        if rest.starts_with('&') {
            continue;
        }
        let target = if rest.is_empty() {
            tokens.get(i + 1).copied().unwrap_or_default()
        } else {
            rest
        };
        if unquote(target) == full_path {
            return true;
        }
    }
    false
}

/// A write operator takes the path as one of its argument tokens, **within one command
/// segment** — where "one of its argument tokens" depends on the operator and on the
/// question: `tee` writes every operand, `cp` and `mv` write only the last, and `rm` writes
/// none of them for a seal.
///
/// The segment bound is what keeps `rm tmp && cat /ws/planted.txt` and `rm tmp;cat
/// /ws/planted.txt` from attributing the file the second command only reads. A separator
/// glued to the front of a token (`;cat`) starts the next segment, and a trailing `;` (`rm
/// foo;`) ends this one *after* its own argument.
fn word_operator_takes(segment: &str, full_path: &str, purpose: Purpose) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    // A bare argument, or `dd`'s output operand. `if=` is dd's input and `FOO=path` is an
    // environment assignment: neither writes.
    let takes = |argument: &str| {
        unquote(argument) == full_path
            || argument
                .split_once('=')
                .is_some_and(|(key, v)| key == "of" && unquote(v) == full_path)
    };
    for (i, token) in tokens.iter().enumerate() {
        let word = unquote(token);
        // `/usr/bin/rm` is `rm`; `perform` is not.
        let command_word = word.rsplit('/').next().unwrap_or(word);
        // Only `sed -i` writes; `sed -n` reads.
        let is_sed_in_place = command_word == "sed"
            && tokens
                .iter()
                .skip(i + 1)
                .take_while(|a| a.starts_with('-'))
                .any(|a| a.starts_with("-i"));
        let operands = if SHELL_WRITE_OPERATORS.contains(&command_word) || is_sed_in_place {
            Operands::Every
        } else if SHELL_DESTINATION_OPERATORS.contains(&command_word) {
            Operands::LastOnly
        } else if SHELL_REMOVE_OPERATORS.contains(&command_word) && purpose == Purpose::ConfigChange
        {
            Operands::Every
        } else {
            continue;
        };
        match operands {
            Operands::Every => {
                if tokens.iter().skip(i + 1).any(|a| takes(a)) {
                    return true;
                }
            }
            // The last token of the segment, and only when it is an operand of this
            // command word rather than the command word itself (`cp` alone writes nothing).
            Operands::LastOnly => {
                if tokens.len() > i + 1 && tokens.last().is_some_and(|a| takes(a)) {
                    return true;
                }
            }
        }
    }
    false
}

/// One traced shell call explaining more than this many sealed files is a Flag, not a
/// pass: `cp -r ~/prepared/. .` is one call and an arbitrary tree.
///
/// The number is a limit, not a target — a call explaining exactly this many raises
/// nothing — and it counts **distinct sealed files**, not path mentions: a command that
/// names fifty paths of which eight are in the seal explains eight.
pub const BULK_ATTRIBUTION_THRESHOLD: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attribution {
    /// The file was in the baseline with this digest: the workspace was not empty of
    /// it, and no traced write is owed. A file present in the baseline under a
    /// **different** digest is not `Baseline`; it was rewritten and needs an explanation
    /// like any other file.
    Baseline { sha256: String },
    /// A write-class call named this file: an edit, a write, or a shell command that
    /// redirects into it or hands it to a write operator.
    Traced { call_id: String, tool: String },
    /// The file is named **only** by calls that read it. Finding `read_only_attribution`,
    /// `Severity::Flag`: "this appeared in the workspace and the agent only ever looked at
    /// it" is a question for a reviewer, not a pass.
    ReadOnly { call_id: String },
    /// Nothing in the trace names this file in a way that explains it. Finding
    /// `unexplained_seal_file`, `Severity::Flag` (D35): no verifier parses every command,
    /// so absence of attribution is an inference an organizer adjudicates, not a rule
    /// objectively broken.
    ///
    /// This is where a path mentioned inside an interpreter one-liner lands
    /// (`python3 - <<'PY' … open("long.py","w") … PY`): the verifier does not parse the
    /// interpreted language, so such a mention is evidence of neither a write nor a read,
    /// and the fail-closed reading of "no evidence" is `Unexplained`, not `ReadOnly`.
    Unexplained,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealAttribution {
    /// Workspace-relative, exactly as the seal recorded it. Never the absolute spelling.
    pub path: String,
    pub attribution: Attribution,
}

/// Attribute every sealed file to the baseline, a traced write, a traced read, or
/// nothing.
///
/// **The baseline is consulted first.** It is the manifest the daemon took when the
/// workspace was created. A sealed file whose path **and** sha256 both appear in the
/// baseline is `Baseline` and owes nothing. A path in the baseline under different bytes
/// owes an explanation like any other file: the agent rewrote it, and the rewrite is the
/// work. A path the baseline names twice exempts nothing at all: two digests for one file
/// is a malformed document, and a participant must not be able to buy an exemption by
/// offering both answers.
///
/// **The caller decides whether the baseline is believed at all.** `verify_trace` passes
/// `Some` only where the chain binds the baseline at seq 0 — see
/// `checks::v13_binding::baseline_binding`. A manifest no link covers is a list the
/// participant's own daemon wrote, and this function never sees one.
///
/// A sealed path is workspace-relative while a tool call's arguments may be either, so
/// each file is compared under both spellings: `<workspace>/<path>` and `<path>`. The
/// absolute spelling is used for comparison only and never reported.
///
/// Files excluded by `manifest.workspace_exclusions` — through
/// `goalcup_protocol::seal::is_excluded`, the single definition — are dropped: they
/// are not the agent's work and need no explanation.
pub fn attribute_seal(
    seal: &SealManifest,
    baseline: Option<&SealManifest>,
    parsed: &ParsedLegs,
    manifest: &ConnectorManifest,
) -> Vec<SealAttribution> {
    struct Call<'a> {
        id: &'a str,
        tool: &'a str,
        args: &'a Value,
        /// The command text, for a shell tool.
        command: Option<&'a str>,
        is_write: bool,
        is_shell: bool,
    }
    let calls: Vec<Call> = parsed
        .trajectory
        .steps
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .map(|c| {
            let is_shell = manifest.tool_classes.shell.contains(&c.function_name);
            Call {
                id: &c.tool_call_id,
                tool: &c.function_name,
                args: &c.arguments,
                command: if is_shell {
                    shell_command_text(&c.arguments)
                } else {
                    None
                },
                is_write: manifest.tool_classes.write.contains(&c.function_name),
                is_shell,
            }
        })
        .collect();

    // `(path, sha256)` from the baseline, sorted here rather than trusted to arrive
    // sorted: `SealManifest::sorted` is the loader's contract, not this function's, and a
    // binary search over an unsorted vector fails silently rather than loudly.
    let mut sorted: Vec<(&str, &str)> = baseline
        .map(|b| {
            b.files
                .iter()
                .map(|e| (e.path.as_str(), e.sha256.as_str()))
                .collect()
        })
        .unwrap_or_default();
    sorted.sort_by_key(|(path, _)| *path);
    // A path the baseline names twice is a malformed baseline: two digests for one file,
    // and the binary search below would return whichever of them the sort happened to
    // leave in the middle. Both go, so the file walks the ordinary ladder — a participant
    // must not be able to buy an exemption with a tie-break. Every path the baseline names
    // once is untouched: one bad entry does not void the whole document.
    let mut baseline_index: Vec<(&str, &str)> = Vec::with_capacity(sorted.len());
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i + 1;
        while j < sorted.len() && sorted[j].0 == sorted[i].0 {
            j += 1;
        }
        if j == i + 1 {
            baseline_index.push(sorted[i]);
        }
        i = j;
    }

    let workspace = seal.workspace.trim_end_matches('/');
    let mut out = Vec::new();
    for f in &seal.files {
        if is_excluded(&f.path, &manifest.workspace_exclusions) {
            continue;
        }
        // The baseline first: a file the workspace already held, byte for byte.
        let from_baseline = baseline_index
            .binary_search_by(|(path, _)| (*path).cmp(f.path.as_str()))
            .ok()
            .map(|i| baseline_index[i].1)
            .filter(|sha256| *sha256 == f.sha256);
        if let Some(sha256) = from_baseline {
            out.push(SealAttribution {
                path: f.path.clone(),
                attribution: Attribution::Baseline {
                    sha256: sha256.to_string(),
                },
            });
            continue;
        }
        let spellings = [format!("{workspace}/{}", f.path), f.path.clone()];
        // A write-class call that actually wrote it. A shell-class call whose arguments
        // carry no command text attributes nothing by the shell rule: fail closed rather
        // than fall back to matching the serialised argument blob.
        //
        // `c.is_write` already excludes a shell-only tool, so `c.is_shell` here means a
        // tool in both classes. That is every shipped connector's shell tool — Codex's
        // `exec` and Claude Code's `Bash` — and each gets both rules. The module header
        // says what that reach costs.
        let traced = calls.iter().find(|c| {
            c.is_write
                && spellings.iter().any(|p| {
                    path_arg_attributes(c.args, p)
                        || (c.is_shell
                            && c.command
                                .is_some_and(|cmd| shell_attributes(cmd, p, Purpose::Seal)))
                })
        });
        let attribution = match traced {
            Some(c) => Attribution::Traced {
                call_id: c.id.to_string(),
                tool: c.tool.to_string(),
            },
            // Named by a call that could not have written it: a read, a grep, a glob
            // naming this exact path. Equality, so a basename never lands here either.
            None => match calls
                .iter()
                .find(|c| !c.is_write && spellings.iter().any(|p| path_arg_attributes(c.args, p)))
            {
                Some(c) => Attribution::ReadOnly {
                    call_id: c.id.to_string(),
                },
                None => Attribution::Unexplained,
            },
        };
        out.push(SealAttribution {
            path: f.path.clone(),
            attribution,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// V9's question — "which traced call changed this watched file" — asked of the
    /// command text. This is the wider of the two operator sets (it keeps `rm`), so every
    /// assertion below about segments, redirects and operand forms is stated against it;
    /// `wrote` covers what the seal's narrower set additionally refuses.
    fn changed(command: &str, full_path: &str) -> bool {
        shell_attributes(command, full_path, Purpose::ConfigChange)
    }

    /// The seal's question: "which traced call put this file in the workspace".
    fn wrote(command: &str, full_path: &str) -> bool {
        shell_attributes(command, full_path, Purpose::Seal)
    }

    /// A file the agent only removed, or only copied from, is not a file the agent put
    /// there — so neither command may silence `unexplained_seal_file`. The attack this
    /// closes: plant a file before the run, have the tracked agent `cp` it somewhere or
    /// `rm` it once, and the planted file reads as `Traced`.
    #[test]
    fn a_removal_or_a_copy_source_never_explains_a_sealed_file() {
        assert!(!wrote("rm -f /ws/planted.txt", "/ws/planted.txt"));
        assert!(!wrote("/usr/bin/rm /ws/planted.txt", "/ws/planted.txt"));
        assert!(!wrote("cp /ws/planted.txt /tmp/backup", "/ws/planted.txt"));
        assert!(!wrote("cp -r /ws/planted.txt /tmp/", "/ws/planted.txt"));
        assert!(!wrote("mv /ws/planted.txt /tmp/backup", "/ws/planted.txt"));
        // The same removal still answers V9's question, which is about a change.
        assert!(changed("rm -f /ws/planted.txt", "/ws/planted.txt"));
        // …and a copy's destination is a write under either question.
        assert!(wrote("cp /tmp/src /ws/planted.txt", "/ws/planted.txt"));
        assert!(wrote("mv /tmp/src /ws/planted.txt", "/ws/planted.txt"));
        assert!(wrote("cp -r /tmp/src/. /ws/planted.txt", "/ws/planted.txt"));
        // A bare command word is not its own operand.
        assert!(!wrote("cp", "cp"));
        // Everything the seal's set keeps still attributes.
        assert!(wrote("cat x > /ws/hello.py", "/ws/hello.py"));
        assert!(wrote("cat x | tee /ws/hello.py", "/ws/hello.py"));
        assert!(wrote("sed -i s/a/b/ /ws/hello.py", "/ws/hello.py"));
        assert!(wrote("dd if=x of=/ws/hello.py", "/ws/hello.py"));
        assert!(wrote("install -m 755 /tmp/x /ws/hello.py", "/ws/hello.py"));
    }

    /// The bypass: a stderr redirect is not a write, and the file the
    /// command actually reads must not be attributed to it.
    #[test]
    fn a_stderr_redirect_never_attributes_the_file_being_read() {
        assert!(!changed(
            "cat /ws/planted.txt 2>/dev/null",
            "/ws/planted.txt"
        ));
        assert!(!changed("cat /ws/p.txt 2> /ws/p.txt.log", "/ws/p.txt"));
        assert!(!changed("run &> /ws/p.txt.log", "/ws/p.txt"));
        assert!(!changed("echo hi >&2 # /ws/p.txt", "/ws/p.txt"));
    }

    /// A redirect attributes its own target and nothing else.
    #[test]
    fn a_redirect_attributes_only_its_target() {
        assert!(changed("cat x > /ws/hello.py", "/ws/hello.py"));
        assert!(changed("cat x >/ws/hello.py", "/ws/hello.py"));
        assert!(changed("cat x >> 'out.txt'", "out.txt"));
        // The path is named, loudly, but the redirect goes somewhere else.
        assert!(!changed(
            "echo /ws/hello.py > /ws/other.txt",
            "/ws/hello.py"
        ));
        assert!(!changed("echo /ws/hello.py", "/ws/hello.py"));
    }

    /// A word operator attributes only what it is handed, after the command word.
    #[test]
    fn a_word_operator_attributes_its_argument_only() {
        assert!(changed("cat x | tee /ws/hello.py", "/ws/hello.py"));
        assert!(changed("/usr/bin/rm -f /ws/hello.py", "/ws/hello.py"));
        assert!(changed("sed -i s/a/b/ /ws/hello.py", "/ws/hello.py"));
        assert!(changed("dd if=x of=/ws/hello.py", "/ws/hello.py"));
        // `sed -n` reads; the path before the operator is not its argument.
        assert!(!changed("sed -n 1p /ws/hello.py", "/ws/hello.py"));
        assert!(!changed("cat /ws/hello.py | tee other", "/ws/hello.py"));
    }

    /// The M0 capture's own trap: `rm` inside "Performance", in a command that also names
    /// `long.py`.
    #[test]
    fn an_operator_inside_a_word_is_not_an_operator() {
        let cmd = "python3 - <<'PYGEN'\nsections = [(\"Performance\", \"x\")]\nwith \
                   open(\"long.py\", \"w\") as f:\n    f.write(t)\nPYGEN";
        assert!(!changed(cmd, "long.py"));
        assert!(!changed("perform added dd_work", "dd"));
    }

    /// The argument scan stops at the next command separator, so a second command's read is
    /// not attributed to the first command's write operator.
    #[test]
    fn an_operators_arguments_end_at_the_next_command_separator() {
        assert!(!changed("rm tmp && cat /ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("rm tmp ; cat /ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("rm tmp | cat /ws/planted.txt", "/ws/planted.txt"));
        // The same two commands the other way round: `tee` really is handed the path.
        assert!(changed("tee /ws/hello.py < x", "/ws/hello.py"));
        assert!(changed("cat x | tee /ws/hello.py", "/ws/hello.py"));
        // …and reading it into a pipe whose sink is elsewhere is not a write.
        assert!(!changed("cat /ws/hello.py | tee other", "/ws/hello.py"));
    }

    /// A line break separates commands as firmly as `&&`, and a trailing `;` ends the
    /// command *after* its own argument.
    #[test]
    fn a_newline_separates_commands_and_a_trailing_semicolon_does_not_hide_its_argument() {
        assert!(!changed("rm tmp\ncat /ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("rm tmp\r\ncat /ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("rm tmp &&cat /ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("rm tmp ;cat /ws/planted.txt", "/ws/planted.txt"));
        // `rm foo;` really does delete `foo`.
        assert!(changed("rm /ws/planted.txt;", "/ws/planted.txt"));
        assert!(changed("rm /ws/planted.txt; ls", "/ws/planted.txt"));
        // …and the operator on the second line still takes its own argument.
        assert!(changed("ls\ntee /ws/planted.txt", "/ws/planted.txt"));
        assert!(changed("&&cp a /ws/planted.txt", "/ws/planted.txt"));
    }

    /// A separator needs no whitespace around it. Every form below is two commands, and the
    /// second one only reads the sealed file — which is the "let the tracked agent discover
    /// the results" attack, typed without spaces.
    #[test]
    fn a_separator_glued_between_two_tokens_still_ends_the_command() {
        for command in [
            "rm tmp;cat /ws/planted.txt",
            "rm tmp|cat /ws/planted.txt",
            "rm tmp&&cat /ws/planted.txt",
            "rm tmp||cat /ws/planted.txt",
            "cp a b;cat /ws/planted.txt",
        ] {
            assert!(
                !changed(command, "/ws/planted.txt"),
                "`{command}` only reads the file"
            );
        }
        // The same shape where the second command really is the write.
        assert!(changed("rm tmp; rm /ws/planted.txt", "/ws/planted.txt"));
        assert!(changed("rm tmp;rm /ws/planted.txt", "/ws/planted.txt"));
        // `&>` and `>&` are redirect syntax, not separators: cutting them in half would
        // hand the redirect scan a bare `>` and turn a both-streams redirect into a write.
        assert!(!changed("run &>/ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed(
            "cat /ws/planted.txt 2>&1 >/dev/null",
            "/ws/planted.txt"
        ));
    }

    /// The redirect scan is line- and segment-scoped, exactly like the word-operator scan,
    /// so a `>` at the end of one line does not take the first token of the next line as
    /// its target.
    #[test]
    fn a_redirect_never_reaches_across_a_line_break_for_its_target() {
        assert!(!changed("echo a >\n/ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("echo a >\r\n/ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed(
            "echo a > ; cat /ws/planted.txt",
            "/ws/planted.txt"
        ));
        // A genuine redirect on one line still attributes…
        assert!(changed("cat x > /ws/planted.txt", "/ws/planted.txt"));
        // …and so does the M0 capture's own heredoc, whose redirect target sits between an
        // `&&` and a `<<'EOF'`.
        assert!(changed(
            "ls -la && cat > README.md <<'EOF'\n# notes\nEOF",
            "README.md"
        ));
        assert!(changed(
            "cat > /ws/hello.py <<'EOF'\nprint(1)\nEOF",
            "/ws/hello.py"
        ));
    }

    /// `1>` is stdout; `if=` reads where `of=` writes; an environment assignment writes
    /// nothing.
    #[test]
    fn an_explicit_stdout_redirect_attributes_and_input_operands_do_not() {
        assert!(changed("cat x 1> /ws/hello.py", "/ws/hello.py"));
        assert!(changed("cat x 1>/ws/hello.py", "/ws/hello.py"));
        assert!(!changed("cat x 3> /ws/hello.py", "/ws/hello.py"));
        assert!(changed("dd if=x of=/ws/hello.py", "/ws/hello.py"));
        assert!(!changed("dd if=/ws/hello.py of=x", "/ws/hello.py"));
        assert!(!changed("FOO=/ws/hello.py rm tmp", "/ws/hello.py"));
        assert!(!changed("FOO=/ws/hello.py tee out", "/ws/hello.py"));
    }

    /// A basename is not a path, in either direction.
    #[test]
    fn a_basename_never_attributes() {
        assert!(!changed("cat x > docs/README.md", "README.md"));
        assert!(!changed("cp a OLD-README.md.bak", "README.md"));
    }

    #[test]
    fn a_path_typed_argument_must_equal_the_path() {
        let args = json!({"file_path": "/ws/hello.py"});
        assert!(path_arg_attributes(&args, "/ws/hello.py"));
        assert!(!path_arg_attributes(&args, "hello.py"));
        assert!(!path_arg_attributes(
            &json!({"file_path": "/ws/hello.py.bak"}),
            "/ws/hello.py"
        ));
        assert!(path_arg_attributes(
            &json!({"input": "*** Begin Patch\n*** Update File: /ws/hello.py\n@@"}),
            "/ws/hello.py"
        ));
        // The envelope is required: a bare header line, which is all an `echo` produces,
        // does not attribute.
        assert!(!path_arg_attributes(
            &json!({"input": "*** Update File: /ws/hello.py\n@@"}),
            "/ws/hello.py"
        ));
    }

    #[test]
    fn a_shell_command_is_read_from_the_command_field_only() {
        assert_eq!(
            shell_command_text(&json!({"command": "ls"})),
            Some("ls"),
            "the command field is the only place a shell effect is visible"
        );
        assert_eq!(shell_command_text(&json!({"description": "rm > x"})), None);
    }

    /// The exact `arguments.input` string recorded on a clean Codex journey, which sealed
    /// one honestly-written `hello.txt`. Codex 0.153.0's code mode nests the command in a
    /// JS-source string, so the two commands are separated by a literal backslash and an
    /// `n`, not by a control character.
    #[test]
    fn the_code_mode_envelope_attributes_the_file_its_redirect_names() {
        let recorded = "text(await tools.exec_command({cmd:\"printf %s hello > \
                        hello.txt\\nwc -c hello.txt\",max_output_tokens:1000}));\n";
        assert!(
            recorded.contains("hello.txt\\nwc"),
            "the separator under test is a backslash and an `n`, not a newline"
        );
        assert!(wrote(recorded, "hello.txt"));
        assert!(changed(recorded, "hello.txt"));
        // The absolute spelling `attribute_seal` also tries.
        let absolute = "text(await tools.exec_command({cmd:\"printf %s hello > \
                        /ws/hello.txt\\nwc -c /ws/hello.txt\",max_output_tokens:1000}));\n";
        assert!(wrote(absolute, "/ws/hello.txt"));
    }

    /// The other terminator `patch_target_paths` knows about: the last command
    /// in the JS string runs into the string's closing quote, so the target token carries
    /// the rest of the JS expression with it.
    #[test]
    fn a_code_mode_target_ends_at_the_js_strings_closing_quote() {
        assert!(wrote(
            "text(await tools.exec_command({cmd:\"printf %s hello > \
             hello.txt\",max_output_tokens:1000}));",
            "hello.txt"
        ));
        assert!(wrote(
            "text(await tools.exec_command({cmd:\"cat x | tee \
             hello.txt\",max_output_tokens:1000}));",
            "hello.txt"
        ));
        // Ordinary shell quoting is unchanged.
        assert!(wrote("cat x > \"out.txt\"", "out.txt"));
        assert!(wrote("cat x > 'out.txt'", "out.txt"));
    }

    /// The envelope explains only what it writes. A code-mode call that reads the file
    /// still attributes nothing, so the file lands on `ReadOnly` or `Unexplained` where it
    /// belongs.
    #[test]
    fn a_code_mode_read_still_attributes_nothing() {
        let read_only = "text(await tools.exec_command({cmd:\"cat \
                         hello.txt\\nwc -c hello.txt\",max_output_tokens:1000}));\n";
        assert!(!wrote(read_only, "hello.txt"));
        assert!(!changed(read_only, "hello.txt"));
        // A redirect to somewhere else does not attribute the file it reads, code mode or
        // not.
        assert!(!wrote(
            "text(await tools.exec_command({cmd:\"cat /ws/planted.txt > \
             /ws/other.txt\",max_output_tokens:1000}));",
            "/ws/planted.txt"
        ));
    }

    /// Both halves of the rule are bounded by the escaped break exactly as they are by a
    /// real one — and that is the direction that closes bypasses, not one that opens them.
    #[test]
    fn a_js_escaped_newline_bounds_an_operator_and_a_redirect_like_a_real_one() {
        assert!(!changed("rm tmp\\ncat /ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("tee out\\ncat /ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("echo a >\\n/ws/planted.txt", "/ws/planted.txt"));
        assert!(!changed("echo a >\\r\\n/ws/planted.txt", "/ws/planted.txt"));
        // …and each command still explains its own file.
        assert!(changed("rm /ws/a.txt\\ncat /ws/planted.txt", "/ws/a.txt"));
        assert!(wrote("cat x > /ws/a.txt\\nwc -c /ws/a.txt", "/ws/a.txt"));
        // A literal `\n` inside a single-quoted shell string is not a break to the shell;
        // breaking there anyway only shortens the scan, and the redirect still attributes.
        assert!(wrote("printf 'a\\nb' > out.txt", "out.txt"));
    }
}
