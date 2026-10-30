//! The end screen (PRD-10 section 5): "nothing was lost" only with no loss of any kind, the
//! fixture's counts and hosts, the rule lines when their facts are there, the gap words, and no
//! finished block for a part that is not sealed (PRD-17 SC6). Sentences come from `text.rs`.

mod cli_common;

use std::path::{Path, PathBuf};

use cli_common::{RUN, T0 as NOW, live, status};
use goalcup::cli::finish::{NotSealed, end_screen, screen};
use goalcup::cli::status::{State, block};
use goalcup::cli::text;
use goalcup::pipeline::counts::TunnelHost;
use goalcup::pipeline::status::{AfterSeal, CutFiles, Mode, Off, SealStatus, Status};
use goalcup::watchdog::SealReason;
use goalcup_format::GapReason;

/// A sealed part that wrote 905 requests and 43 session files and lost nothing.
fn sealed() -> Status {
    let mut st = status(Mode::Sealed);
    let c = &mut st.written.counts;
    c.http_post = 905;
    c.req_whole = 905;
    c.native_files = 43;
    c.native_bytes = 48_200_000;
    c.hosts = [
        ("api.example.com".to_owned(), 812),
        ("chat.example.com".to_owned(), 93),
    ]
    .into();
    st.written.model_requests = st.written.counts.model_requests();
    st.seal = Some(SealStatus {
        reason: SealReason::HarnessExit,
        workspace_head: None,
    });
    st.cut_files = Some(CutFiles::default());
    st.after_seal = Some(AfterSeal::default());
    st
}

/// One kind of loss, made in a clean sealed part.
type Lose = fn(&mut Status);

fn show(st: &Status) -> String {
    screen(st, None, RUN, &PathBuf::from("/home/x/goalcup.log"), NOW).expect("sealed")
}

/// Rule 1, through `screen`: the "nothing was lost" claim with no loss, and never with any one.
#[test]
fn nothing_lost_only_without_any_loss() {
    let clean = sealed();
    assert!(show(&clean).contains(text::NOTHING_LOST));
    let unlisted = TunnelHost {
        conns: 2,
        up: 5,
        down: 9,
        listed: None,
    };
    let mut quiet = clean.clone();
    quiet
        .written
        .counts
        .tunnel_hosts
        .insert("registry.example.org".into(), unlisted);
    assert!(
        show(&quiet).contains(text::NOTHING_LOST),
        "an unlisted tunnel is not a loss"
    );
    let losses: Vec<(&str, Lose)> = vec![
        ("gap", |s| s.written.counts.gaps = 1),
        ("gap of unknown size", |s| {
            s.written.counts.gaps = 1;
            s.written.counts.lost_unknown = 1;
        }),
        ("lost bytes", |s| s.written.counts.lost_bytes = 10),
        ("req_holes", |s| s.written.counts.req_holes = 1),
        ("off span", |s| {
            s.off = Off {
                spans: 1,
                ms: 150_000,
                since: None,
            }
        }),
        ("listed tunnel", |s| {
            s.written.counts.tunnel_hosts.insert(
                "pinned.example.com".into(),
                TunnelHost {
                    conns: 1,
                    up: 1,
                    down: 1,
                    listed: Some("pinned".into()),
                },
            );
        }),
        ("fail-closed byte", |s| {
            s.written.counts.mask_failclosed_bytes = 1
        }),
        ("cut file", |s| {
            s.cut_files = Some(CutFiles {
                files: 1,
                bytes: 10,
            })
        }),
        ("after-seal bytes", |s| {
            s.after_seal = Some(AfterSeal {
                bytes: 1,
                hosts: Vec::new(),
            })
        }),
    ];
    // Each loss has its line: one at least takes the place of the claim.
    let clean_lines = show(&clean).lines().count();
    for (kind, lose) in &losses {
        let mut st = clean.clone();
        lose(&mut st);
        let out = show(&st);
        assert!(!out.contains(text::NOTHING_LOST), "{kind}");
        assert!(out.lines().count() >= clean_lines, "{kind} has no line");
    }
}

#[test]
fn counts_and_hosts_are_the_fixtures() {
    let mut st = sealed();
    let out = show(&st);
    let hosts = [("api.example.com", 812), ("chat.example.com", 93)];
    assert!(out.contains(&text::requests_written(905, &hosts)), "{out}");
    assert!(out.contains(&text::session_files(43, 48_200_000)));
    assert!(!out.contains("896"), "req_whole only with holes");
    st.written.counts.req_whole = 896;
    st.written.counts.req_holes = 9;
    st.written.counts.lost_bytes = 1_500_000;
    st.off = Off {
        spans: 2,
        ms: 150_000,
        since: None,
    };
    let out = show(&st);
    assert!(
        out.contains(&text::whole_and_lost(896, 9, 1_500_000)),
        "{out}"
    );
    assert!(out.contains(&text::off_total(2, 150_000)));
    st.written.model_requests = 0;
    assert!(show(&st).contains(&text::no_request_written("your agent")));
}

