//! The end screen (PRD-10 section 5): "nothing was lost" only with no loss of any kind, the
//! fixture's counts and hosts, the rule lines when their facts are there, the gap words, and no
//! finished block for a part that is not sealed (PRD-17 SC6). Sentences come from `text.rs`.

mod cli_common;

use std::path::Path;

use cli_common::{RUN, T0 as NOW, live, part, patch, put, with};
use goalcup::cli::finish::{NotSealed, end_screen, screen};
use goalcup::cli::status::{State, block};
use goalcup::cli::text;
use goalcup::pipeline::status::{Mode, Status};
use goalcup_format::GapReason;
use serde_json::{Map, Value, json};

/// A sealed part that wrote 905 requests to two hosts and 43 session files, and lost nothing.
fn sealed() -> Status {
    with(
        Mode::Sealed,
        json!({"written": {"model_requests": 905, "http_post": 905, "req_whole": 905,
                "native_files": 43, "native_bytes": 48_200_000,
                "hosts": {"api.example.com": 812, "chat.example.com": 93}},
            "seal": {"reason": "harness_exit"}, "cut_files": {"files": 0, "bytes": 0},
            "after_seal": {"bytes": 0, "hosts": []}}),
    )
}

/// The end screen of `sealed()` with `p` laid over it.
fn show(p: Value) -> String {
    let st = patch(&sealed(), p);
    screen(&st, None, RUN, Path::new("/home/x/goalcup.log"), NOW).expect("sealed")
}

/// Rule 1: the "nothing was lost" claim with no loss, and never with any one, whose line takes its
/// place. An unlisted tunnel is not a loss.
#[test]
fn nothing_lost_only_without_any_loss() {
    let tunnel = |listed: Value| {
        json!({"written": {"tunnel_hosts": {"t.example.org":
            {"conns": 2, "up": 5, "down": 9, "listed": listed}}}})
    };
    let clean_lines = show(json!({})).lines().count();
    for (kind, p, lost) in [
        ("no loss", json!({}), false),
        ("unlisted tunnel", tunnel(Value::Null), false),
        ("listed tunnel", tunnel(json!("pinned")), true),
        ("gap", json!({"written": {"gaps": 1}}), true),
        ("gap of unknown size", json!({"written": {"gaps": 1, "lost_unknown": 1}}), true),
        ("lost bytes", json!({"written": {"lost_bytes": 10}}), true),
        ("req_holes", json!({"written": {"req_holes": 1}}), true),
        ("off span", json!({"off": {"spans": 1, "ms": 150_000}}), true),
        ("fail-closed byte", json!({"written": {"mask_failclosed_bytes": 1}}), true),
        ("cut file", json!({"cut_files": {"files": 1, "bytes": 10}}), true),
        ("after-seal bytes", json!({"after_seal": {"bytes": 1}}), true),
    ] {
        let out = show(p);
        assert_eq!(out.contains(text::NOTHING_LOST), !lost, "{kind}");
        assert!(out.lines().count() >= clean_lines, "{kind} has no line");
    }
}

