use goalcup_protocol::manifest::{
    ConnectorManifest, CursorType, ForeignScanReader, ForeignScanRoot, InstallScope, KillStrategy,
    MatchRule, PATH_TOKENS, PartitionSpec, ScopingClause, Tier,
};
use goalcup_protocol::template::{TemplateVars, render};
use std::path::PathBuf;

fn connectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

fn load(id: &str) -> ConnectorManifest {
    let path = connectors_dir().join(id).join("manifest.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let m = ConnectorManifest::from_toml_str(&text).expect("manifest parses");
    m.validate().expect("manifest validates");
    m
}

/// Every file the daemon INSTALLS into the workspace is excluded from the seal walk.
///
/// The daemon renders `hook_install[]` into the workspace before the run and the seal walks
/// the workspace after it, so an installed file the exclusion list does not cover turns up
/// in `seal.json` with nothing in the trace naming it — and `unexplained_seal_file` is a
/// Flag on the tracker's own file, which is noise an organizer should never be handed.
///
/// Asserted through `seal::is_excluded`, the single definition the attributor and the seal
/// walk both use, rather than by string comparison: what matters is that the pattern
/// list actually covers the path, not that the two strings look alike.
#[test]
fn every_workspace_file_a_connector_installs_is_excluded_from_the_seal() {
    for id in ["claude-code-cli", "codex-cli"] {
        let m = load(id);
        for h in m
            .hook_install
            .iter()
            .filter(|h| h.scope == InstallScope::Workspace)
        {
            assert!(
                goalcup_protocol::seal::is_excluded(&h.path, &m.workspace_exclusions),
                "{id}: `{}` is installed into the workspace and `workspace_exclusions` does \
                 not cover it, so the seal would list a file the tracker wrote",
                h.path
            );
        }
    }
}

#[test]
fn both_retained_v1_connectors_load_and_validate() {
    for id in ["claude-code-cli", "codex-cli"] {
        let m = load(id);
        assert_eq!(m.id, id);
        assert_eq!(
            m.tier,
            Tier::Full,
            "{id} is measured `full` in DECISIONS D23"
        );
        assert!(!m.store.is_empty(), "{id} declares at least one store");
        assert!(!m.human_rule.byte_exact_field.is_empty());
        assert!(!m.human_rule.pair_key.is_empty());
    }
}

#[test]
fn claude_code_manifest_matches_spike_4() {
    let m = load("claude-code-cli");
    assert_eq!(m.tested_version_range, ">=2.1.259");
    assert_eq!(m.kill.strategy, KillStrategy::ProcessGroup);
    assert!(m.normalisation.trailing_newline);
    assert_eq!(m.human_rule.byte_exact_field, "prompt");
    assert_eq!(m.human_rule.leg2_event, "UserPromptSubmit");
    assert_eq!(m.human_rule.pair_key, "prompt_id");
    assert!(m.unsafe_cursors.is_empty());
    assert!(
        m.human_rule.positive.contains(&MatchRule::FieldEquals {
            field: "origin.kind".into(),
            value: "human".into()
        }),
        "spike 4 A2: origin.kind=human is the positive test"
    );
    assert_eq!(
        m.store_named("main").unwrap().cursor.kind,
        CursorType::InodeOffset
    );
    // `aux_calls[]` is asserted absent in
    // `aux_calls_is_gone_from_the_schema_and_from_every_manifest`.
}

#[test]
fn codex_manifest_matches_spike_2() {
    let m = load("codex-cli");
    assert_eq!(m.tested_version_range, "=0.153.0");
    assert_eq!(m.arming.signal, "SessionStart");
    assert_eq!(
        m.kill.strategy,
        KillStrategy::NativePid,
        "D22: the npm wrapper is not the session"
    );
    assert_eq!(
        m.join_key.as_deref(),
        Some("turn_id+order"),
        "D2/V8: the exec-family id spaces are disjoint, so the join is by turn and order"
    );
    assert!(
        m.lock_paths
            .iter()
            .any(|p| p.contains("thread-writer-locks"))
    );
    assert_eq!(
        m.store_named("rollout").unwrap().cursor.kind,
        CursorType::Ordinal
    );
    assert_eq!(
        m.store_named("rollout").unwrap().partition_key.as_deref(),
        Some("session_meta.cwd")
    );
    assert!(
        m.human_rule
            .scoping
            .contains(&ScopingClause::FirstSessionMetaIsTracked)
    );
    assert!(
        m.human_rule
            .scoping
            .contains(&ScopingClause::NotSubagentThreadSource)
    );
}

#[test]
fn validate_rejects_an_empty_parser_family() {
    let mut m = load("claude-code-cli");
    m.family.clear();
    let error = m
        .validate()
        .expect_err("a family selects the parser and cannot be empty");
    assert_eq!(
        error.to_string(),
        "field family: must not be empty",
        "the schema remains extensible while refusing a surface with no parser family"
    );
}

// ---------------------------------------------------------------- the daemon's half

#[test]
fn claude_installs_only_its_launch_settings() {
    let m = load("claude-code-cli");
    let paths: Vec<&str> = m.hook_install.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["settings-launch.json"],
        "the workspace settings shim went with the hook leg"
    );
    assert_eq!(m.hook_install[0].scope, InstallScope::ArtifactRoot);
    assert_eq!(m.hook_install[0].template, "settings-launch.json");
    let main = m.store_named("main").unwrap();
    assert_eq!(main.cursor.kind, CursorType::InodeOffset);
    assert!(
        main.partition.is_none(),
        "spike 4 A9: process-scoped pins leaked nothing"
    );
}

