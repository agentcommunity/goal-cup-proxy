//! The verdict document: deterministic results, evidence references, and a
//! review queue of flags. Identical on the server and offline.
//!
//! Nothing here is derived from the machine the verification ran on. There is no
//! timestamp of the run, no absolute path, and no `HashMap` — two runs over the same
//! directory, on two machines, serialise to the same bytes or the document is wrong.

use crate::attribution::{Attribution, BULK_ATTRIBUTION_THRESHOLD, SealAttribution};
use crate::checks::{CheckResult, Evidence, Finding, Severity};
use crate::gaps::Gap;
use crate::human_rule::HumanMessage;
use crate::parsers::ParsedLegs;
use goalcup_protocol::manifest::ConnectorManifest;
use serde::{Deserialize, Serialize};

/// The seal findings, which belong to no check: `verify_trace` raises them into the review
/// queue itself.
///
/// A sealed file nothing explains is a `Flag` — the workspace holds work the trace cannot
/// account for, and an organizer decides what that means. A file only ever read is a
/// `Flag` too — "it appeared and the agent only looked at it" is neither innocent nor
/// proven. A `Traced` file says nothing: that is the expected case, and a queue of
/// expected cases is a queue nobody reads.
///
/// **Why `unexplained_seal_file` is a Flag and not a Fail (D35).** A Fail is for a rule
/// objectively broken or evidence objectively broken — V2's human message, V13's unbound
/// start. Absence of attribution is neither: it is an inference from incomplete
/// information, because no verifier parses every command. `npm install`, `make`, `cargo
/// build` and `python gen.py` all produce files no command text names, so a Fail here would
/// disqualify every real entry that used a build tool. The finding stays loud and stays in
/// the queue; it is not a verdict.
///
/// A `Baseline` file says nothing either, and for a stronger reason: the workspace held
/// it before the run began, so there is no work to account for.
///
/// **`bulk_attribution`.** Reading the baseline closes the "pre-populated workspace" hole
/// and opens its inverse: with the baseline in hand, one traced `cp -r ~/prepared/. .`
/// still explains an arbitrary tree, and every file in it passes. So the sealed files are
/// counted per attributing `call_id`, and a **shell** call explaining more than
/// `BULK_ATTRIBUTION_THRESHOLD` of them is a `Flag` naming the call and the count.
/// Non-shell write calls are exempt: an `Edit` explains one file by construction, and a
/// hundred of them are a hundred writes.
///
/// The order is `attribute_seal`'s, which is the seal's own path order (sorted), so the
/// queue is deterministic without being re-sorted here; the bulk findings follow, in
/// first-attribution order, which is that same path order.
pub fn seal_findings(
    attribution: &[SealAttribution],
    parsed: &ParsedLegs,
    manifest: &ConnectorManifest,
) -> Vec<Finding> {
    let mut out = Vec::new();
    // `(call_id, tool, distinct sealed files)`, in first-attribution order. A `Vec` and
    // not a map: this module's ordering is the document's ordering (see the header).
    let mut per_call: Vec<(&str, &str, usize)> = Vec::new();
    for a in attribution {
        let evidence = |note: &str| vec![Evidence::note("seal", note)];
        match &a.attribution {
            Attribution::Baseline { .. } => {}
            Attribution::Traced { call_id, tool } => {
                match per_call.iter_mut().find(|(id, _, _)| *id == call_id) {
                    Some((_, _, n)) => *n += 1,
                    None => per_call.push((call_id, tool, 1)),
                }
            }
            Attribution::ReadOnly { call_id } => out.push(crate::checks::flag(
                "read_only_attribution",
                format!(
                    "the sealed file `{}` is named only by calls that READ it; nothing in \
                     the trace explains how it came to be in the workspace",
                    a.path
                ),
                evidence(&format!("read by tool call {call_id}")),
            )),
            Attribution::Unexplained => out.push(crate::checks::flag(
                "unexplained_seal_file",
                format!(
                    "the sealed file `{}` is explained by no traced edit, write or shell \
                     command; the workspace holds work the trace does not account for",
                    a.path
                ),
                evidence("no traced write"),
            )),
        }
    }
    for (call_id, tool, count) in per_call {
        // The class comes from the trace's own call where the trajectory still carries it,
        // and from the attribution's recorded tool name otherwise — the same string by
        // construction, but a call the trace does not contain cannot be classified from
        // the attribution alone.
        let name = parsed
            .trajectory
            .steps
            .iter()
            .flat_map(|s| s.tool_calls.iter())
            .find(|c| c.tool_call_id == call_id)
            .map(|c| c.function_name.as_str())
            .unwrap_or(tool);
        if !manifest.tool_classes.shell.iter().any(|t| t == name) {
            continue;
        }
        if count <= BULK_ATTRIBUTION_THRESHOLD {
            continue;
        }
        out.push(crate::checks::flag(
            "bulk_attribution",
            format!(
                "the single shell call `{call_id}` (`{name}`) explains {count} sealed \
                 files, more than the limit of {BULK_ATTRIBUTION_THRESHOLD}; one command \
                 can import a whole prepared tree, and a tree explained by one command is \
                 not a tree the trace shows being written"
            ),
            vec![Evidence::note(
                "seal",
                format!("{count} sealed files attributed to tool call {call_id}"),
            )],
        ));
    }
    out
}

