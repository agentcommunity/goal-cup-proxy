//! Lessons from the old code, host names (PRD-17 section 2.7, rows N1 to N6). Pure functions and a
//! hint file in a temp folder: no network, no sleep. Outcomes only, never message wording.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::SystemTime;

use goalcup::connector::{self, LaunchPlan};
use goalcup::connector::{Decision, Env, Source, hosts};

const CLAUDE: &str = "claude-code-cli";

/// The plan for the claude connector in a new folder, with only `vars` set and no config file.
fn plan_with(vars: &[(&str, &str)]) -> LaunchPlan {
    let dir = tempfile::tempdir().unwrap();
    let mut all = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
    all.extend(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let env = Env {
        vars: all,
        workspace: dir.path().join("workspace"),
        resume: false,
        run_start: SystemTime::now(),
        goalcup_home: dir.path().join("goalcup"),
        server_host: "ingest.example.org".into(),
    };
    let home = dir.path().join("goalcup/homes").join(CLAUDE);
    connector::plan(&connector::load(CLAUDE, None).unwrap(), &env, &home)
}

fn hint(value: &str) -> LaunchPlan {
    plan_with(&[("ANTHROPIC_BASE_URL", value)])
}

fn no_config(plan: &LaunchPlan) -> bool {
    plan.hosts.lists()["config"].as_array().unwrap().is_empty()
}

fn listed(plan: &LaunchPlan, host: &str) -> bool {
    let d = plan.hosts.decide(host);
    matches!(d, Decision::Listed { source, .. } if source == Source::Config)
}

fn refused(name: &str) -> bool {
    hosts::check(name, Source::Shared).is_err()
}

fn ips(list: &[&str]) -> Vec<IpAddr> {
    list.iter().map(|s| s.parse().unwrap()).collect()
}

#[test]
fn authority_with_backslash_or_control_is_never_a_host() {
    for value in [
        "https://api.example.com\\@evil.com/v1",
        "https://api.example.com evil.com/v1",
        "https://api.example.com\u{1}.evil.com/v1",
    ] {
        let plan = hint(value);
        assert!(no_config(&plan), "{value:?}");
        assert!(plan.notes.is_empty(), "{value:?}");
        assert!(!plan.log.is_empty(), "a skipped hint is noted: {value:?}");
        let shown = format!("{:?}", plan.log);
        assert!(!shown.contains("evil"), "never the value: {value:?}");
    }
    // The same input to the one host check (what `--record-host` calls) is refused whole.
    for name in [
        "api.example.com\\@evil.com",
        "api.example.com evil.com",
        "api.example.com\u{1}",
        "api.example.com\t",
        "api.example.com\u{7f}",
    ] {
        let skipped = hosts::check(name, Source::Participant).unwrap_err();
        assert!(!skipped.reason.contains("evil"), "{name:?}");
    }
    // Userinfo is dropped from a hint, and it is not a host name for the check.
    let plan = hint("https://user:pw-secret@api.example.com/v1");
    assert!(listed(&plan, "api.example.com"));
    assert!(!format!("{:?}{:?}", plan.log, plan.notes).contains("pw-secret"));
    let skipped = hosts::check("user:pw-secret@api.example.com", Source::Participant).unwrap_err();
    assert!(!skipped.reason.contains("pw-secret"));
}

#[test]
fn unspecified_and_mapped_loopback_reach_this_computer() {
    for name in ["0.0.0.0", "::", "::ffff:127.0.0.1", "::ffff:0.0.0.0"] {
        let skipped = hosts::check(name, Source::Shared).unwrap_err();
        assert!(skipped.local, "{name}");
        assert!(hosts::resolves_only_here(&ips(&[name])), "{name}");
    }
    let public = "::ffff:203.0.113.9";
    assert!(!hosts::check(public, Source::Shared).unwrap_err().local);
    assert!(!hosts::resolves_only_here(&ips(&[public])));
}

#[test]
fn loopback_names_and_numeric_labels_are_refused() {
    for name in [
        "localhost",
        "a.localhost",
        "LOCALHOST",
        "localhost.localdomain",
        "127.1",
        "0x7f.1",
        "127.0.0.01",
        "intranet",
    ] {
        assert!(refused(name), "{name}");
        assert!(hosts::check(name, Source::Participant).is_err(), "{name}");
    }
    assert!(!refused("localhost.example.com"));
}

#[test]
fn host_shape() {
    let name = |label: usize| "a".repeat(label);
    let ok = |n: &str| hosts::check(n, Source::Shared).is_ok();
    assert!(ok("api.example.com"));
    assert!(ok("a-b.c1.example.com"));
    // Uppercase is lowered, as the check defines it.
    assert_eq!(
        hosts::check("API.Example.COM", Source::Shared).unwrap().name,
        "api.example.com"
    );
    // Labels of 1 to 63.
    assert!(ok(&format!("{}.example.com", name(63))));
    assert!(!ok(&format!("{}.example.com", name(64))));
    assert!(!ok("a..example.com"));
    // At most 253 in all.
    let long = |last: usize| format!("{0}.{0}.{0}.{1}", name(63), name(last));
    assert_eq!(long(61).len(), 253);
    assert!(ok(&long(61)));
    assert!(!ok(&long(62)));
    // No other bytes, no address, no trailing dot.
    for bad in [
        "a_b.example.com",
        "ex ample.com",
        "203.0.113.9",
        "2001:db8::1",
        "example.com.",
        "example.com:443",
        "https://example.com",
        "",
    ] {
        assert!(!ok(bad), "{bad:?}");
    }
}

#[test]
fn the_event_server_is_never_listed() {
    for name in [
        "agentcommunity.org",
        "ingest.agentcommunity.org",
        "a.b.agentcommunity.org",
        "AgentCommunity.org",
        "goalcup.example.com",
        "api.goalcup.io",
        "my-goalcup-proxy.example.com",
    ] {
        assert!(refused(name), "{name}");
        assert!(hosts::check(name, Source::Participant).is_err(), "{name}");
        assert!(hosts::check(name, Source::Config).is_err(), "{name}");
    }
    assert!(!refused("notagentcommunity.org"));
}

#[test]
fn a_name_resolving_only_here_is_skipped() {
    assert!(hosts::resolves_only_here(&ips(&["127.0.0.1"])));
    assert!(hosts::resolves_only_here(&ips(&["127.0.0.1", "::1"])));
    assert!(hosts::resolves_only_here(&ips(&["0.0.0.0", "::"])));
    assert!(hosts::resolves_only_here(&ips(&["::ffff:127.0.0.1"])));
    assert!(!hosts::resolves_only_here(&ips(&["203.0.113.9"])));
    assert!(!hosts::resolves_only_here(&ips(&["::ffff:203.0.113.9"])));
    assert!(!hosts::resolves_only_here(&ips(&["127.0.0.1", "203.0.113.9"])));
    assert!(!hosts::resolves_only_here(&[]));
    // A hint that points here is a note, never a host.
    let plan = hint("http://127.0.0.1:11434/v1");
    assert!(no_config(&plan));
    assert_eq!(plan.notes.len(), 1, "{:?}", plan.notes);
    assert_eq!(plan.notes[0]["what"], "local_model");
}