#[test]
fn codex_declares_a_file_first_record_partition_and_only_its_harness_config() {
    let m = load("codex-cli");
    let paths: Vec<&str> = m.hook_install.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["config.toml"],
        "the hooks.json and hook-post.sh shims went with the hook leg"
    );
    assert_eq!(m.hook_install[0].scope, InstallScope::HarnessHome);
    assert!(m.hook_install[0].harness_owned);

    let rollout = m.store_named("rollout").unwrap();
    assert_eq!(rollout.cursor.kind, CursorType::Ordinal);
    assert_eq!(rollout.cursor.column.as_deref(), Some("payload.ordinal"));
    match rollout.partition.as_ref().expect("rollout is partitioned") {
        PartitionSpec::FileFirstRecord {
            select_path,
            select_value,
            value_path,
        } => {
            assert_eq!(select_path, "type");
            assert_eq!(select_value, "session_meta");
            assert_eq!(value_path, "payload.cwd");
        }
        other => panic!("codex partitions by the file's first session_meta, got {other:?}"),
    }
    assert!(matches!(
        m.foreign_scan
            .iter()
            .find(|scan| scan.harness == "codex-cli")
            .expect("the Codex rollout scan remains shipped")
            .reader,
        ForeignScanReader::FileFirstRecord {}
    ));
    let claude = load("claude-code-cli");
    assert!(
        claude
            .foreign_scan
            .iter()
            .filter(|scan| matches!(scan.reader, ForeignScanReader::FileFirstIdentity {}))
            .count()
            == 2,
        "the two measured Claude own-source rows use the bounded prefix reader"
    );
}

// ---------------------------------------------------------------- the CLI's half

#[test]
fn every_template_var_a_manifest_uses_is_one_the_tracker_supplies() {
    // The eight variables the launch argv fills.
    const SUPPLIED: [&str; 8] = [
        "workspace",
        "artifact_root",
        "harness_home",
        "session_id",
        "entry_id",
        "run_id",
        "server_port",
        "status_line_path",
    ];
    for id in ["claude-code-cli", "codex-cli"] {
        for used in load(id).template_vars_used() {
            assert!(
                SUPPLIED.contains(&used.as_str()),
                "{id} uses unknown var {{{used}}}"
            );
        }
    }
}

#[test]
fn validate_rejects_a_native_pid_strategy_with_no_binary_named() {
    let mut m = load("codex-cli");
    m.kill.native_binary = None;
    let err = m
        .validate()
        .expect_err("native_pid needs a binary to look for");
    assert!(format!("{err}").contains("kill.native_binary"), "got {err}");
}