/// The Fail an entry earns when its descriptor names no session at all.
///
/// An entry created, armed and sealed without a harness ever starting still gets a document:
/// refusing an empty `tracked_sessions` would be exit 3 and nothing anybody could file — no
/// overall, no checks, no queue. It is a Fail and it is written down, because "nothing was
/// captured" is a real thing to say about a run and it is the only honest thing to say about
/// that one.
///
/// It belongs to no check. Every V-check reads a parsed leg, and on a trace with no bound
/// session there is nothing for any of them to read — they report their own absences, none
/// of which names the reason. So this is raised beside `seal_findings` and
/// `unrendered_gap_findings`, in `Pipeline::verdict`, over the descriptor itself.
pub(crate) fn no_session_finding(trace: &crate::trace::TraceDescriptor) -> Vec<Finding> {
    if !trace.tracked_sessions.is_empty() {
        return Vec::new();
    }
    vec![crate::checks::fail(
        "no_session",
        format!(
            "entry `{}` bound no session: its trace names no `tracked_sessions`, so no \
             harness ever started in the workspace the tracker armed. There is nothing here \
             to judge, and nothing any check can read.",
            trace.entry_id
        ),
        vec![Evidence::note("trace", "tracked_sessions is empty")],
    )]
}

/// Gaps that V4's rendering rules would not turn into a finding.
///
/// V4 renders a hole as `dirty_gap`/`clean_gap`, and every reason carrying a code of its
/// own under that code; a dirty gap that named no coded reason still gets `dirty_gap`. The
/// complement is exactly this: a gap that is not a hole and whose reasons all lack a code.
/// `classify` cannot produce one — `RecordsAppended` is the only codeless reason and it is
/// only ever pushed inside a hole — so this returns empty on every trace the crate has. It
/// exists because "cannot happen" is a property of two files agreeing, and if they stop
/// agreeing a classified gap would otherwise disappear silently between the classifier and
/// the document.
pub(crate) fn unrendered_gap_findings(gaps: &[Gap]) -> Vec<Finding> {
    gaps.iter()
        .filter(|g| !g.is_hole && g.reasons.iter().all(|r| r.code().is_none()))
        .map(|g| {
            crate::checks::flag(
                "unrendered_gap",
                format!(
                    "the classifier reported a gap over chain seq {}..{} that no check \
                     rendered; {} record(s) lie in it",
                    g.from_seq,
                    g.to_seq,
                    g.appended.len()
                ),
                g.appended
                    .iter()
                    .take(3)
                    .map(|l| {
                        Evidence::at(
                            &l.store,
                            l.seq,
                            Some(l.sha256.clone()),
                            "in an unrendered gap",
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

/// Where the key used to check acknowledgement signatures came from.
///
/// `TraceDescriptor.server_public_key` is a field the participant writes, so trusting it
/// unconditionally would let the same person sign the acknowledgements and name the key
/// that verifies them. The verdict records the answer rather than leaving a reader to
/// guess it: `Pinned` is the verifier's own allowlist (`crate::SERVER_PUBLIC_KEYS`),
/// `Descriptor` is the development escape hatch (`--dev-descriptor-key`), and `None` is
/// "no signature was checked at all" — which V12 reports as `anchor_unverified` whenever
/// there were acknowledgements to check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    /// The descriptor's key is on the verifier's embedded allowlist.
    Pinned,
    /// The descriptor's key was accepted because the caller passed
    /// `allow_descriptor_key`. Never the default, and never available to a published run.
    Descriptor,
    /// The caller supplied the key on the command line
    /// (`VerifyOptions::extra_server_keys`, `goalcup-verify --dev-key <hex>`). Serialises
    /// as `"supplied"`; every finding that rests on it says the run borrowed its trust.
    ///
    /// It is a variant of its own, and not an extra entry in `SERVER_PUBLIC_KEYS`, because
    /// a development signer whose seed is published in this repository must never be
    /// indistinguishable from a production anchor — not in the constant, and not in a
    /// verdict a reader has to interpret.
    Supplied,
    /// No key was used: the descriptor names none and the caller supplied none, or it
    /// names one this build does not recognise and no dev flag was given.
    None,
}

/// Whether the connector manifest that judged this run is the one this build pins.
///
/// The manifest supplies the human rule, the exclusion catalogue and the tool classes, and
/// `trace.toml` points at it with a participant-written path. `connector_manifest_sha256`
/// says which bytes were read; this says whether those bytes are the rules this build
/// ships. A verdict that recorded the digest and not the answer would leave every reader
/// to look the digest up by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestPin {
    /// The digest of the bytes read equals this build's pin for the connector.
    Pinned,
    /// This build pins a manifest for the connector, and it is not the one that was read.
    Altered,
    /// This build pins no manifest for the connector at all.
    Unpinned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanMessageSummary {
    pub index: usize,
    pub text: String,
    pub leg1_store: String,
    pub leg1_seq: u64,
    pub leg1_sha256: String,
    /// `None` for an unwitnessed message — leg 1 recorded it and leg 2 did not.
    /// Serialised as absent rather than as a sentinel, so a reader cannot mistake a
    /// missing post for post index 0.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub leg2_index: Option<usize>,
}

impl From<&HumanMessage> for HumanMessageSummary {
    fn from(m: &HumanMessage) -> Self {
        HumanMessageSummary {
            index: m.index,
            text: m.text.clone(),
            leg1_store: m.leg1_store.clone(),
            leg1_seq: m.leg1_seq,
            leg1_sha256: m.leg1_sha256.clone(),
            leg2_index: m.leg2_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTokenTotals {
    pub model: String,
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSummary {
    pub by_model: Vec<ModelTokenTotals>,
    /// Plan, credit and billing fields dropped before publication (V11).
    pub stripped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub verifier_version: String,
    pub schema_version: String,
    pub connector_id: String,
    /// SHA-256 of the connector manifest's bytes exactly as `verify_trace` loaded them.
    ///
    /// The descriptor points at the manifest with a participant-written path, so
    /// `connector_id` alone says only what that file *called* itself. This is the digest a
    /// reviewer compares against the shipped `connectors/<id>/manifest.toml`.
    pub connector_manifest_sha256: String,
    /// Whether that digest is the one this build pins for the connector. Carried in the
    /// document because a digest without an answer is a lookup a reader has to perform,
    /// and because a matching pin raises no finding — silence is what "pinned" looks like
    /// in the review queue, and this is where it is said out loud.
    pub manifest_pin: ManifestPin,
    pub entry_id: String,
    pub harness_version: String,
    /// Which key checked the acknowledgement signatures. Carried in the
    /// document because "the signatures verified" means nothing without it.
    pub key_source: KeySource,
    /// Whether this verdict was produced with `goalcup-verify --dev`.
    ///
    /// A verdict is a file people pass around, and every other `--dev` effect already
    /// leaves a sentence in `checks` — `manifest_not_pinned` becomes a Flag that says so,
    /// `connector_dir_unpinned` and `placeholder_server_key` become Passes that say so.
    /// This is the one word that says it without the reader having to find the sentence,
    /// and it is always serialised: a document that omits it is a document from a build
    /// that could not answer, which is a different fact from `false`.
    #[serde(default)]
    pub dev: bool,
    pub overall: Severity,
    pub human_messages: Vec<HumanMessageSummary>,
    pub checks: Vec<CheckResult>,
    pub review_queue: Vec<Finding>,
    pub gaps: Vec<Gap>,
    pub seal_attribution: Vec<SealAttribution>,
    pub tokens: TokenSummary,
    /// Leg-1 record kinds the parser recognised and deliberately ignored, counted
    /// (`ParsedLegs::census`).
    ///
    /// A statement and never a finding: a dropped kind is not by itself evidence of
    /// anything, so nothing here reaches `review_queue` or moves `overall`. It is in the
    /// document because the alternative is silence: `last-prompt` and `queue-operation`,
    /// both of which carry the human's text verbatim, are among the kinds the Claude parser
    /// drops.
    ///
    /// Scoped to the entry's own stores: a `foreign = true` store's dropped kinds are
    /// another session's and are V10's story, not this map's. A `BTreeMap`, so two runs
    /// over the same directory serialise to the same bytes.
    pub parser_census: std::collections::BTreeMap<String, u64>,
    /// Which of the participant's own harness settings each launch carried into this run's
    /// per-run home, from `EventBody::HomeSeeded`, in launch order.
    ///
    /// A plain fact list. No check reads it, no finding rests on it, and it does not move
    /// `overall`. Harness settings are the participant's decision and the tracker's job is
    /// to record which ones were carried — a run whose agent had browser tools
    /// (`claudeInChromeDefaultEnabled`) is not the same run as one whose agent did not, and
    /// a reader is owed that either way.
    ///
    /// What a record may hold is the protocol's rule and not this document's: a key's value
    /// is carried for a boolean, a number and a string of at most 32 bytes, and an object, an
    /// array, `null` or a longer string is recorded by name only; a seeded file is a name and
    /// a byte length, never its contents and never a digest of them. A record's `written`
    /// list is the one thing the tracker decided rather than copied — this run's own
    /// workspace, marked trusted — kept apart so a reader can tell the two apart.
    ///
    /// **It is the CLI's word.** A participant who edits the per-run home by hand after the
    /// seed is not caught by this — the config watch and the hooks are what catch that.
    ///
    /// Empty on a trace with no `HomeSeeded` event, and skipped when empty, so a verdict
    /// over an older chain serialises to exactly the bytes it did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried_settings: Vec<goalcup_protocol::events::HomeSeedRecord>,
}

impl Verdict {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn exit_code(&self) -> i32 {
        match self.overall {
            Severity::Pass => 0,
            Severity::Flag => 1,
            Severity::Fail => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_follows_the_overall_severity() {
        let mut v = Verdict {
            verifier_version: "0".to_string(),
            schema_version: crate::atif::SCHEMA_VERSION.to_string(),
            connector_id: "c".to_string(),
            connector_manifest_sha256: "00".repeat(32),
            manifest_pin: ManifestPin::Pinned,
            entry_id: "e".to_string(),
            harness_version: "0".to_string(),
            key_source: KeySource::None,
            dev: false,
            overall: Severity::Pass,
            human_messages: Vec::new(),
            checks: Vec::new(),
            review_queue: Vec::new(),
            gaps: Vec::new(),
            seal_attribution: Vec::new(),
            tokens: TokenSummary {
                by_model: Vec::new(),
                stripped: Vec::new(),
            },
            parser_census: std::collections::BTreeMap::new(),
            carried_settings: Vec::new(),
        };
        assert_eq!(v.exit_code(), 0);
        v.overall = Severity::Flag;
        assert_eq!(v.exit_code(), 1);
        v.overall = Severity::Fail;
        assert_eq!(v.exit_code(), 2);
        // Round-trips, so a committed verdict can be read back by a later tool.
        let back: Verdict = serde_json::from_str(&v.to_json().unwrap()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn seal_findings_report_the_two_unexplained_shapes_and_stay_silent_on_a_traced_file() {
        let attribution = vec![
            SealAttribution {
                // The workspace already held it, so there is no work to account for.
                path: "cargo.lock".to_string(),
                attribution: Attribution::Baseline {
                    sha256: "aa".repeat(32),
                },
            },
            SealAttribution {
                path: "hello.py".to_string(),
                attribution: Attribution::Traced {
                    call_id: "call_1".to_string(),
                    tool: "Write".to_string(),
                },
            },
            SealAttribution {
                path: "notes.md".to_string(),
                attribution: Attribution::ReadOnly {
                    call_id: "call_2".to_string(),
                },
            },
            SealAttribution {
                path: "planted.txt".to_string(),
                attribution: Attribution::Unexplained,
            },
        ];
        let findings = seal_findings(
            &attribution,
            &crate::checks::support::parsed(),
            &crate::checks::support::manifest("claude-code-cli"),
        );
        assert_eq!(
            findings.len(),
            2,
            "a traced file and a baseline file both raise nothing"
        );
        assert_eq!(findings[0].code, "read_only_attribution");
        assert_eq!(findings[0].severity, Severity::Flag);
        assert!(findings[0].message.contains("notes.md"));
        assert!(findings[0].evidence[0].note.contains("call_2"));
        assert_eq!(findings[1].code, "unexplained_seal_file");
        // D35: a Flag, not a Fail. The message stays loud — the finding is
        // still the seal's whole job — but no verifier parses every command, so it is an
        // organizer's question and not a verdict.
        assert_eq!(findings[1].severity, Severity::Flag);
        assert!(findings[1].message.contains("planted.txt"));
        assert!(
            findings[1]
                .message
                .contains("the workspace holds work the trace does not account for"),
            "downgrading the severity must not soften the message"
        );
    }

    /// Bulk attribution at the boundary, and the `Baseline` exemption alongside it. The
    /// `ParsedLegs` here carries no calls at all, so this also pins the fallback: a call
    /// the trajectory does not contain is classified by the tool name the attribution
    /// recorded, which `attribute_seal` took from that same trajectory.
    #[test]
    fn bulk_attribution_counts_sealed_files_per_shell_call_and_only_above_the_limit() {
        use crate::attribution::BULK_ATTRIBUTION_THRESHOLD as LIMIT;
        let bash = |n: usize| -> Vec<SealAttribution> {
            (0..n)
                .map(|i| SealAttribution {
                    path: format!("imported/f{i}.py"),
                    attribution: Attribution::Traced {
                        call_id: "call_bulk".to_string(),
                        tool: "Bash".to_string(),
                    },
                })
                .collect()
        };
        let findings = |a: &[SealAttribution]| {
            seal_findings(
                a,
                &crate::checks::support::parsed(),
                &crate::checks::support::manifest("claude-code-cli"),
            )
        };
        let over = findings(&bash(LIMIT + 1));
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].code, "bulk_attribution");
        assert_eq!(over[0].severity, Severity::Flag);
        assert!(over[0].message.contains("call_bulk"));
        assert!(over[0].message.contains(&format!("{}", LIMIT + 1)));
        // The limit itself is not a finding.
        assert!(findings(&bash(LIMIT)).is_empty());
        // A baseline file attributed to nothing cannot push a call over the limit, since
        // it is attributed to no call at all.
        let mut mixed = bash(LIMIT);
        mixed.push(SealAttribution {
            path: "vendored.py".to_string(),
            attribution: Attribution::Baseline {
                sha256: "bb".repeat(32),
            },
        });
        assert!(findings(&mixed).is_empty());
    }

    #[test]
    fn a_gap_v4_would_not_render_still_reaches_the_queue() {
        use crate::gaps::{Gap, GapKind, GapLine, GapReason};
        // Not a hole, and `RecordsAppended` is the one reason with no code of its own:
        // V4 renders nothing for this gap, so the document has to.
        let silent = Gap {
            kind: GapKind::Dirty,
            is_hole: false,
            from_seq: 4,
            to_seq: 9,
            from: None,
            to: None,
            appended: vec![GapLine {
                store: "main".to_string(),
                seq: 7,
                sha256: "ab".to_string(),
            }],
            reasons: vec![GapReason::RecordsAppended],
        };
        // A hole is V4's own business, and so is any reason carrying a code.
        let hole = Gap {
            is_hole: true,
            ..silent.clone()
        };
        let coded = Gap {
            reasons: vec![GapReason::Unanchored],
            ..silent.clone()
        };
        let findings = unrendered_gap_findings(&[silent, hole, coded]);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "unrendered_gap");
        assert!(findings[0].message.contains("4..9"));
        assert_eq!(findings[0].evidence[0].seq, Some(7));
    }

    #[test]
    fn key_source_serialises_as_the_four_documented_strings() {
        for (source, text) in [
            (KeySource::Pinned, "\"pinned\""),
            (KeySource::Descriptor, "\"descriptor\""),
            (KeySource::Supplied, "\"supplied\""),
            (KeySource::None, "\"none\""),
        ] {
            assert_eq!(serde_json::to_string(&source).unwrap(), text);
        }
    }
}
