//! V9: hook-config change events reconciled against the traced tool calls, with the
//! window during which capture may have been degraded (D6) — **and the on-disk
//! signatures of a rewritten transcript**.
//!
//! What this check answers, in one sentence: *did anything change underneath the capture,
//! and can the agent's own traced work explain it?*
//!
//!   * **`store_rewritten`.** `EventBody::TailNotice` carries the two filesystem signatures
//!     of a rewritten transcript: the file shrank below the saved offset, or the inode at
//!     the path changed. A re-read from offset 0 would otherwise legitimise the doctored
//!     file, because D19 expects "a second complete chain" after one. A truncation or inode
//!     replacement **not declared an expected resume re-read** is a `store_rewritten`
//!     finding at `Severity::Fail`.
//!   * **A traced write does not exonerate a leg-1 store.** A traced call that wrote a
//!     leg-1 store's own path is not an explanation of the truncation, it is the attack in
//!     the trace admitting itself. The harness writes a transcript; an agent does not. The
//!     attribution is still computed and goes into the Fail, naming the call. The same
//!     disjunct stays for `ConfigDeviation`, where D6 makes it the right reading.
//!   * **The declaration needs a witness.** `expected_reread` is written by the
//!     participant, so it is honoured only where the chain carries its own corroboration at
//!     a lower seq than the notice: a `DaemonRestart`, or a `TailNotice` of kind `Reread`
//!     for the same store, **with no leg-1 record of that store chained in between**. See
//!     `reread_witness`.
//!   * **Config-deviation attribution.** Every config deviation is attributed to a traced
//!     tool call, or reported as unattributed.
//!   * **One attribution rule.** A call "explains" a rewrite only by the rule in
//!     `crate::attribution`, which seal attribution shares, so "did this call write that
//!     file?" has one answer in the verifier. A write operator is matched as a whole token
//!     against the **command field only**. A bare mention (`echo /path`) or a basename-only
//!     match never attributes, and a shell call whose arguments carry no command text
//!     attributes nothing.
//!   * **No leaked bytes.** A finding never interpolates a full path, a diff, or a raw hook
//!     body: `ConfigDeviation`/`TailNotice` findings cite `store`, `seq`, `hash`, and the
//!     config file's workspace-relative name (its basename) — never the rendered
//!     `{home}`/`{config_dir}` path underneath it — plus, for a deviation, the diff's
//!     *length*, never its text. `ConfigFact.detail` (a raw hook body — the whole POST body
//!     for Claude Code's `ConfigChange`) is never interpolated either; only its length is.
//!   * **An absent event stream is unverifiable, not clean.** An M0
//!     trace carries no daemon event stream at all, so `ConfigDeviation`/`TailNotice`
//!     reconciliation cannot run — that is a fact worth stating (`no_event_stream`, Flag),
//!     not the same as having reconciled and found nothing (`no_config_deviation`, Pass).
//!   * **An honest note.** Claude Code's `arming.signal` is
//!     `"launch+first_transcript_record"` — the name of a *procedure*, not of a message.
//!     Nothing on either leg ever emits it as a `ConfigFact`, so the arming branch below
//!     counts zero on that connector and is dead code there. That is correct and is
//!     recorded so nobody "fixes" it by inventing an event.

use super::{Check, CheckContext, CheckResult, Evidence, Finding, fail, flag, pass};
use crate::attribution::{Purpose, path_arg_attributes, shell_attributes, shell_command_text};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::{HealthState, TailNoticeKind};
use serde_json::Value;
use std::collections::BTreeSet;

/// The last path segment — the config file's workspace-relative name — never the
/// rendered `{home}`/`{config_dir}` path underneath it.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// How a notice reads as an on-disk signature of a rewritten transcript, or `None` when
/// the kind is not one of those signatures.
///
/// Three kinds are not. `Reread` is a consequence rather than a cause: the notice that
/// caused it is the one being judged. `OverlongLine` and `OversizeFile` are the daemon's
/// own capacity bounds — the tailer declining to carry bytes it measured — and reading a
/// bound as tamper evidence is exactly the confusion that gave them their own kinds.
fn rewrite_signature(kind: &TailNoticeKind) -> Option<&'static str> {
    match kind {
        TailNoticeKind::Truncated => Some("truncated below the saved offset"),
        TailNoticeKind::FileReplaced => Some("replaced at the inode level"),
        TailNoticeKind::Reread | TailNoticeKind::OverlongLine | TailNoticeKind::OversizeFile => {
            None
        }
    }
}

/// Where this descriptor declares that a re-read from zero was expected.
///
/// Two homes, one meaning. A **chain** descriptor declares no `[[leg1]]` — the stores are
/// lifted out of the chain — so the flag sits at the top level of `trace.toml`
/// (`TraceDescriptorDoc::expected_reread`, which is what `TraceExport` writes). A **legacy**
/// M0 descriptor names its stores as files, and the flag sits on the store it is about. The
/// branch is on which kind of descriptor this is rather than on which field happens to be
/// set, because `TraceDescriptor::load` refuses a descriptor that is both and a check that
/// read both would be answering a question the loader already refused to ask.
fn declares_expected_reread(cx: &CheckContext, store: &str) -> bool {
    match cx.chain {
        Some(_) => cx.trace.expected_reread,
        None => cx
            .trace
            .leg1
            .iter()
            .any(|s| s.name == store && s.expected_reread),
    }
}