#[test]
fn tunnels_largest_first_and_listed_alone() {
    let mut st = sealed();
    for (i, host) in ["a", "b", "c", "d", "e", "f", "g"].iter().enumerate() {
        let t = TunnelHost {
            conns: 1,
            up: 1000 * (i as u64 + 1),
            down: 0,
            listed: None,
        };
        st.written
            .counts
            .tunnel_hosts
            .insert(format!("{host}.example.org"), t);
    }
    let pinned = TunnelHost {
        conns: 4,
        up: 1,
        down: 2,
        listed: Some("pinned".into()),
    };
    st.written
        .counts
        .tunnel_hosts
        .insert("z.example.com".into(), pinned);
    let out = show(&st);
    let top: Vec<(String, u64, u64)> = ["g", "f", "e", "d", "c"]
        .iter()
        .zip([7000, 6000, 5000, 4000, 3000])
        .map(|(h, b)| (format!("{h}.example.org"), 1, b))
        .collect();
    let top: Vec<(&str, u64, u64)> = top.iter().map(|(h, c, b)| (h.as_str(), *c, *b)).collect();
    assert!(out.contains(&text::tunnels_unlisted(&top, 2)), "{out}");
    assert!(
        out.contains(&text::tunnel_listed("z.example.com", "pinned", 4, 3)),
        "{out}"
    );
}

#[test]
fn rule_lines_follow_their_facts() {
    let mut st = sealed();
    let after = text::after_seal("your agent", 2_000_000, &["api.example.com".into()]);
    st.after_seal = Some(AfterSeal {
        bytes: 0,
        hosts: vec!["api.example.com".into()],
    });
    assert!(!show(&st).contains(&after));
    st.after_seal.as_mut().unwrap().bytes = 2_000_000;
    assert!(show(&st).contains(&after));
    st.cut_files = Some(CutFiles {
        files: 3,
        bytes: 2_000_000,
    });
    assert!(show(&st).contains(&text::cut_files(3, 2_000_000)));
    assert!(!show(&st).contains(text::RECOVERED));
    st.seal.as_mut().unwrap().reason = SealReason::Recovered;
    assert!(show(&st).contains(text::RECOVERED));
    let log = Path::new("/home/x/Application Support/goalcup.log");
    assert!(
        screen(&st, None, RUN, log, NOW)
            .unwrap()
            .contains(&text::details_at(log))
    );
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

/// PRD-17 SC6: a `status.json` with no seal never prints the finished line or block.
#[test]
fn unsealed_status_never_reads_as_finished() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join(RUN).join("e0123456789abcdef");
    std::fs::create_dir_all(&entry).unwrap();
    for mode in [Mode::Recording, Mode::Losing, Mode::WriterSlow, Mode::Null] {
        for seal in [
            None,
            Some(SealStatus {
                reason: SealReason::HarnessExit,
                workspace_head: None,
            }),
        ] {
            let mut st = sealed();
            st.mode = mode;
            st.seal = seal;
            let log = PathBuf::from("/l");
            assert_eq!(screen(&st, None, RUN, &log, NOW), Err(NotSealed));
            std::fs::write(entry.join("status.json"), serde_json::to_vec(&st).unwrap()).unwrap();
            assert_eq!(end_screen(&entry), Err(NotSealed));
            let cx = live(true, true, true, 0);
            for state in [State::Recording, State::Continue { since: NOW }] {
                let shown = block(&st, &state, None, &cx, &[state.word()]);
                assert!(!shown.contains(text::FINISHED) && !shown.contains(text::PART_RECORDED));
            }
        }
    }
    let mut st = sealed();
    st.seal = None;
    std::fs::write(entry.join("status.json"), serde_json::to_vec(&st).unwrap()).unwrap();
    assert_eq!(
        end_screen(&entry),
        Err(NotSealed),
        "sealed mode with no seal facts"
    );
    std::fs::write(
        entry.join("status.json"),
        serde_json::to_vec(&sealed()).unwrap(),
    )
    .unwrap();
    assert!(end_screen(&entry).unwrap().contains(text::PART_RECORDED));
}
