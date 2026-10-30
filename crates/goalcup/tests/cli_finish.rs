//! The end screen (PRD-10 section 5): "nothing was lost" only with no loss of any kind, the
//! fixture's counts and hosts, the rule lines when their facts are there, the gap words, and no
//! finished block for a part that is not sealed (PRD-17 SC6). Sentences come from `text.rs`.

use std::path::{Path, PathBuf};

use goalcup::cli::finish::{NotSealed, end_screen, screen};
use goalcup::cli::status::{Ctx, State, block};
use goalcup::cli::text;
use goalcup::pipeline::counts::TunnelHost;
use goalcup::pipeline::status::{
    AfterSeal, CutFiles, DurableStatus, Losing, Mode, Off, STATUS_V, SealStatus, Status, Written,
};
use goalcup::watchdog::SealReason;

const NOW: u64 = 1_794_045_600;
const RUN: &str = "r0123456789abcdef";

fn sealed() -> Status {
    let mut written = Written::default();
    written.counts.http_post = 905;
    written.counts.req_whole = 905;
    written.counts.native_files = 43;
    written.counts.native_bytes = 48_200_000;
    written.counts.hosts = [
        ("api.example.com".to_owned(), 812),
        ("chat.example.com".to_owned(), 93),
    ]
    .into();
    written.model_requests = written.counts.model_requests();
    Status {
        v: STATUS_V,
        updated: "2026-11-07T10:00:00Z".into(),
        pid: 0,
        start_id: 0,
        boot: 0,
        port: 41_000,
        mode: Mode::Sealed,
        harness: None,
        written,
        last_request_at: None,
        last_native_at: None,
        losing: Losing::default(),
        off: Off::default(),
        notes: Vec::new(),
        held_bytes: 0,
        held_peak: 0,
        guard: 0,
        writer_idle_ms: 0,
        durable: DurableStatus::default(),
        deadline: None,
        sealing: None,
        seal: Some(SealStatus {
            reason: SealReason::HarnessExit,
            workspace_head: None,
        }),
        cut_files: Some(CutFiles::default()),
        outlived: None,
        after_seal: Some(AfterSeal::default()),
    }
}

fn show(st: &Status) -> String {
    screen(st, None, RUN, &PathBuf::from("/home/x/goalcup.log"), NOW).expect("sealed")
}

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
    let losses: Vec<fn(&mut Status)> = vec![
        |s| s.written.counts.gaps = 1,
        |s| s.written.counts.lost_bytes = 10,
        |s| s.written.counts.req_holes = 1,
        |s| {
            s.off = Off {
                spans: 1,
                ms: 150_000,
                since: None,
            }
        },
        |s| {
            s.written.counts.tunnel_hosts.insert(
                "pinned.example.com".into(),
                TunnelHost {
                    conns: 1,
                    up: 1,
                    down: 1,
                    listed: Some("pinned".into()),
                },
            );
        },
        |s| s.written.counts.mask_failclosed_bytes = 1,
        |s| {
            s.cut_files = Some(CutFiles {
                files: 1,
                bytes: 10,
            })
        },
        |s| {
            s.after_seal = Some(AfterSeal {
                bytes: 1,
                hosts: Vec::new(),
            })
        },
    ];
    for (i, lose) in losses.iter().enumerate() {
        let mut st = clean.clone();
        lose(&mut st);
        assert!(!show(&st).contains(text::NOTHING_LOST), "loss {i}");
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
    for n in 1..=15u8 {
        let words = text::gap_words(n);
        assert!(!words.is_empty());
        assert_eq!(words == other(n), n == 8 || n == 9, "{n}: {words}");
    }
    for n in [0u8, 16, 200] {
        assert_eq!(text::gap_words(n), other(n));
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
            let cx = Ctx {
                name: "Agent",
                run: RUN,
                harness_alive: true,
                deadline: None,
                now: NOW,
            };
            for state in [State::Recording, State::Continue { since: NOW }] {
                let shown = block(&st, &state, None, &cx, &[false]);
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