/// The seq of the chain's own witness that a re-read from zero was coming, or `None`.
///
/// `expected_reread` is a **participant-written** switch: honoured on the participant's
/// word alone, declaring it would turn an unexplained truncation — the on-disk signature of
/// a rewritten transcript — from a Fail into a Flag. Its sibling `Leg1Spec.foreign` is
/// gated behind a live detection (`live == 0` in V10); this is the same gate for the same
/// reason. The declaration says how to read a witness the chain carries, and says nothing
/// on its own.
///
/// Two things count as a witness, and both are links the daemon stamped:
///
///   * a `DaemonRestart` envelope — the tailer coming back up, which is when a re-read
///     from offset 0 is the correct behaviour rather than a suspicious one; and
///   * a `TailNotice { kind: Reread }` **for this store** — the tailer saying it had
///     already re-read this file, so the notice being judged is not the first sign of it.
///
/// A `DaemonRestart` is not filtered by store because it is not about one: a restarting
/// daemon re-reads every store it tails.
///
/// **At a lower seq than the notice being judged.** A witness above it explains nothing —
/// the truncation had already happened — and accepting one would let a participant
/// truncate a store and then restart the daemon to license it after the fact.
///
/// **And with nothing of this store read in between.** `DaemonRestart` is store-agnostic,
/// so an unbounded lookback would let one restart at seq 3 license every truncation of
/// every store for the rest of the run. What a restart actually licenses is the re-read
/// that follows it, and only until the tailer has an offset again: once a leg-1 record of
/// this store is chained above the witness, the daemon demonstrably read the file, so a
/// truncation after that is a second event the witness says nothing about. The bound is
/// therefore the interval `(witness, notice)`, and a leg-1 record of the truncated store
/// inside it spends the witness. Records of other stores do not — they are no evidence that
/// this file was read.
///
/// Only the highest candidate is tested, which is not a shortcut: a lower witness spans a
/// superset of that interval, so any record that spends the highest spends every lower one
/// too.
///
/// `all` is the whole envelope stream (`VerifiedChain::envelopes`), not `cx.events`: the
/// loader splits leg-1 records out of `events`, so the records this rule is about are only
/// visible there.
fn reread_witness(all: &[Envelope], store: &str, notice_seq: u64) -> Option<u64> {
    let witness = all
        .iter()
        .filter(|e| e.link.seq < notice_seq)
        .filter(|e| match &e.body {
            EventBody::DaemonRestart { .. } => true,
            EventBody::TailNotice {
                store: s,
                kind: TailNoticeKind::Reread,
                ..
            } => s == store,
            _ => false,
        })
        .map(|e| e.link.seq)
        .max()?;
    let read_since = all.iter().any(|e| {
        e.link.seq > witness
            && e.link.seq < notice_seq
            && matches!(&e.body, EventBody::Leg1Record { store: s, .. } if s == store)
    });
    (!read_since).then_some(witness)
}

/// How many arming signals a trace should carry, and what it means when it carries more.
///
/// **The launches are counted, not inferred from the signal count.** The number of arming
/// signals per launch is not a property of the harness, and a manifest is a rulebook rather
/// than a place to keep a measurement that moves. The trace already says how many launches
/// it holds: one launch loads one shim — measured, not guaranteed by anything — and the shim
/// mints an id at load and stamps every post with it, so the distinct shim ids over the
/// arming signals are the launches. Where a connector's shim mints no id, one signal is one
/// launch. A signal carrying no id where others carry one counts as a launch of its own, so
/// dropping the field from a second launch's posts cannot hide that launch behind a first
/// launch that stamped its own.
///
/// **What this does not close.** The id is participant-written: a shim that mints one
/// constant id and stamps it across two launches is indistinguishable here from a single
/// launch. Counting distinct ids catches a launch that forgot to hide itself, not a shim
/// built to hide them; what bounds that case is the witnessed-relaunch cap below and the
/// leg-1 evidence it reads, neither of which the shim writes.
///
/// **What it does instead is fail closed.** `launches` is the distinct stamped ids plus
/// every unstamped signal, so one launch whose arming posts are only partly stamped counts
/// twice — one instance and one orphan — and Flags a run that re-armed once. That is the
/// direction to be wrong in: the cost is a person looking at a clean trace, where the
/// opposite mistake would let a real second launch through in silence.
///
/// **What is flagged is a launch the trace cannot account for.** A run may be launched once
/// and relaunched once per relaunch the trace witnesses — a resume marker in leg 1 where
/// the harness leaves one, and otherwise a chained `AgentExited` health event, which is the
/// daemon measuring the launch that ended and the next one beginning. So `launches <=
/// witnessed + 1`: the resume is what the rules ask for ("keep going", D8) and a re-arm on
/// top of it is not.
fn arming_finding(cx: &CheckContext, arming: &str, count: usize) -> Option<Finding> {
    if count == 0 {
        return None;
    }
    // The shim ids of the arming posts, which is where a launch is visible. A connector
    // whose shim mints none leaves this empty and each signal counts as its own launch.
    let shims: BTreeSet<&str> = cx
        .parsed
        .config_events
        .iter()
        .filter(|c| c.event == arming)
        .filter_map(|c| c.shim.as_deref())
        .collect();
    // A signal carrying no id belongs to none of those instances, so it is a launch of its
    // own rather than a signal discarded. Degrades to the signal count when nothing is
    // stamped and to the instance count when everything is.
    let unstamped = cx
        .parsed
        .config_events
        .iter()
        .filter(|c| c.event == arming && c.shim.is_none())
        .count();
    let launches = shims.len() + unstamped;
    let witnessed = relaunches(cx);
    let instances = match (shims.len(), unstamped) {
        (0, _) => "no shim instance id, so each signal is read as its own launch".to_string(),
        (n, 0) => format!("{n} distinct shim instance(s)"),
        (n, u) => format!(
            "{n} distinct shim instance(s) and {u} signal(s) carrying none, each read as \
             its own launch"
        ),
    };
    if launches > witnessed + 1 {
        return Some(flag(
            "arming_signals",
            format!(
                "{count} `{arming}` event(s) across {launches} launch(es) ({instances}), and \
                 the trace witnesses {witnessed} relaunch(es): a run may be launched once \
                 and launched again once per relaunch it can account for, so at least one \
                 of these is a session re-armed rather than the resume the rules ask for"
            ),
            Vec::new(),
        ));
    }
    if launches == 1 {
        return Some(pass(
            "arming_signals",
            format!("session armed once via `{arming}` ({count} signal(s), {instances})"),
        ));
    }
    Some(pass(
        "arming_signals",
        format!(
            "armed once per launch: {count} `{arming}` event(s) across {launches} launch(es) \
             ({instances}), and the trace witnesses {witnessed} relaunch(es) — a resume is \
             what the rules ask for, not a re-arm"
        ),
    ))
}