/// Claude Code declares what a fresh per-run home has to be seeded with, and
/// every rule about what that section may name is refused rather than trusted.
///
/// The `from` clauses are the load-bearing ones: `home_seed` is the only place a connector
/// names a file under the participant's REAL home that the CLI then opens, so a token other
/// than `{home}`, a root other than `{home}/`, and a `..` escape are three separate
/// refusals. `to` is not a template at all — the destination root is the daemon's per-run
/// harness home and never the connector's to choose.
#[test]
fn claude_declares_a_home_seed_and_validate_polices_what_it_may_name() {
    let m = load("claude-code-cli");
    let seed = m.home_seed.clone().expect("claude-code-cli seeds its home");
    assert_eq!(
        seed.from,
        ["{home}/.claude.json", "{home}/.claude/.claude.json"],
        "the default location first, then where an exported CLAUDE_CONFIG_DIR puts it"
    );
    assert_eq!(seed.to, ".claude.json", "relative to the per-run home");
    for required in ["hasCompletedOnboarding", "oauthAccount"] {
        assert!(
            seed.required.contains(&required.to_string()),
            "without `{required}` the run meets an onboarding screen"
        );
        assert!(
            seed.keys.contains(&required.to_string()),
            "a required key is one of the copied keys"
        );
    }
    assert!(
        seed.keys.contains(&"theme".to_string()),
        "the theme picker is the first screen a fresh home shows"
    );
    // The two prompts that appear AFTER the theme picker and the login wall
    // have been cleared, and the keys carrying the participant's answers to them. Named one
    // at a time rather than counted, because a key quietly dropped from this list is a modal
    // that comes back on the next fresh entry — and the Chrome one ate a typed message
    // rather than merely delaying the run.
    for (key, prompt) in [
        (
            "fullscreenUpsellSeenCount",
            "Try the new fullscreen renderer?",
        ),
        (
            "hasCompletedClaudeInChromeOnboarding",
            "Claude in Chrome extension detected",
        ),
        (
            "claudeInChromeDefaultEnabled",
            "Claude in Chrome extension detected",
        ),
        (
            "cachedChromeExtensionInstalled",
            "Claude in Chrome extension detected",
        ),
    ] {
        assert!(
            seed.keys.contains(&key.to_string()),
            "without `{key}` a fresh per-run home meets \"{prompt}\""
        );
        assert!(
            !seed.required.contains(&key.to_string()),
            "`{key}` is copied where it is there and never demanded: a participant who has \
             not been asked that question has no such key, and that is not a refused start"
        );
    }
    // The second first-run screen, which no copied key can answer: it is asked per project,
    // about a workspace that did not exist when the participant's file was written.
    assert_eq!(seed.trust_field.as_deref(), Some("projects"));
    assert_eq!(seed.trust_keys, ["hasTrustDialogAccepted"]);

    let bad = |change: &dyn Fn(&mut goalcup_protocol::manifest::HomeSeed), want: &str| {
        let mut m = load("claude-code-cli");
        let mut seed = m.home_seed.clone().unwrap();
        change(&mut seed);
        m.home_seed = Some(seed);
        let err = m.validate().expect_err("validate refuses it");
        assert!(format!("{err}").contains(want), "got {err}, wanted {want}");
    };
    bad(&|s| s.keys.clear(), "home_seed.keys");
    bad(&|s| s.keys = vec![String::new()], "home_seed.keys");
    bad(&|s| s.from.clear(), "home_seed.from");
    bad(
        &|s| s.from = vec!["{workspace}/.claude.json".into()],
        "home_seed.from",
    );
    bad(&|s| s.from = vec!["/etc/passwd".into()], "home_seed.from");
    bad(
        &|s| s.from = vec!["{home}/../other/.x".into()],
        "home_seed.from",
    );
    bad(&|s| s.to = "/etc/passwd".into(), "home_seed.to");
    bad(&|s| s.to = "../.claude.json".into(), "home_seed.to");
    bad(&|s| s.to = "{home}/.claude.json".into(), "home_seed.to");
    bad(
        &|s| s.required = vec!["neverCopied".into()],
        "home_seed.required",
    );
    // The two halves of the trust clause are useless apart, in both directions.
    bad(&|s| s.trust_field = None, "home_seed.trust_field");
    bad(&|s| s.trust_keys.clear(), "home_seed.trust_field");
    bad(
        &|s| s.trust_field = Some(String::new()),
        "home_seed.trust_field",
    );
    bad(
        &|s| s.trust_keys = vec![String::new()],
        "home_seed.trust_keys",
    );
}

/// The section is additive: a manifest that declares none still loads, and seeds nothing.
///
/// No shipped connector is in that case, so the assertion is made on a copy with the
/// section cleared. `codex-cli` declares the WHOLE-FILE clause and no keys, which the
/// assertions below are about: `CODEX_HOME` is the per-run home, so a fresh entry has no
/// `auth.json` and meets the sign-in screen, and the section copies that one file.
#[test]
fn a_manifest_that_declares_no_home_seed_still_loads() {
    let mut unseeded = load("codex-cli");
    unseeded.home_seed = None;
    unseeded
        .validate()
        .expect("a manifest with no home_seed is not an error");
    let codex = load("codex-cli");
    let seed = codex
        .home_seed
        .as_ref()
        .expect("codex-cli seeds the per-run CODEX_HOME");
    assert_eq!(seed.from_dir.as_deref(), Some("{home}/.codex"));
    assert_eq!(seed.files, vec!["auth.json".to_string()]);
    assert!(
        seed.keys.is_empty() && seed.from.is_empty() && seed.to.is_empty(),
        "it copies a whole file and merges no keys"
    );
    assert!(
        seed.required.is_empty(),
        "a machine with no credential is a line on the screen, not a refused start"
    );
    codex.validate().expect("the whole-file clause validates");
}

#[test]
fn validate_rejects_a_launch_env_that_sets_a_model_base_url() {
    let mut m = load("claude-code-cli");
    m.launch
        .env
        .insert("ANTHROPIC_BASE_URL".into(), "http://127.0.0.1:9".into());
    let err = m.validate().expect_err("D1: no proxy, ever");
    assert!(format!("{err}").contains("launch.env"), "got {err}");
}

#[test]
fn render_substitutes_and_refuses_an_unknown_placeholder() {
    let mut v = TemplateVars::new();
    v.set("entry_id", "entry-0001");
    assert_eq!(render("a {entry_id} b", &v).unwrap(), "a entry-0001 b");
    assert_eq!(render("{{literal}}", &v).unwrap(), "{literal}");
    let err = render("{not_a_var}", &v).expect_err("unknown placeholders must fail");
    assert!(format!("{err}").contains("not_a_var"), "{err}");
}