/// The printed counts and hosts are the fixture's, tunnels largest first with a listed host on its
/// own line; `req_whole` is shown only when `req_holes` is above 0.
#[test]
fn counts_and_hosts_are_the_fixtures() {
    let out = show(json!({}));
    let hosts = [("api.example.com", 812), ("chat.example.com", 93)];
    assert!(out.contains(&text::requests_written(905, &hosts)), "{out}");
    assert!(out.contains(&text::session_files(43, 48_200_000)));
    let whole = json!({"written": {"req_whole": 896}});
    assert!(!show(whole).contains("896"), "req_whole only with holes");
    let holes = json!({"written": {"req_whole": 896, "req_holes": 9, "lost_bytes": 1_500_000,
        "model_requests": 0}, "off": {"spans": 2, "ms": 150_000}});
    let out = show(holes);
    assert!(out.contains(&text::whole_and_lost(896, 9, 1_500_000)), "{out}");
    assert!(out.contains(&text::off_total(2, 150_000)));
    assert!(out.contains(&text::no_request_written("your agent")));
    let host = |c: char| format!("{c}.example.org");
    let mut tunnels: Map<String, Value> = ('a'..='g')
        .zip(1u64..)
        .map(|(c, i)| (host(c), json!({"conns": 1, "up": 1000 * i, "down": 0})))
        .collect();
    let pinned = json!({"conns": 4, "up": 1, "down": 2, "listed": "pinned"});
    tunnels.insert("z.example.com".into(), pinned);
    let out = show(json!({"written": {"tunnel_hosts": tunnels}}));
    let top: Vec<String> = ('c'..='g').rev().map(host).collect();
    let top: Vec<(&str, u64, u64)> = top
        .iter()
        .zip((3..=7).rev())
        .map(|(h, i)| (h.as_str(), 1, 1000 * i))
        .collect();
    assert!(out.contains(&text::tunnels_unlisted(&top, 2)), "{out}");
    let listed = text::tunnel_listed("z.example.com", "pinned", 4, 3);
    assert!(out.contains(&listed), "{out}");
}

/// The after-seal line only when bytes came after the seal, the cut-file line with cut files, the
/// recovered line only for a recovered seal.
#[test]
fn rule_lines_follow_their_facts() {
    let after = text::after_seal("your agent", 2_000_000, &["api.example.com".into()]);
    let seal_after = |b: u64| json!({"after_seal": {"bytes": b, "hosts": ["api.example.com"]}});
    let (cut, recovered) = (text::cut_files(3, 2_000_000), text::RECOVERED.to_owned());
    for (p, line, shown) in [
        (seal_after(0), &after, false),
        (seal_after(2_000_000), &after, true),
        (json!({"cut_files": {"files": 3, "bytes": 2_000_000}}), &cut, true),
        (json!({}), &recovered, false),
        (json!({"seal": {"reason": "recovered"}}), &recovered, true),
    ] {
        assert_eq!(show(p.clone()).contains(line.as_str()), shown, "{p}");
    }
}

#[test]
fn every_gap_reason_has_words() {
    let other = |n: u8| format!("other ({n})");
    let words = |n: u8| text::gap_words(&GapReason::from_u8(n).name());
    for n in 1..=15u8 {
        assert!(!words(n).is_empty());
        assert_eq!(words(n) == other(n), n == 8 || n == 9, "{n}: {}", words(n));
    }
    for n in [0u8, 16, 200] {
        assert_eq!(words(n), other(n));
    }
}

/// PRD-17 SC6: a `status.json` with no seal never prints the finished line or block: an open
/// mode with or without seal facts, or sealed mode without them.
#[test]
fn unsealed_status_never_reads_as_finished() {
    let dir = tempfile::tempdir().unwrap();
    let entry = part(dir.path());
    let mut cases = vec![json!({"seal": null})];
    for mode in [Mode::Recording, Mode::Losing, Mode::WriterSlow, Mode::Null] {
        cases.extend([json!({"mode": mode}), json!({"mode": mode, "seal": null})]);
    }
    let cx = live(true, true, true, 0);
    for p in cases {
        let st = patch(&sealed(), p.clone());
        let log = Path::new("/l");
        assert_eq!(screen(&st, None, RUN, log, NOW), Err(NotSealed), "{p}");
        put(&entry, "status.json", &st);
        assert_eq!(end_screen(&entry), Err(NotSealed), "{p}");
        for state in [State::Recording, State::Continue { since: NOW }] {
            let shown = block(&st, &state, None, &cx, &[state.word()]);
            let finished = shown.contains(text::FINISHED) || shown.contains(text::PART_RECORDED);
            assert!(!finished, "{p}");
        }
    }
    put(&entry, "status.json", &sealed());
    assert!(end_screen(&entry).unwrap().contains(text::PART_RECORDED));
}