/// How many times this trace says the harness was launched again.
///
/// Leg 1 first, where the harness leaves a durable marker (Claude Code writes one, Codex
/// writes one). Otherwise the daemon's own chained evidence: an `AgentExited` health event
/// is the tracker measuring the launch that ended, and a run that went on recording
/// afterwards was relaunched.
fn relaunches(cx: &CheckContext) -> usize {
    if !cx.parsed.resume_markers.is_empty() {
        return cx.parsed.resume_markers.len();
    }
    cx.events
        .iter()
        .filter(|e| matches!(&e.body, EventBody::Health(h) if h.state == HealthState::AgentExited))
        .count()
}

pub struct V9Config;

impl Check for V9Config {
    fn id(&self) -> &'static str {
        "V9"
    }
    fn title(&self) -> &'static str {
        "hook-config change events reconciled"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings = Vec::new();
        let arming = cx.manifest.arming.signal.clone();
        let arming_count = cx
            .parsed
            .config_events
            .iter()
            .filter(|c| c.event == arming)
            .count();
        findings.extend(arming_finding(cx, &arming, arming_count));
        // `ConfigFact.detail` is a raw hook body — the whole POST body for Claude Code's
        // `ConfigChange` — and is never interpolated. Only its length is, alongside the
        // event name.
        for c in cx.parsed.config_events.iter().filter(|c| c.event != arming) {
            findings.push(flag(
                "config_event",
                format!(
                    "config event `{}`{}: {} byte(s) of detail (not shown; may carry a \
                     hook body or a machine path)",
                    c.event,
                    c.at.as_deref()
                        .map(|a| format!(" at {a}"))
                        .unwrap_or_default(),
                    c.detail.len()
                ),
                vec![Evidence::note("leg2", &c.event)],
            ));
        }

        // Every traced tool call, classified once, for the two reconciliations below.
        // Only write-class calls count: D11's rule is a traced edit, write or shell
        // command, and a `Read` naming a path explains nothing about how it changed.
        let writes: Vec<(&str, &str, bool, &Value, Option<&str>)> = cx
            .parsed
            .trajectory
            .steps
            .iter()
            .flat_map(|s| s.tool_calls.iter())
            .filter(|c| cx.manifest.tool_classes.write.contains(&c.function_name))
            .map(|c| {
                let is_shell = cx.manifest.tool_classes.shell.contains(&c.function_name);
                (
                    c.tool_call_id.as_str(),
                    c.function_name.as_str(),
                    is_shell,
                    &c.arguments,
                    shell_command_text(&c.arguments),
                )
            })
            .collect();

        let attributed_by = |full_path: &str| -> Option<(&str, &str)> {
            writes
                .iter()
                .find(|(_, _, is_shell, args, command)| {
                    if *is_shell {
                        // The command text only: an operator sitting in some other
                        // argument is not a write, and a shell call with no command text
                        // explains nothing.
                        command.is_some_and(|cmd| {
                            shell_attributes(cmd, full_path, Purpose::ConfigChange)
                        })
                    } else {
                        path_arg_attributes(args, full_path)
                    }
                })
                .map(|(call_id, tool, ..)| (*call_id, *tool))
        };

        // Attribute each config deviation to a traced write, or say it is unattributed.
        for env in cx.events {
            let EventBody::ConfigDeviation(d) = &env.body else {
                continue;
            };
            let name = basename(&d.path);
            match attributed_by(&d.path) {
                // Flag. D6: "An edit matched to a traced agent tool call is a flag reviewed
                // like any anomaly." The other reading, `unattributed_config_change` below,
                // is a Fail.
                Some((call_id, tool)) => findings.push(flag(
                    "config_change_attributed",
                    format!(
                        "`{name}` deviated at {} and is explained by the traced `{tool}` \
                         call {call_id}; restored = {}, arming_lost = {}",
                        d.detected_at, d.restored, d.arming_lost
                    ),
                    vec![Evidence::at(
                        "events",
                        env.link.seq,
                        Some(env.link.hash.clone()),
                        name,
                    )],
                )),
                None => findings.push(fail(
                    "unattributed_config_change",
                    format!(
                        "`{name}` deviated at {} and no traced write explains it — the \
                         tracker's own config changed underneath the capture (R9, D6). \
                         restored = {}, arming_lost = {}. diff length: {} byte(s)",
                        d.detected_at,
                        d.restored,
                        d.arming_lost,
                        d.diff.len()
                    ),
                    vec![Evidence::at(
                        "events",
                        env.link.seq,
                        Some(env.link.hash.clone()),
                        name,
                    )],
                )),
            }
        }

        // The two filesystem signatures of a rewritten transcript.
        //
        // The witness rule below has to see leg-1 records, which `chain::load_and_verify`
        // splits out of `events` — so it reads the whole stream where there is one. A
        // legacy M0 descriptor has no chain and no events either, so the fallback is
        // `cx.events`, which is also what a unit test builds by hand.
        let all_envelopes: &[Envelope] = match cx.chain {
            Some(c) => &c.envelopes,
            None => cx.events,
        };
        for env in cx.events {
            let EventBody::TailNotice {
                store,
                path,
                kind,
                offset_before,
                offset_after,
                inode_before,
                inode_after,
                at,
            } = &env.body
            else {
                continue;
            };
            let Some(signature) = rewrite_signature(kind) else {
                continue;
            };
            // The participant's declaration, and the chain's own corroboration of it.
            // Declared without a witness is not declared: `witness` is `None` and the
            // notice falls through to `store_rewritten` below.
            let declared = declares_expected_reread(cx, store);
            let witness = declared
                .then(|| reread_witness(all_envelopes, store, env.link.seq))
                .flatten();
            // A traced tool call that wrote a leg-1 store's own path is not an explanation
            // of the truncation; it is the attack, in the trace, admitting itself. No
            // honest run writes its own transcript through a tool call — the harness writes
            // it. The attribution is still computed, because "which call did it" is exactly
            // what the reviewer wants said; it is said inside the Fail.
            //
            // The same disjunct stays where D6 makes it the right reading: a traced call
            // that edited a watched config file explains that deviation
            // (`config_change_attributed`, above).
            let traced = attributed_by(path);
            if let Some(seq) = witness {
                findings.push(flag(
                    "store_reread",
                    format!(
                        "`{store}` was re-read from zero at {at} ({kind:?}); the trace \
                         declares this an expected resume re-read and the chain carries a \
                         restart or an earlier re-read at seq {seq} that corroborates it"
                    ),
                    Vec::new(),
                ));
                continue;
            }
            // Rendered rather than `{:?}`-ed: `inode Some(8612004) -> Some(8612004)` is
            // Rust talking to itself inside a finding a human has to act on. An inode
            // that did not move is the file staying put — a truncation in place, which is
            // a different story from a replacement — and a platform that reports no inode
            // says `none`, not `None`.
            let render = |i: Option<u64>| i.map_or_else(|| "none".to_string(), |v| v.to_string());
            let inode = if inode_before == inode_after {
                match inode_before {
                    Some(v) => format!("inode {v} unchanged"),
                    None => "inode none".to_string(),
                }
            } else {
                format!(
                    "inode {} -> {}",
                    render(*inode_before),
                    render(*inode_after)
                )
            };
            // What the traced call adds to the Fail.
            let by_call = match traced {
                Some((call_id, tool)) => format!(
                    "A traced `{tool}` call ({call_id}) wrote this store's own path — the \
                     harness writes a transcript and an agent does not, so that is the \
                     rewrite naming itself rather than an excuse for it"
                ),
                None => "No traced tool call explains it".to_string(),
            };
            // Two ways the second half of that sentence can be true, said apart.
            // A trace that declared nothing and a trace that declared an expected re-read
            // the chain does not corroborate are different documents, and a reviewer who
            // wrote `expected_reread = true` needs to be told that the declaration was
            // read and found unwitnessed rather than that it was not there.
            let unexplained = if declared {
                "the trace declares an expected re-read and the chain carries no unspent \
                 witness for it — no `DaemonRestart` or earlier `Reread` notice for this \
                 store sits below it with no record of the store read since — so the \
                 declaration is the participant's word and nothing else"
            } else {
                "the trace does not declare an expected re-read"
            };
            findings.push(fail(
                "store_rewritten",
                format!(
                    "`{store}` was {} at {at}: offset {offset_before} -> {offset_after}, \
                     {inode}. {by_call} and {unexplained}. \
                     These are the on-disk signatures of a rewritten transcript, and the \
                     re-read that follows chains the file as it is now, not as it was.",
                    signature
                ),
                vec![Evidence::at(
                    store,
                    env.link.seq,
                    Some(env.link.hash.clone()),
                    format!("offset {offset_before} -> {offset_after}"),
                )],
            ));
        }

        // Reconciliation above needs `cx.events` to run at all. An M0
        // trace carries no daemon event stream, so "nothing to reconcile against" is
        // unverifiable, not the same fact as "reconciled and found nothing".
        if cx.events.is_empty() {
            findings.push(flag(
                "no_event_stream",
                "no daemon event stream in this trace; config-deviation and \
                 store-rewrite reconciliation cannot be verified here (unverifiable, not \
                 clean)"
                    .to_string(),
                Vec::new(),
            ));
        } else if findings.is_empty() {
            findings.push(pass(
                "no_config_deviation",
                "no tracker config file deviated and no store was truncated or replaced \
                 during the capture"
                    .to_string(),
            ));
        }
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atif::{Message, Step, StepSource, ToolCall};
    use crate::checks::{Severity, support};
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::envelope::Envelope;
    use goalcup_protocol::events::ConfigDeviation;
    use serde_json::json;

    /// An M0 fixture carries no event stream, so `ConfigDeviation` and `TailNotice`
    /// reconciliation are only reachable by construction. This builds the daemon's
    /// chained stream directly.
    fn env(seq: u64, body: EventBody) -> Envelope {
        Envelope {
            protocol_version: 1,
            entry_id: "test-entry".to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq,
                prev: "0".repeat(64),
                hash: "h".repeat(64),
            },
            body,
            redaction: None,
        }
    }

    fn write_step(tool_call_id: &str, function_name: &str, args: Value) -> Step {
        Step {
            step_id: 1,
            timestamp: None,
            source: StepSource::Agent,
            model_name: None,
            message: Message::Text(String::new()),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                tool_call_id: tool_call_id.to_string(),
                function_name: function_name.to_string(),
                arguments: args,
                extra: None,
            }],
            observation: None,
            metrics: None,
            llm_call_count: None,
            is_copied_context: None,
            extra: None,
        }
    }

    fn deviation(path: &str) -> ConfigDeviation {
        ConfigDeviation {
            path: path.to_string(),
            expected_sha256: "a".repeat(64),
            observed_sha256: "b".repeat(64),
            detected_at: "2026-09-04T00:00:00Z".to_string(),
            diff: "- old\n+ new".to_string(),
            restored: true,
            arming_lost: false,
        }
    }

    fn cx_with<'a>(
        trace: &'a crate::trace::TraceDescriptor,
        manifest: &'a goalcup_protocol::manifest::ConnectorManifest,
        parsed: &'a crate::parsers::ParsedLegs,
        human: &'a crate::human_rule::HumanRuleOutcome,
        events: &'a [Envelope],
    ) -> CheckContext<'a> {
        CheckContext {
            trace,
            manifest,
            parsed,
            human,
            join_strategy: "prompt_id",
            stores: &[],
            events,
            acks: &[],
            acks_verified: &crate::gaps::NO_ACKS,
            seal: None,
            gaps: &[],
            server_key: None,
            key_source: crate::KeySource::None,
            dev_mode: false,
            chain: None,
            receipt: None,
            baseline: None,
            manifest_sha256: "",
            manifest_pin: crate::ManifestPinning::default(),
        }
    }

    /// A non-shell write tool's path-typed argument, equal to the full path, attributes;
    /// one that names something else does not.
    #[test]
    fn a_config_deviation_is_attributed_to_a_traced_write_or_reported_as_unexplained() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.trajectory.steps.push(write_step(
            "c1",
            "Write",
            json!({"file_path": "hooks.json"}),
        ));
        let human = support::human();
        let events = vec![
            env(1, EventBody::ConfigDeviation(deviation("hooks.json"))),
            env(2, EventBody::ConfigDeviation(deviation("unrelated.json"))),
        ];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "config_change_attributed" && f.message.contains("hooks.json"))
        );
        let unattributed = result
            .findings
            .iter()
            .find(|f| f.code == "unattributed_config_change")
            .expect("the second deviation names no traced write");
        assert_eq!(unattributed.severity, Severity::Fail);
        assert!(unattributed.message.contains("unrelated.json"));
        assert!(unattributed.message.contains("diff length"));
        assert!(
            !unattributed.message.contains("- old"),
            "the diff text itself must not appear"
        );
    }

    /// A bare mention of the path (`echo /path`) is not a write, even
    /// though `echo` appears in a shell command that also names the path.
    #[test]
    fn a_bare_mention_of_the_path_does_not_attribute() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.trajectory.steps.push(write_step(
            "c1",
            "Bash",
            json!({"command": "echo /Users/real/.claude/settings.json"}),
        ));
        let human = support::human();
        let events = vec![env(
            1,
            EventBody::ConfigDeviation(deviation("/Users/real/.claude/settings.json")),
        )];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "unattributed_config_change"),
            "echo is a bare mention, not a write operator"
        );
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == "config_change_attributed")
        );
    }

    /// A genuine shell write — the full path plus a write operator —
    /// does attribute.
    #[test]
    fn a_genuine_shell_write_attributes() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.trajectory.steps.push(write_step(
            "c1",
            "Bash",
            json!({"command": "cat header.txt > /Users/real/.claude/settings.json"}),
        ));
        let human = support::human();
        let events = vec![env(
            1,
            EventBody::ConfigDeviation(deviation("/Users/real/.claude/settings.json")),
        )];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "config_change_attributed")
        );
    }

    /// A basename-only match — the command names `settings.json` but
    /// not the full path the deviation carries — must never attribute, shell or not.
    #[test]
    fn a_basename_only_match_does_not_attribute() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.trajectory.steps.push(write_step(
            "c1",
            "Bash",
            json!({"command": "cat header.txt > settings.json"}),
        ));
        parsed.trajectory.steps.push(write_step(
            "c2",
            "Write",
            json!({"file_path": "settings.json"}),
        ));
        let human = support::human();
        let events = vec![env(
            1,
            EventBody::ConfigDeviation(deviation("/Users/real/.claude/settings.json")),
        )];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "unattributed_config_change"),
            "a basename match is not a full-path match"
        );
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == "config_change_attributed")
        );
    }

    /// The write operator is matched as a whole token against the **command field only**,
    /// so the `rm` inside a `description` — or inside the word "Performance" — does not
    /// make a bare mention of the path look like a write.
    #[test]
    fn an_operator_outside_the_command_or_inside_a_word_does_not_attribute() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.trajectory.steps.push(write_step(
            "c1",
            "Bash",
            json!({
                "command": "cat /Users/real/.claude/settings.json",
                "description": "rm the stale copy > /dev/null",
            }),
        ));
        parsed.trajectory.steps.push(write_step(
            "c2",
            "Bash",
            json!({
                "command": "python3 -c \"print('Performance of \
                            /Users/real/.claude/settings.json')\"",
            }),
        ));
        let human = support::human();
        let events = vec![env(
            1,
            EventBody::ConfigDeviation(deviation("/Users/real/.claude/settings.json")),
        )];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "unattributed_config_change"),
            "neither call wrote the file: one reads it, the other prints its name"
        );
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == "config_change_attributed")
        );
    }

    /// An `apply_patch`-DSL target path, extracted from wherever the argument text
    /// carries the header line, attributes exactly like a plain `file_path` argument.
    #[test]
    fn an_apply_patch_dsl_target_path_attributes() {
        let trace = support::trace("codex-cli");
        let manifest = support::manifest("codex-cli");
        let mut parsed = support::parsed();
        parsed.trajectory.steps.push(write_step(
            "c1",
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Update File: hooks.json\n+x\n*** End Patch"
            }),
        ));
        let human = support::human();
        let events = vec![env(1, EventBody::ConfigDeviation(deviation("hooks.json")))];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "config_change_attributed")
        );
    }

    /// A `TailNotice` truncation or inode replacement with no traced write and no
    /// `expected_reread` declaration is `store_rewritten` at `Severity::Fail`— and
    /// the finding cites only the store name and the offsets, never the on-disk path.
    #[test]
    fn an_unexplained_truncation_is_store_rewritten_without_a_path() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let human = support::human();
        let events = vec![env(
            1,
            EventBody::TailNotice {
                store: "main".to_string(),
                path: "/Users/real/.claude/projects/x/main.jsonl".to_string(),
                kind: TailNoticeKind::Truncated,
                offset_before: 4096,
                offset_after: 128,
                inode_before: Some(1),
                inode_after: Some(1),
                at: "2026-09-04T00:00:00Z".to_string(),
            },
        )];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "store_rewritten")
            .expect("an unexplained truncation must be reported");
        assert!(f.message.contains("main"));
        assert!(
            !f.message.contains("/Users/"),
            "leaks a path: {}",
            f.message
        );
        assert!(!f.evidence.is_empty());
        for e in &f.evidence {
            assert!(!e.note.contains("/Users/"), "leaks a path: {}", e.note);
        }
    }

    /// A legacy descriptor declaring `Leg1Spec.expected_reread` for the store, with a
    /// `DaemonRestart` below the notice to corroborate it.
    fn legacy_expected_reread_trace() -> crate::trace::TraceDescriptor {
        let mut trace = support::trace("claude-code-cli");
        trace.leg1.push(crate::trace::Leg1Spec {
            name: "main".to_string(),
            kind: crate::trace::StoreKind::RawJsonl,
            path: crate::trace_fs::TracePath::parse("test.jsonl").unwrap(),
            foreign: false,
            scrubbed: false,
            expected_reread: true,
        });
        trace
    }

    fn replaced_at(seq: u64) -> Envelope {
        env(
            seq,
            EventBody::TailNotice {
                store: "main".to_string(),
                path: "main.jsonl".to_string(),
                kind: TailNoticeKind::FileReplaced,
                offset_before: 4096,
                offset_after: 0,
                inode_before: Some(1),
                inode_after: Some(2),
                at: "2026-09-04T00:00:00Z".to_string(),
            },
        )
    }

    fn restart_at(seq: u64) -> Envelope {
        env(
            seq,
            EventBody::DaemonRestart {
                at: "2026-09-04T00:00:00Z".to_string(),
                previous_head_seq: seq.saturating_sub(1),
                previous_head_hash: "h".repeat(64),
            },
        )
    }

    /// The same truncation is a `store_reread` flag, not a fail, once the trace declares
    /// it an expected resume re-read (`Leg1Spec.expected_reread`) **and** the chain
    /// corroborates it. That witness is the only downgrade; see
    /// `a_traced_write_to_the_store_does_not_downgrade_the_rewrite`.
    #[test]
    fn a_truncation_declared_expected_and_corroborated_is_a_reread_flag() {
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let trace = legacy_expected_reread_trace();
        let parsed = support::parsed();
        let events = vec![restart_at(1), replaced_at(2)];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Flag);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "store_reread" && f.message.contains("expected resume re-read"))
        );
        assert!(!result.findings.iter().any(|f| f.code == "store_rewritten"));
    }

    /// A traced tool call that wrote the truncated store's own path does not turn this Fail
    /// into a `store_reread` Flag. No honest run writes its own transcript through a tool
    /// call — the harness writes it. The Fail stays, and it names the call.
    #[test]
    fn a_traced_write_to_the_store_does_not_downgrade_the_rewrite() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let mut parsed = support::parsed();
        parsed.trajectory.steps.push(crate::atif::Step {
            step_id: 0,
            timestamp: None,
            source: crate::atif::StepSource::Agent,
            model_name: None,
            message: crate::atif::Message::Text(String::new()),
            reasoning_content: None,
            tool_calls: vec![crate::atif::ToolCall {
                tool_call_id: "call_1".to_string(),
                function_name: "Write".to_string(),
                arguments: serde_json::json!({ "file_path": "/tmp/main.jsonl" }),
                extra: None,
            }],
            observation: None,
            metrics: None,
            llm_call_count: None,
            is_copied_context: None,
            extra: None,
        });
        let events = vec![env(
            1,
            EventBody::TailNotice {
                store: "main".to_string(),
                path: "/tmp/main.jsonl".to_string(),
                kind: TailNoticeKind::Truncated,
                offset_before: 4096,
                offset_after: 0,
                inode_before: Some(1),
                inode_after: Some(1),
                at: "2026-09-04T00:00:00Z".to_string(),
            },
        )];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "store_rewritten")
            .expect("a traced write to the store's own path is the attack, not the excuse");
        assert!(
            f.message.contains("call_1") && f.message.contains("rewrite naming itself"),
            "the Fail must name the call rather than be silenced by it: {}",
            f.message
        );
        assert!(!f.message.contains("/tmp/"), "leaks a path: {}", f.message);
        assert!(!result.findings.iter().any(|f| f.code == "store_reread"));
    }

    /// The switch is the participant's to write, so writing it is not
    /// evidence. With no `DaemonRestart` and no earlier `Reread` notice anywhere in the
    /// chain, the declaration downgrades nothing.
    #[test]
    fn expected_reread_without_a_witness_downgrades_nothing() {
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let trace = legacy_expected_reread_trace();
        let parsed = support::parsed();
        let events = vec![replaced_at(2)];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "store_rewritten")
            .expect("an uncorroborated declaration is still a rewritten store");
        assert!(
            f.message
                .contains("the participant's word and nothing else"),
            "the finding must say the declaration was read and found unwitnessed: {}",
            f.message
        );
        assert!(!result.findings.iter().any(|f| f.code == "store_reread"));
    }

    /// A witness above the notice explains nothing: the truncation had already happened,
    /// and accepting one would let a participant truncate a store and restart the daemon
    /// to license it after the fact.
    #[test]
    fn a_witness_above_the_notice_does_not_corroborate_it() {
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let trace = legacy_expected_reread_trace();
        let parsed = support::parsed();
        let events = vec![replaced_at(2), restart_at(3)];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        assert!(result.findings.iter().any(|f| f.code == "store_rewritten"));
    }

    /// A `Reread` notice for another store is not a witness for this one.
    #[test]
    fn a_reread_notice_for_another_store_is_not_a_witness() {
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let trace = legacy_expected_reread_trace();
        let parsed = support::parsed();
        let events = vec![
            env(
                1,
                EventBody::TailNotice {
                    store: "subagent".to_string(),
                    path: "subagent.jsonl".to_string(),
                    kind: TailNoticeKind::Reread,
                    offset_before: 0,
                    offset_after: 0,
                    inode_before: Some(1),
                    inode_after: Some(1),
                    at: "2026-09-04T00:00:00Z".to_string(),
                },
            ),
            replaced_at(2),
        ];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        assert!(result.findings.iter().any(|f| f.code == "store_rewritten"));
    }

    /// One leg-1 record of `store`, as the tailer chains it.
    ///
    /// In the real pipeline these never reach `cx.events` — `chain::load_and_verify`
    /// splits them into `stores` — so the witness rule reads the whole envelope stream
    /// (`cx.chain`) and falls back to `cx.events` where there is no chain, which is what
    /// lets a unit test place one.
    fn record_at(seq: u64, store: &str) -> Envelope {
        env(
            seq,
            EventBody::Leg1Record {
                store: store.to_string(),
                payload: Value::String("{\"type\":\"user\"}".to_string()),
            },
        )
    }

    /// The lookback is bounded by what the daemon did in between.
    ///
    /// A `DaemonRestart` licenses a re-read from zero because the tailer had no offset to
    /// resume from. Once it has chained a record of that store it has one again, so a later
    /// truncation of the same store is not the restart's re-read — it is a second event the
    /// restart says nothing about. Without this bound one early restart would license every
    /// later truncation of every store for the rest of the run.
    #[test]
    fn a_record_of_the_store_between_witness_and_notice_voids_the_witness() {
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let trace = legacy_expected_reread_trace();
        let parsed = support::parsed();
        let events = vec![restart_at(1), record_at(2, "main"), replaced_at(3)];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "store_rewritten")
            .expect("the daemon read the store after restarting, so the restart is spent");
        assert!(
            f.message
                .contains("the participant's word and nothing else"),
            "{}",
            f.message
        );
        assert!(!result.findings.iter().any(|f| f.code == "store_reread"));
    }

    /// The bound is per store, like the notice it judges. Records of other stores between
    /// the restart and the notice say nothing about whether this store was read, so the
    /// witness still stands.
    #[test]
    fn records_of_other_stores_between_witness_and_notice_leave_it_standing() {
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let trace = legacy_expected_reread_trace();
        let parsed = support::parsed();
        let events = vec![
            restart_at(1),
            record_at(2, "subagent"),
            record_at(3, "sidechain"),
            replaced_at(4),
        ];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Flag);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "store_reread" && f.message.contains("expected resume re-read"))
        );
        assert!(!result.findings.iter().any(|f| f.code == "store_rewritten"));
    }

    /// Only the interval between the witness and the notice is looked at. A record of the
    /// store chained below the restart is the run before it, which the restart is
    /// precisely the end of — so a witness sitting immediately under the notice is
    /// honoured however much of that store was read earlier.
    #[test]
    fn a_witness_immediately_below_the_notice_is_honoured() {
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let trace = legacy_expected_reread_trace();
        let parsed = support::parsed();
        let events = vec![record_at(1, "main"), restart_at(2), replaced_at(3)];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Flag);
        assert!(result.findings.iter().any(|f| f.code == "store_reread"));
        assert!(!result.findings.iter().any(|f| f.code == "store_rewritten"));
    }

    // The chain-descriptor half of `declares_expected_reread` — the top-level
    // `trace.toml` flag, gated by the same witness — is exercised end to end over a real
    // trace directory in `tests/it/compliant.rs`
    // (`expected_reread_without_a_restart_does_not_downgrade_a_rewrite`). A unit test
    // would have to hand-build a `VerifiedChain` for `cx.chain` to be `Some`, which is a
    // fixture asserting nothing the integration test does not assert over real bytes.

    /// A bare `Reread` notice is a consequence, never judged on its own — the notice that
    /// caused it (a `Truncated` or `FileReplaced` immediately before it) is what carries
    /// the verdict.
    #[test]
    fn a_bare_reread_notice_is_never_judged_on_its_own() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let human = support::human();
        let events = vec![env(
            1,
            EventBody::TailNotice {
                store: "main".to_string(),
                path: "main.jsonl".to_string(),
                kind: TailNoticeKind::Reread,
                offset_before: 0,
                offset_after: 0,
                inode_before: Some(1),
                inode_after: Some(1),
                at: "2026-09-04T00:00:00Z".to_string(),
            },
        )];
        let cx = cx_with(&trace, &manifest, &parsed, &human, &events);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Pass);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "no_config_deviation")
        );
    }

    /// With no event stream at all, reconciliation is unverifiable, not
    /// the same fact as "reconciled and found nothing" — `no_event_stream` at `Flag`,
    /// never `no_config_deviation` at `Pass`.
    #[test]
    fn an_empty_event_stream_is_unverifiable_not_clean() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let human = support::human();
        let cx = cx_with(&trace, &manifest, &parsed, &human, &[]);
        let result = V9Config.run(&cx);
        assert_eq!(result.verdict, Severity::Flag);
        assert!(result.findings.iter().any(|f| f.code == "no_event_stream"));
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == "no_config_deviation")
        );
    }

    /// One launch's arming signals are a `Pass`, whatever number of them that launch
    /// emitted.
    ///
    /// Two arming posts from one shim instance and one arming post alone are both one
    /// launch, and the trace says so without being told a number: the shim mints its id at
    /// load. The shape is asserted on Codex's arming signal.
    #[test]
    fn arming_signals_from_one_shim_instance_are_one_launch() {
        let trace = support::trace("codex-cli");
        let manifest = support::manifest("codex-cli");
        for signals in [1, 2, 5] {
            let mut parsed = support::parsed();
            for _ in 0..signals {
                parsed.config_events.push(load(Some("shim-a")));
            }
            let human = support::human();
            let cx = cx_with(&trace, &manifest, &parsed, &human, &[]);
            let f = arming_of(&V9Config.run(&cx));
            assert_eq!(f.0, Severity::Pass, "{signals} signal(s): {}", f.1);
            assert!(f.1.contains("armed once"), "{}", f.1);
        }
    }

    /// A launch the trace cannot account for is a Flag, and one witnessed relaunch excuses
    /// exactly one extra launch.
    ///
    /// The middle case: a run with one honest resume and a re-arm on top of it is not
    /// clean. The resume is what the rules ask for; the re-arm is not.
    #[test]
    fn a_launch_the_trace_cannot_account_for_is_flagged() {
        let trace = support::trace("codex-cli");
        let manifest = support::manifest("codex-cli");
        let human = support::human();
        let exited = [env(
            1,
            EventBody::Health(goalcup_protocol::events::HealthEvent {
                state: HealthState::AgentExited,
                at: "2026-09-07T03:36:00Z".to_string(),
                detail: String::new(),
            }),
        )];
        // `launches` distinct shim instances, two signals each.
        let run = |launches: usize, events: &[Envelope]| {
            let mut parsed = support::parsed();
            for i in 0..launches {
                let id = format!("shim-{i}");
                parsed.config_events.push(load(Some(&id)));
                parsed.config_events.push(load(Some(&id)));
            }
            let cx = cx_with(&trace, &manifest, &parsed, &human, events);
            arming_of(&V9Config.run(&cx))
        };

        let (two_unwitnessed, said) = run(2, &[]);
        assert_eq!(
            two_unwitnessed,
            Severity::Flag,
            "two launches and nothing in the trace says the harness was relaunched: {said}"
        );
        assert!(said.contains("re-armed"), "{said}");

        let (two_witnessed, said) = run(2, &exited);
        assert_eq!(
            two_witnessed,
            Severity::Pass,
            "one launch, one witnessed relaunch — the resume the rules ask for: {said}"
        );

        let (three_witnessed, said) = run(3, &exited);
        assert_eq!(
            three_witnessed,
            Severity::Flag,
            "one honest resume does not excuse a re-arm beside it: {said}"
        );
        assert!(said.contains("3 launch(es)"), "{said}");
    }

    /// A connector whose shim mints no instance id reads one signal as one launch.
    #[test]
    fn without_a_shim_id_each_arming_signal_is_its_own_launch() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let arming = manifest.arming.signal.clone();
        let signal = || crate::parsers::ConfigFact {
            event: arming.clone(),
            at: None,
            detail: String::new(),
            shim: None,
        };
        let mut one = support::parsed();
        one.config_events.push(signal());
        let cx = cx_with(&trace, &manifest, &one, &human, &[]);
        assert_eq!(arming_of(&V9Config.run(&cx)).0, Severity::Pass);

        let mut two = support::parsed();
        two.config_events.push(signal());
        two.config_events.push(signal());
        let cx = cx_with(&trace, &manifest, &two, &human, &[]);
        let (severity, said) = arming_of(&V9Config.run(&cx));
        assert_eq!(severity, Severity::Flag, "{said}");
        assert!(said.contains("no shim instance id"), "{said}");
    }

    /// A launch whose posts carry no shim id is still a launch, even where another launch's
    /// posts carry one.
    ///
    /// Counting only distinct ids would read the two stamped signals as the whole trace and
    /// pass a run that armed itself twice more with the field dropped. The unstamped signals
    /// are counted as launches of their own instead, so the trace holds three.
    #[test]
    fn arming_signals_without_a_shim_id_still_count_where_others_carry_one() {
        let trace = support::trace("codex-cli");
        let manifest = support::manifest("codex-cli");
        let human = support::human();
        let mut parsed = support::parsed();
        parsed.config_events.push(load(Some("shim-a")));
        parsed.config_events.push(load(Some("shim-a")));
        parsed.config_events.push(load(None));
        parsed.config_events.push(load(None));
        let cx = cx_with(&trace, &manifest, &parsed, &human, &[]);
        let (severity, said) = arming_of(&V9Config.run(&cx));
        assert_eq!(
            severity,
            Severity::Flag,
            "two stamped signals from one instance plus two unstamped are three launches, \
             and nothing witnesses a relaunch: {said}"
        );
        assert!(said.contains("3 launch(es)"), "{said}");
        assert!(said.contains("carrying none"), "{said}");
    }

    /// One arming fact on Codex's `SessionStart` signal, optionally stamped with the shim
    /// instance that posted it.
    fn load(shim: Option<&str>) -> crate::parsers::ConfigFact {
        crate::parsers::ConfigFact {
            event: "SessionStart".to_string(),
            at: None,
            detail: String::new(),
            shim: shim.map(str::to_string),
        }
    }

    /// The `arming_signals` finding's severity and message, which every test above reads.
    fn arming_of(result: &CheckResult) -> (Severity, String) {
        result
            .findings
            .iter()
            .find(|f| f.code == "arming_signals")
            .map(|f| (f.severity, f.message.clone()))
            .expect("arming signals were recorded, so the check reports on them")
    }

    /// `ConfigFact.detail` — a raw hook body — is never interpolated into a finding; only
    /// its length is.
    #[test]
    fn config_event_detail_is_never_interpolated() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.config_events.push(crate::parsers::ConfigFact {
            event: "ConfigChange".to_string(),
            at: None,
            detail: "{\"hookHome\":\"/Users/real/.claude\",\"secret\":\"do-not-print-me\"}"
                .to_string(),
            shim: None,
        });
        let human = support::human();
        let cx = cx_with(&trace, &manifest, &parsed, &human, &[]);
        let result = V9Config.run(&cx);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "config_event")
            .expect("a non-arming config event was recorded");
        assert!(!f.message.contains("/Users/"));
        assert!(!f.message.contains("do-not-print-me"));
        assert!(f.message.contains("byte(s) of detail"));
    }
}