// ---------------------------------------------------------------- path templates

#[test]
fn every_path_template_token_is_one_of_the_nine_and_no_manifest_names_an_env_var() {
    assert_eq!(PATH_TOKENS.len(), 9);
    for id in ["claude-code-cli", "codex-cli"] {
        let m = load(id);
        let used = m.path_tokens_used();
        assert!(
            !used.is_empty(),
            "{id} declares stores, so it uses path tokens"
        );
        for token in &used {
            assert!(
                PATH_TOKENS.contains(&token.as_str()),
                "{id} uses path token {{{token}}}, which is not in PATH_TOKENS"
            );
        }
        // No second template language appears in any path-bearing field.
        let mut paths: Vec<String> = m.lock_paths.clone();
        paths.extend(m.config_allowlist.clone());
        paths.extend(m.foreign_scan.iter().map(|f| f.path_glob.clone()));
        for s in &m.store {
            paths.push(s.path_template.clone());
            paths.extend(s.subagent_path_template.clone());
            paths.extend(s.spill_path_template.clone());
        }
        for p in &paths {
            assert!(
                !p.contains('$'),
                "{id}: `{p}` still names an environment variable"
            );
            assert!(
                !p.contains('<'),
                "{id}: `{p}` still uses the old <token> form"
            );
            assert!(!p.starts_with('~'), "{id}: `{p}` still uses a tilde");
        }
    }
}

#[test]
fn validate_rejects_a_path_template_token_the_tracker_does_not_fill() {
    let mut m = load("codex-cli");
    m.store[0].path_template = "{codex_home}/sessions/{yyyy}/rollout-*.jsonl".into();
    let err = m
        .validate()
        .expect_err("a path token outside PATH_TOKENS must fail");
    assert!(format!("{err}").contains("codex_home"), "got {err}");
    assert!(format!("{err}").contains("PATH_TOKENS"), "got {err}");
}

// ---------------------------------------------------------------- tools, scans, heartbeat, secrets

#[test]
fn tool_classes_name_the_write_tools_and_the_shell_subset() {
    // D11: attribution restricts its candidate set to these names, so a `Read`,
    // a `grep` or an `ls` can never explain a sealed file.
    let claude = load("claude-code-cli");
    assert_eq!(
        claude.tool_classes.write,
        vec!["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"]
    );
    assert_eq!(claude.tool_classes.shell, vec!["Bash"]);
    assert!(!claude.tool_classes.write.iter().any(|t| t == "Read"));

    let codex = load("codex-cli");
    // The parser emits the raw tool names Codex's `exec`-as-JS-macro convention uses, never
    // the classic `shell`/`apply_patch` function-call names.
    assert_eq!(codex.tool_classes.write, vec!["exec", "apply_patch"]);
    assert_eq!(codex.tool_classes.shell, vec!["exec"]);

    for id in ["claude-code-cli", "codex-cli"] {
        let m = load(id);
        for s in &m.tool_classes.shell {
            assert!(
                m.tool_classes.write.contains(s),
                "{id}: {s} must also be a write tool"
            );
        }
    }
}

#[test]
fn every_connector_scans_for_a_foreign_harness_and_declares_a_heartbeat() {
    for id in ["claude-code-cli", "codex-cli"] {
        let m = load(id);
        assert!(
            !m.foreign_scan.is_empty(),
            "{id}: R8 sentence 2 needs somewhere to look"
        );
        // The scan looks at an UNPINNED installation under the participant's real home.
        assert!(
            m.foreign_scan
                .iter()
                .any(|f| f.path_glob.contains("{home}")),
            "{id}: at least one scan must cover the participant's real home"
        );
        for f in &m.foreign_scan {
            assert!(
                !f.directory_field.is_empty(),
                "{id}/{}: needs a directory field",
                f.harness
            );
            assert!(
                !f.session_id_field.is_empty(),
                "{id}/{}: needs an id field",
                f.harness
            );
        }
        assert_eq!(
            m.heartbeat_seconds,
            Some(60),
            "{id}: B-5's interval, stated per connector"
        );
    }
    // Cross-harness: Claude looks for a stray Codex thread, and Codex for a stray Claude
    // slug — a second harness in a bound workspace is D10's case, not only a second self.
    assert!(
        load("claude-code-cli")
            .foreign_scan
            .iter()
            .any(|f| f.harness == "codex-cli")
    );
    assert!(
        load("codex-cli")
            .foreign_scan
            .iter()
            .any(|f| f.harness == "claude-code-cli")
    );
}

