//! The host list (PRD-11 section 4): `never` and goalcup's own server win over every list (PRD-17
//! HF8), `pinned` marks a listed host, `record-hosts.txt` fills the participant list, and the run
//! CA gets wildcard-free suffixes. The N1 to N6 name cases live in PRD-17's `lessons_hosts.rs`.

use std::collections::BTreeMap;
use std::time::SystemTime;

use goalcup::connector::{self, Decision, Env, HostList, Kind, Source};

fn list(text: &str) -> HostList {
    HostList::from_file(text.as_bytes(), "hosts.toml").unwrap()
}

#[test]
fn never_and_the_own_server_win_over_every_list() {
    let hosts = list("provider = [\"*.example.com\"]\nnever = [\"auth.example.com\"]\n");
    assert!(matches!(
        hosts.decide("api.example.com"),
        Decision::Listed { .. }
    ));
    for name in [
        "auth.example.com",
        "AUTH.example.com",
        "goalcup.example.com",
    ] {
        assert_eq!(hosts.decide(name), Decision::Never, "{name}");
    }
    assert_eq!(hosts.decide("ingest.agentcommunity.org"), Decision::Never);
    assert_eq!(hosts.decide("agentcommunity.org"), Decision::Never);
}

#[test]
fn a_pinned_entry_decides_pinned() {
    let hosts = list(
        "provider = [\"api.example.com\", \"api.example.net\"]\npinned = [\"api.example.com\"]\n",
    );
    let pinned = |pinned| Decision::Listed {
        source: Source::Shared,
        kind: Kind::Provider,
        pinned,
    };
    assert_eq!(hosts.decide("api.example.com"), pinned(true));
    assert_eq!(hosts.decide("api.example.net"), pinned(false));
}

#[test]
fn record_hosts_fill_the_participant_list_and_bad_lines_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir_all(d.join("goalcup")).unwrap();
    let lines = "# mine\n\n  models.example.net  \nbad host\n127.0.0.1\nlocalhost\n*.wild.example.org\na.*.example.org\nauth.openai.com\ningest.example.org\n";
    std::fs::write(d.join("goalcup/record-hosts.txt"), lines).unwrap();
    let env = Env {
        vars: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
        workspace: d.join("workspace"),
        resume: false,
        run_start: SystemTime::now(),
        goalcup_home: d.join("goalcup"),
        server_host: "ingest.example.org".into(),
    };
    let c = connector::load("codex-cli", None).unwrap();
    let plan = connector::plan(&c, &env, &d.join("goalcup/homes/codex-cli"));
    let other = Decision::Listed {
        source: Source::Participant,
        kind: Kind::Other,
        pinned: false,
    };
    assert_eq!(plan.hosts.decide("models.example.net"), other);
    assert_eq!(plan.hosts.decide("x.wild.example.org"), other);
    let participant = plan.hosts.lists()["participant"].as_array().unwrap().len();
    assert_eq!(participant, 2, "only the two good lines, never hosts left out");
    let never = plan.log.iter().filter(|l| l.contains("record-hosts.txt")).count();
    assert_eq!(never, 2, "one line per never host: {:?}", plan.log);
}

#[test]
fn ca_subtrees_are_wildcard_free_suffixes() {
    let hosts = list(
        "provider = [\"api.example.com\", \"*.a.example.net\", \"bedrock.*.example.org\"]\n\
         gateway = [\"x*y.b.example.io\", \"api.example.com\"]\n",
    );
    let want = [
        "api.example.com",
        "a.example.net",
        "example.org",
        "b.example.io",
    ];
    assert_eq!(hosts.ca_subtrees(), want);
    assert!(matches!(hosts.decide("x-mid-y.b.example.io"), Decision::Listed { .. }));
    assert_eq!(hosts.decide("x-mid.b.example.io"), Decision::NotListed, "in-label `*`");
}