#[test]
fn connectors_declare_the_billing_patterns_the_shared_set_misses() {
    // `plan_type`, credit balance and quota were measured 15 times in a ten-minute
    // Codex session. D26 strips them before publication; this strips them at capture.
    let codex = load("codex-cli");
    let kinds: Vec<&str> = codex
        .redaction
        .extra_patterns
        .iter()
        .map(|p| p.kind.as_str())
        .collect();
    assert!(kinds.contains(&"codex_rate_limits"), "have {kinds:?}");
    assert!(kinds.contains(&"codex_plan_type"), "have {kinds:?}");

    let claude = load("claude-code-cli");
    assert!(
        claude
            .redaction
            .extra_patterns
            .iter()
            .any(|p| p.kind == "anthropic_billing_header"),
        "the x-anthropic-billing-* block rides in every raw body"
    );
    for id in ["claude-code-cli", "codex-cli"] {
        for p in &load(id).redaction.extra_patterns {
            assert!(
                !p.regex.is_empty(),
                "{id}/{}: a pattern needs a regex",
                p.kind
            );
        }
    }
}

// ---------------------------------------------------------------- the tightened schema

fn shipped(id: &str) -> String {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
        .join("manifest.toml");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn every_shipped_manifest_still_loads_and_validates_under_the_tightened_schema() {
    for id in ["claude-code-cli", "codex-cli"] {
        let m =
            ConnectorManifest::from_toml_str(&shipped(id)).unwrap_or_else(|e| panic!("{id}: {e}"));
        m.validate().unwrap_or_else(|e| panic!("{id}: {e}"));
        // Every store says how it is partitioned, or says why it needs no partition.
        for s in &m.store {
            assert!(
                s.partition_key.is_some() ^ s.unpartitioned_reason.is_some(),
                "{id}/{}: exactly one of partition_key and unpartitioned_reason",
                s.name
            );
        }
    }
}

#[test]
fn an_unknown_key_is_refused_rather_than_silently_ignored() {
    // PREPENDED, not appended: a bare key after the last table belongs to that table, so
    // appending would exercise whichever nested struct happens to come last in the file
    // and would say nothing about `ConnectorManifest` itself.
    let text = format!("human_rule_id = \"typo\"\n{}", shipped("claude-code-cli"));
    let err = ConnectorManifest::from_toml_str(&text).unwrap_err();
    assert!(
        err.to_string().contains("human_rule_id"),
        "deny_unknown_fields must name the key: {err}"
    );

    // And the nested structs are covered too, each named explicitly.
    let nested =
        shipped("claude-code-cli").replace("[[store]]", "[[store]]\nrotates_hourly = true");
    let err = ConnectorManifest::from_toml_str(&nested).unwrap_err();
    assert!(err.to_string().contains("rotates_hourly"), "{err}");
}

/// The hook-leg sections and the human-rule exclusion catalogue are accepted and ignored, so
/// a recorded V1 manifest still parses and stays bound to its receipt's `manifest_sha256`.
/// Only those named keys: an unknown key beside them is still refused.
#[test]
fn the_removed_hook_leg_sections_are_accepted_and_ignored_and_nothing_else_is() {
    let legacy = format!(
        "{}\n[start_signal]\nrequired = true\n\n[blocking]\ncapability = \"blocking\"\n\
         response_shape = \"decision_block\"\n\n[receiver]\ntoken_field = \"X-GoalCup-Token\"\n\n\
         [leg2_partition]\npath = \"payload.cwd\"\n\n[[hook_silence_policy]]\nevent = \"SessionStart\"\n\
         policy = \"conditional\"\nhot_disable = \"restart\"\nhot_disable_seconds = 1.5\n",
        shipped("codex-cli")
            .replace(
                "timeout_seconds = 300\n",
                "timeout_seconds = 300\nrecipe = \"an old human step\"\n",
            )
            .replace("positive = []", "reportable_exclusions = []\npositive = []",)
            + "\n[[human_rule.exclusions]]\nid = \"environment-context\"\n\
               rule = { match = \"text_prefix\", prefix = \"<environment_context>\" }\n"
    );
    let m = ConnectorManifest::from_toml_str(&legacy).expect("a recorded V1 manifest parses");
    m.validate().expect("and validates");
    let json = serde_json::to_string(&m).unwrap();
    for key in [
        "start_signal",
        "blocking",
        "receiver",
        "leg2_partition",
        "hook_silence_policy",
        "recipe",
    ] {
        assert!(
            !json.contains(key),
            "`{key}` is ignored, never serialised: {json}"
        );
    }
    // The exclusion catalogue in `[human_rule]`. (Quoted, since
    // `workspace_exclusions` is a live key that serialises.)
    for key in ["\"exclusions\"", "\"reportable_exclusions\""] {
        assert!(
            !json.contains(key),
            "{key} is ignored, never serialised: {json}"
        );
    }

    // An unknown key is still refused, at the top level and in `[arming]`.
    let top = format!("hook_silence = true\n{}", shipped("codex-cli"));
    let err = ConnectorManifest::from_toml_str(&top).unwrap_err();
    assert!(err.to_string().contains("hook_silence"), "{err}");
    let nested = shipped("codex-cli").replace(
        "timeout_seconds = 300\n",
        "timeout_seconds = 300\nsignal_required = true\n",
    );
    let err = ConnectorManifest::from_toml_str(&nested).unwrap_err();
    assert!(err.to_string().contains("signal_required"), "{err}");
    let in_human_rule =
        shipped("codex-cli").replace("positive = []", "exclusion = []\npositive = []");
    let err = ConnectorManifest::from_toml_str(&in_human_rule).unwrap_err();
    assert!(err.to_string().contains("exclusion"), "{err}");
}

/// No shipped manifest carries a hook-leg section or the exclusion catalogue; only recorded
/// ones do.
#[test]
fn no_shipped_manifest_carries_a_removed_hook_leg_section() {
    for id in ["claude-code-cli", "codex-cli"] {
        let text = shipped(id);
        for header in [
            "[start_signal]",
            "[blocking]",
            "[receiver]",
            "[leg2_partition]",
            "[[hook_silence_policy]]",
            "[[human_rule.exclusions]]",
        ] {
            assert!(!text.contains(header), "{id} still declares {header}");
        }
        assert!(
            !text.contains("\nrecipe ="),
            "{id} still declares an arming recipe"
        );
        assert!(
            !text.contains("reportable_exclusions"),
            "{id} still declares reportable_exclusions"
        );
    }
}

#[test]
fn a_stringly_mode_that_is_not_one_of_the_three_is_refused_at_load() {
    let text = shipped("claude-code-cli").replace("mode = \"jsonl\"", "mode = \"jsonlines\"");
    assert!(ConnectorManifest::from_toml_str(&text).is_err());
}

#[test]
fn validate_refuses_duplicate_store_names_and_hook_paths() {
    let base = ConnectorManifest::from_toml_str(&shipped("claude-code-cli")).unwrap();

    let mut dup_store = base.clone();
    let first = dup_store.store[0].clone();
    dup_store.store.push(first);
    let e = dup_store.validate().unwrap_err().to_string();
    assert!(e.contains("store") && e.contains("unique"), "{e}");

    let mut dup_hook = base.clone();
    if let Some(h) = dup_hook.hook_install.first().cloned() {
        dup_hook.hook_install.push(h);
        assert!(
            dup_hook
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unique")
        );
    }
}

/// `hook_install[].path` is rendered under `scope`'s root
/// (`InstallScope::Workspace | ArtifactRoot | HarnessHome`) with no filesystem check of
/// its own — the V1 renderer (removed by OS-31) joined it onto that root and wrote there. An absolute
/// path or a `..` segment escapes the scope root the same way `hook_install.template`
/// escaping `connectors/<id>/files/` would (that field is already refused above it), so
/// the same refusal belongs on `path`.
#[test]
fn validate_refuses_an_absolute_or_dot_dot_hook_install_path() {
    let base = ConnectorManifest::from_toml_str(&shipped("claude-code-cli")).unwrap();

    let mut absolute = base.clone();
    absolute.hook_install[0].path = "/etc/passwd".to_string();
    let e = absolute.validate().unwrap_err().to_string();
    assert!(e.contains("hook_install.path"), "{e}");

    let mut traversal = base.clone();
    traversal.hook_install[0].path = "../../etc/passwd".to_string();
    let e = traversal.validate().unwrap_err().to_string();
    assert!(e.contains("hook_install.path"), "{e}");

    // A path that merely contains the two characters as part of a real name is fine —
    // the refusal is for a `..` path *segment*, not the substring.
    let mut fine = base.clone();
    fine.hook_install[0].path = "settings..json".to_string();
    assert!(fine.validate().is_ok(), "{:?}", fine.validate());
}

/// A file on both `config_allowlist[]` and `hook_install[]` is refused by `validate()`. The two
/// lists mean opposite things — `config_allowlist[]` is never restored, `hook_install[]`
/// always is — so a manifest that puts the same raw template in both is asking the
/// daemon to do two contradictory things to one file, whichever scope either half meant.
#[test]
fn validate_refuses_a_hook_install_path_that_is_also_on_the_config_allowlist() {
    let mut m = ConnectorManifest::from_toml_str(&shipped("claude-code-cli")).unwrap();
    let path = m.hook_install[0].path.clone();
    m.config_allowlist.push(path.clone());
    let e = m.validate().unwrap_err().to_string();
    assert!(e.contains("config_allowlist") && e.contains(&path), "{e}");
}

#[test]
fn validate_enforces_the_cursor_column_each_type_needs() {
    use goalcup_protocol::manifest::{Cursor, CursorType};
    let mut m = ConnectorManifest::from_toml_str(&shipped("codex-cli")).unwrap();
    m.store[0].cursor = Cursor {
        kind: CursorType::Ordinal,
        column: None,
        group_column: None,
    };
    assert!(
        m.validate()
            .unwrap_err()
            .to_string()
            .contains("cursor.column")
    );

    m.store[0].cursor = Cursor {
        kind: CursorType::SessionSeq,
        column: Some("seq".to_string()),
        group_column: None,
    };
    assert!(
        m.validate()
            .unwrap_err()
            .to_string()
            .contains("group_column")
    );
}

#[test]
fn validate_refuses_a_store_that_says_nothing_about_partitioning() {
    let mut m = ConnectorManifest::from_toml_str(&shipped("claude-code-cli")).unwrap();
    m.store[0].partition_key = None;
    m.store[0].unpartitioned_reason = None;
    let e = m.validate().unwrap_err().to_string();
    assert!(
        e.contains("partition_key") && e.contains("unpartitioned_reason"),
        "the V10 flag became a manifest refusal: {e}"
    );
}

#[test]
fn aux_calls_is_gone_from_the_schema_and_from_every_manifest() {
    for id in ["claude-code-cli", "codex-cli"] {
        assert!(
            !shipped(id).contains("aux_calls"),
            "{id} still declares aux_calls, which schema v0 no longer carries"
        );
    }
    let text = format!(
        "{}\n[[aux_calls]]\nid = \"x\"\ndescription = \"y\"\n",
        shipped("codex-cli")
    );
    assert!(ConnectorManifest::from_toml_str(&text).is_err());
}

/// D10's detector on Claude Code: a second `claude` under the SAME `CLAUDE_CONFIG_DIR`
/// leaves a second project slug in `<config-dir>/projects/`, so the scan enumerates slugs
/// and not just the tracked one. The real home stays covered too, as the unpinned
/// installation.
#[test]
fn claude_scans_the_second_slug_under_its_own_config_dir() {
    let m = ConnectorManifest::from_toml_str(&shipped("claude-code-cli")).unwrap();
    let globs: Vec<&str> = m
        .foreign_scan
        .iter()
        .map(|f| f.path_glob.as_str())
        .collect();
    assert!(
        globs.contains(&"{config_dir}/projects/*/*.jsonl"),
        "{globs:?}"
    );
    assert!(
        globs.contains(&"{home}/.claude/projects/*/*.jsonl"),
        "the real home stays covered"
    );
    assert_eq!(m.foreign_scan.len(), 3);
    assert!(m.foreign_scan.iter().any(|scan| {
        scan.path_glob == "{config_dir}/projects/*/*.jsonl"
            && scan.root == ForeignScanRoot::OwnConfigDir
    }));
    m.validate().unwrap();
}

#[test]
fn every_shipped_claude_jsonl_foreign_scan_reads_first_joint_identity() {
    let readers: Vec<(&str, String, ForeignScanReader)> = ["claude-code-cli", "codex-cli"]
        .into_iter()
        .flat_map(|id| {
            let manifest = load(id);
            manifest
                .foreign_scan
                .into_iter()
                .filter(|scan| {
                    scan.harness == "claude-code-cli" && scan.path_glob.ends_with(".jsonl")
                })
                .map(move |scan| (id, scan.path_glob, scan.reader))
        })
        .collect();

    assert_eq!(readers.len(), 3, "the shipped Claude JSONL detector set");
    for (connector, path_glob, reader) in readers {
        assert_eq!(
            reader,
            ForeignScanReader::FileFirstIdentity {},
            "{connector}: {path_glob} can begin with bookkeeping before its joint identity"
        );
    }
}

#[test]
fn foreign_scan_defaults_preserve_line_home_declarations_and_sql_identifiers_are_closed() {
    let codex = load("codex-cli");
    let codex_home = codex
        .foreign_scan
        .iter()
        .find(|scan| scan.harness == "codex-cli")
        .expect("the Codex home declaration remains shipped");
    assert_eq!(codex_home.root, ForeignScanRoot::Home);
    assert_eq!(
        codex_home.reader,
        ForeignScanReader::FileFirstRecord {},
        "the Codex rollout still begins with its complete session metadata"
    );

    let mut sqlite = load("codex-cli");
    sqlite.foreign_scan[0].reader = ForeignScanReader::SqliteSession {
        table: "session".to_string(),
        session_id_column: "id".to_string(),
        directory_column: "directory".to_string(),
    };
    sqlite
        .validate()
        .expect("a closed SQL identifier is accepted");

    for bad in ["", "9session", "session.id", "session-id", "\"session\""] {
        let mut manifest = sqlite.clone();
        let ForeignScanReader::SqliteSession { table, .. } = &mut manifest.foreign_scan[0].reader
        else {
            unreachable!("the declaration was set to sqlite above")
        };
        *table = bad.to_string();
        let err = manifest.validate().expect_err("the identifier is refused");
        assert!(format!("{err}").contains("foreign_scan.reader"), "{err}");
    }

    let mut wrong_root = load("claude-code-cli");
    wrong_root.foreign_scan[1].root = ForeignScanRoot::Home;
    wrong_root
        .validate()
        .expect("the compatibility default remains readable by historical verifiers");
}

#[test]
fn foreign_scan_reader_rejects_unused_nested_keys_for_both_variants() {
    let line = shipped("codex-cli").replacen(
        "session_id_field = \"payload.session_id\"",
        "session_id_field = \"payload.session_id\"\nreader = { kind = \"file_first_record\", table_name = \"typo\" }",
        1,
    );
    let line_error = ConnectorManifest::from_toml_str(&line)
        .expect_err("a file reader's unused nested key is refused");
    assert!(
        format!("{line_error}").contains("table_name"),
        "{line_error}"
    );

    let identity = shipped("claude-code-cli").replacen(
        "reader = { kind = \"file_first_identity\" }",
        "reader = { kind = \"file_first_identity\", table_name = \"typo\" }",
        1,
    );
    let identity_error = ConnectorManifest::from_toml_str(&identity)
        .expect_err("the identity-prefix reader's unused nested key is refused");
    assert!(
        format!("{identity_error}").contains("table_name"),
        "{identity_error}"
    );

    let sqlite = shipped("codex-cli").replacen(
        "session_id_field = \"payload.session_id\"",
        "session_id_field = \"payload.session_id\"\nreader = { kind = \"sqlite_session\", table = \"session\", session_id_column = \"id\", directory_column = \"directory\", table_name = \"typo\" }",
        1,
    );
    let sqlite_error = ConnectorManifest::from_toml_str(&sqlite)
        .expect_err("a SQLite reader's unused nested key is refused");
    assert!(
        format!("{sqlite_error}").contains("table_name"),
        "{sqlite_error}"
    );
}

#[test]
fn connector_directory_hash_returns_the_manifest_bytes_from_its_own_stream() {
    let dir = tempfile::tempdir().expect("a temporary connector directory");
    let first = b"id = 'first'\n";
    let second = b"id = 'second'\n";
    std::fs::write(dir.path().join("manifest.toml"), first).unwrap();

    let (first_digest, authority) =
        goalcup_protocol::manifest::connector_dir_sha256_with_manifest(dir.path()).unwrap();
    std::fs::write(dir.path().join("manifest.toml"), second).unwrap();

    assert_eq!(authority, first);
    assert_ne!(
        first_digest,
        goalcup_protocol::connector_dir::connector_dir_sha256(dir.path()).unwrap(),
        "a later pathname read is distinguishable from the bytes in the accepted stream"
    );
}

/// An exclusion pattern that cannot match is refused at load, because the failure
/// is otherwise completely silent and lands on an honest run.
///
/// `seal::is_excluded` is path-anchored, so `.opencode/node_modules/` excludes nothing —
/// and the seal then lists every harness-written file under it, each one failing
/// attribution. The same argument as the `PATH_TOKENS` check one field over: a typo must
/// be an error at load, not a non-matching pattern at seal time.
#[test]
fn validate_rejects_a_workspace_exclusion_that_can_never_match() {
    for pattern in ["", ".codex/node_modules/", "**/"] {
        let mut bad = load("codex-cli");
        bad.workspace_exclusions = vec![pattern.to_string()];
        let Err(err) = bad.validate() else {
            panic!("`{pattern}` matches nothing and must be refused")
        };
        assert!(
            format!("{err}").contains("workspace_exclusions"),
            "the error names the field: {err}"
        );
    }

    // The spellings that DO match are untouched, including the any-depth opt-in.
    let mut good = load("codex-cli");
    good.workspace_exclusions = vec![
        ".codex/node_modules".to_string(),
        "**/.git".to_string(),
        "target".to_string(),
    ];
    good.validate().expect("an anchored pattern is fine");

    // And every shipped connector already writes them the matching way.
    for id in ["claude-code-cli", "codex-cli"] {
        for p in &load(id).workspace_exclusions {
            assert!(
                !p.is_empty() && !p.ends_with('/') && p != "**/",
                "{id}: `{p}` matches nothing"
            );
        }
    }
}

// ---------------------------------------------------------------- recorded V1 compatibility

#[test]
fn recorded_capture_keys_accept_arbitrary_values_and_never_serialize() {
    let mut recorded: toml::Table = shipped("codex-cli").parse().unwrap();
    recorded.insert(
        "capture_only".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "arbitrary".into(),
            toml::Value::Array(vec![1.into(), true.into(), "old".into()]),
        )])),
    );
    recorded.insert(
        "capture".into(),
        toml::Value::Table(toml::Table::from_iter([
            ("old".into(), "shape".into()),
            (
                "nested".into(),
                toml::Value::Table(toml::Table::from_iter([("any".into(), true.into())])),
            ),
        ])),
    );
    let recorded = toml::to_string(&recorded).unwrap();
    let manifest = ConnectorManifest::from_toml_str(&recorded)
        .expect("recorded capture shapes are consumed without interpretation");
    manifest
        .validate()
        .expect("ignored values do not affect V1 validation");
    let json = serde_json::to_string(&manifest).unwrap();
    assert!(!json.contains("capture_only"), "{json}");
    assert!(!json.contains("\"capture\""), "{json}");
}
