use super::*;
use crate::account::device::RunMode;
use std::path::Path;

fn last(lines: &[String]) -> &str {
    lines.last().map(String::as_str).unwrap_or("")
}

#[test]
fn a_clock_is_a_weekday_a_date_a_time_and_a_zone() {
    // 2026-11-14T09:12:00Z was a Saturday.
    let unix = goalcup_protocol::control::parse_z("2026-11-14T09:12:00Z").unwrap();
    // Local time with its zone, and UTC in brackets.
    assert_eq!(
        clock_words(unix, 3600, "CET"),
        "Sat 14 Nov, 10:12 CET (09:12 UTC)"
    );
    // A computer on UTC says it once.
    assert_eq!(clock_words(unix, 0, "UTC"), "Sat 14 Nov, 09:12 UTC");
    // Over midnight the UTC date is named; a zone with no plain name is its offset (macOS
    // names some zones `+04`).
    assert_eq!(
        clock_words(unix, 15 * 3600, "+15"),
        "Sun 15 Nov, 00:12 UTC+15:00 (Sat 14 Nov, 09:12 UTC)"
    );
    assert_eq!(
        clock_words(unix, -(9 * 3600 + 1800), "\u{1b}[31m"),
        "Fri 13 Nov, 23:42 UTC-09:30 (Sat 14 Nov, 09:12 UTC)"
    );
    // The local one always carries the UTC minute.
    assert!(
        local_clock(unix).contains("09:12 UTC"),
        "{}",
        local_clock(unix)
    );
}

#[test]
fn time_left_is_hours_and_minutes() {
    assert_eq!(left_words(95 * 3600 + 10 * 60 + 59), "95 h 10 min left");
    assert_eq!(left_words(168 * 3600), "168 h 0 min left");
    assert_eq!(left_words(600), "10 min left");
    assert_eq!(left_words(5), "less than a minute left");
    let end = end_words("2026-11-14T09:12:00Z", Some(600));
    assert!(end.ends_with(", 10 min left"), "{end}");
    assert_eq!(end_words("not a time", None), "not a time");
}

#[test]
fn the_time_left_follows_the_servers_deadline_without_a_count() {
    let deadline = goalcup_protocol::control::format_z(1_000_000);
    assert_eq!(seconds_left(None, Some(&deadline), 999_400), Some(600));
    assert_eq!(seconds_left(None, Some(&deadline), 2_000_000), Some(0));
    assert_eq!(seconds_left(None, None, 1), None);
}

#[test]
fn each_approval_says_what_it_is_for_and_that_two_are_normal() {
    let login = approval_intro(Approving::Login).join(" ");
    assert!(login.contains("Connect this computer"), "{login}");
    assert!(login.contains("once per computer"), "{login}");
    for (mode, word) in [
        (RunMode::Practice, "practice run"),
        (RunMode::Competition, "competition run"),
        (RunMode::Test, "test run"),
    ] {
        let run = approval_intro(Approving::Run {
            mode,
            harness: "Some Agent",
        })
        .join(" ");
        assert!(run.contains(word), "{run}");
        assert!(run.contains("Some Agent"), "{run}");
        assert!(run.contains("normal"), "{run}");
    }
    // The matrix reads the code from `shows the code XXXX-XXXX.`
    let address = "https://x.example/a?code=BCDF-GHJK";
    let code = approval_code("BCDF-GHJK", address, true, 900);
    assert!(code[0].contains("opened"), "{code:?}");
    assert!(code[0].ends_with("shows the code BCDF-GHJK."), "{code:?}");
    assert!(code[1].contains(address));
    assert!(code[2].contains("15 min"), "{code:?}");
    // No browser: the address and the code in one line, and nothing says a browser opened.
    let code = approval_code("BCDF-GHJK", address, false, 900);
    assert!(code[0].contains(address), "{code:?}");
    assert!(code[0].ends_with("shows the code BCDF-GHJK."), "{code:?}");
    assert!(!code.join(" ").contains("opened"), "{code:?}");
    assert!(replaces_open_run("Fri 7 Nov").contains("replaces"));
    let done = login_done("member-1", None);
    assert!(done.join(" ").contains("member-1"));
    assert!(!done.join(" ").contains("upload to"));
    assert_eq!(last(&done), "Next: goalcup setup");
    assert!(
        login_done("m", Some("https://s.example"))
            .join(" ")
            .contains("https://s.example")
    );
}

#[test]
fn the_launch_block_says_the_prompt_rule_the_end_and_ends_with_the_ready_mark() {
    let ends = end_words("2026-11-14T09:12:00Z", Some(95 * 3600 + 600));
    let block = launch_block(&Launch {
        harness: "Some Agent",
        mode: Some(RunMode::Competition),
        opens: Opens::First,
        ends: Some(ends.clone()),
        first_screen: Some("It asks whether you trust this folder. Choose Yes."),
    });
    let text = block.join("\n");
    assert!(
        text.contains("Some Agent opens now in this terminal."),
        "{text}"
    );
    assert!(text.contains("one prompt"), "{text}");
    assert!(text.contains("press Enter"), "{text}");
    assert!(text.contains(KEEP_GOING), "{text}");
    assert!(text.contains(&ends), "{text}");
    assert!(text.contains("95 h 10 min left"), "{text}");
    assert!(text.contains("trust this folder"), "{text}");
    assert!(last(&block).starts_with(READY_MARK), "{text}");
    // The trust hint comes before what to type (it is the first screen), and nothing
    // internal is said.
    assert!(text.find("trust").unwrap() < text.find("one prompt").unwrap());
    for word in ["entry", "segment", "daemon", "host.log", "D57"] {
        assert!(!text.contains(word), "{word}: {text}");
    }

    let continued = launch_block(&Launch {
        harness: "Some Agent",
        mode: Some(RunMode::Competition),
        opens: Opens::LastSession,
        ends: Some(ends.clone()),
        first_screen: None,
    })
    .join("\n");
    assert!(continued.contains(RUN_GOES_ON), "{continued}");
    assert!(
        continued.contains("picks up its last session"),
        "{continued}"
    );
    assert!(continued.contains(KEEP_GOING), "{continued}");
    assert!(continued.contains("never sent your prompt"), "{continued}");
    // What this computer kept is the approval, never a "key" a reader takes for an API key.
    assert!(!continued.contains("key"), "{continued}");

    // A continue of a run with no session promises none: it says the session is new, in
    // one line, and asks for the one prompt as a first launch does.
    let fresh = launch_block(&Launch {
        harness: "Some Agent",
        mode: Some(RunMode::Competition),
        opens: Opens::NewSession,
        ends: Some(ends),
        first_screen: None,
    });
    let text = fresh.join("\n");
    assert!(text.contains(RUN_GOES_ON), "{text}");
    assert!(!text.contains("picks up"), "{text}");
    assert_eq!(
        fresh.iter().filter(|l| l.contains("new session")).count(),
        1,
        "{text}"
    );
    assert!(
        text.contains(
            "Some Agent opens now in this terminal with a new session: no saved session \
             was found for this run."
        ),
        "{text}"
    );
    assert!(
        text.contains("Type your one prompt and press Enter"),
        "{text}"
    );

    let practice = launch_block(&Launch {
        harness: "A",
        mode: Some(RunMode::Practice),
        opens: Opens::First,
        ends: None,
        first_screen: None,
    })
    .join("\n");
    assert!(practice.contains("practice run"), "{practice}");
    assert!(!practice.contains("Your run ends at"), "{practice}");
}

#[test]
fn an_exit_while_the_run_is_open_names_the_command_the_run_and_the_end() {
    let until = end_words("2026-11-14T09:12:00Z", Some(3600));
    let open = Ending::Open {
        run_id: "r0123456789abcdef",
        how: "exit code 1",
        until: until.clone(),
    };
    let block = ending_block("Some Agent", &open);
    let text = block.join("\n");
    assert!(
        text.contains("Some Agent has exited (exit code 1)."),
        "{text}"
    );
    assert!(text.contains(&format!("{STILL_OPEN} {until}")), "{text}");
    assert!(
        text.contains(&format!("{CONTINUE_HINT} r0123456789abcdef")),
        "{text}"
    );
    assert!(text.contains("another terminal now"), "{text}");
    assert!(last(&block).contains("need do nothing"), "{text}");
    // After the upload: done, or what sends the rest; or the server closed it meanwhile.
    assert!(last(&after_upload(&open, true, false)).starts_with(UPLOAD_DONE));
    assert!(
        after_upload(&open, false, false)
            .join(" ")
            .ends_with("goalcup upload")
    );
    let closed = after_upload(&open, true, true).join(" ");
    assert!(closed.contains("cannot be continued"), "{closed}");
}

#[test]
fn a_run_that_is_over_never_offers_continue_and_ends_with_what_to_do() {
    for ending in [
        Ending::TimeUp { practice: false },
        Ending::TimeUp { practice: true },
        Ending::Replaced,
        Ending::Over { how: "exit code 0" },
        Ending::NoRestart {
            how: "exit code 0",
            until: "Sat 14 Nov, 10:12 CET (09:12 UTC)".into(),
        },
    ] {
        for all_sent in [true, false] {
            let head = ending_block("Some Agent", &ending);
            let tail = after_upload(&ending, all_sent, false);
            let text = head
                .iter()
                .chain(&tail)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!text.contains(CONTINUE_HINT), "{ending:?}: {text}");
            assert!(head.join(" ").contains("Some Agent"), "{ending:?}: {text}");
            let last_line = last(&tail);
            assert!(
                last_line.contains(NOTHING_MORE)
                    || last_line.starts_with("Part of the run")
                    || last_line.starts_with("Next: your real run"),
                "{ending:?}: {text}"
            );
            if !all_sent {
                assert!(text.contains("goalcup upload"), "{ending:?}: {text}");
            }
        }
    }
    let up = ending_block("A", &Ending::TimeUp { practice: false }).join("\n");
    assert!(up.contains(TIME_IS_UP), "{up}");
    let sent = after_upload(&Ending::TimeUp { practice: false }, true, false).join("\n");
    assert!(sent.contains("sealed and uploaded"), "{sent}");
    // A practice run that is all uploaded points to the real run, and does not also say there
    // is nothing more to do.
    let practice = after_upload(&Ending::TimeUp { practice: true }, true, false);
    assert!(
        last(&practice).starts_with("Next: your real run"),
        "{practice:?}"
    );
    assert!(!practice.join(" ").contains(NOTHING_MORE), "{practice:?}");
    // With part of it unsent the prep step is not ok yet: no pointer to it, the upload
    // command instead.
    let unsent = after_upload(&Ending::TimeUp { practice: true }, false, false);
    let text = unsent.join(" ");
    assert!(!text.contains("goalcup prep"), "{text}");
    assert!(text.contains("goalcup upload"), "{text}");
}

fn header(phase: Option<Phase>, mode: Option<RunMode>) -> StatusHeader<'static> {
    StatusHeader {
        harness: "Some Agent",
        mode,
        phase,
        ends: Some(ends_line(
            "2026-11-14T09:12:00Z",
            Some(95 * 3600 + 600),
            End::Open,
        )),
        agent: Agent::Running,
        upload: UploadNow::UpToDate { ago: Some(120) },
        problems: 0,
        stale: false,
        early: false,
        young: true,
    }
}

#[test]
fn the_status_header_names_the_run_the_state_the_end_the_agent_and_the_upload() {
    let text =
        status_header(&header(Some(Phase::Recording), Some(RunMode::Competition))).join("\n");
    assert!(
        text.starts_with("Your competition run with Some Agent: recording."),
        "{text}"
    );
    assert!(text.contains("95 h 10 min left"), "{text}");
    assert!(text.contains("09:12 UTC"), "{text}");
    assert!(text.contains("Some Agent: running."), "{text}");
    assert!(text.contains("up to date (last sent 2 min ago)"), "{text}");
    // Records waiting for the next timed send: never "up to date".
    for (waiting, said) in [
        (
            Some(3201),
            "Upload: working, sent every 30 minutes (last sent 2 min ago; 3201 records wait for \
             the next send).",
        ),
        (
            Some(1),
            "Upload: working, sent every 30 minutes (last sent 2 min ago; 1 record waits for the \
             next send).",
        ),
        (
            None,
            "Upload: working, sent every 30 minutes (last sent 2 min ago).",
        ),
    ] {
        let mut sending = header(Some(Phase::Recording), Some(RunMode::Competition));
        sending.upload = UploadNow::Sending {
            ago: Some(120),
            waiting,
        };
        let text = status_header(&sending).join("\n");
        assert!(text.contains(said), "{said}: {text}");
        assert!(!text.contains("up to date"), "{text}");
    }
    assert!(text.contains("Problems: none."), "{text}");

    for (mode, word) in [
        (Some(RunMode::Practice), "practice run"),
        (Some(RunMode::Test), "organizer test run"),
        (None, "this computer only"),
    ] {
        let text = status_header(&header(Some(Phase::Recording), mode)).join("\n");
        assert!(text.contains(word), "{word}: {text}");
    }
    // Each state is in the first line.
    for (phase, word) in [
        (Phase::AgentExited, "has exited, the run is still open"),
        (Phase::Finishing, "time is up"),
        (Phase::Sealed, "sealed"),
        (Phase::Uploaded, "sealed and uploaded"),
    ] {
        let first = status_header(&header(Some(phase), Some(RunMode::Competition))).remove(0);
        assert!(first.contains(word), "{word}: {first}");
    }
    // A run over says when it ended and has no time left.
    let over = ends_line("2026-11-14T09:12:00Z", Some(5), End::Over);
    assert!(over.starts_with("Ended: "), "{over}");
    assert!(!over.contains("left"), "{over}");
    // A run closed before its time names no deadline as its end.
    let early = ends_line("2026-11-14T09:12:00Z", Some(5), End::Early);
    assert!(early.starts_with("Ended early"), "{early}");
    assert!(!early.contains("09:12"), "{early}");
    // No state yet: said in plain words, and nothing about the agent or the upload.
    let text = status_header(&header(None, Some(RunMode::Practice))).join("\n");
    assert!(text.contains("no run state yet"), "{text}");
    assert!(!text.contains("Upload:"), "{text}");
    // An old run with no state did not start.
    let mut old = header(None, Some(RunMode::Practice));
    old.young = false;
    let text = status_header(&old).join("\n");
    assert!(text.contains("did not start"), "{text}");
    // A state nothing updates is not called recording, and the agent is not called running.
    let mut stale = header(Some(Phase::Recording), Some(RunMode::Competition));
    stale.stale = true;
    let text = status_header(&stale).join("\n");
    assert!(
        !text.contains("recording") && !text.contains("running"),
        "{text}"
    );
    // Time up and nothing finishing the run: say so, not "finishing".
    let mut dead = header(Some(Phase::Finishing), Some(RunMode::Competition));
    dead.stale = true;
    let first = status_header(&dead).remove(0);
    assert!(first.contains("stopped before it finished"), "{first}");
    // Problems are counted.
    let mut some = header(Some(Phase::Recording), Some(RunMode::Competition));
    some.problems = 2;
    assert!(status_header(&some).join("\n").contains(" 2"));
    // The agent and the upload in trouble.
    some.agent = Agent::Exited { how: "exit code 1" };
    some.upload = UploadNow::GaveUp;
    let text = status_header(&some).join("\n");
    assert!(text.contains("exited (exit code 1)"), "{text}");
    assert!(text.contains("stopped"), "{text}");
    for word in ["entry", "segment", "chain", "daemon", "tracer"] {
        assert!(!text.contains(word), "{word}: {text}");
    }
}

#[test]
fn the_status_next_line_is_the_one_thing_to_do() {
    assert!(status_next(&Next::NothingEnds).starts_with("Next: nothing to do."));
    assert!(status_next(&Next::Nothing).starts_with("Next: nothing to do."));
    // A command ends its line: nothing after it to be copied with it.
    let go_on = status_next(&Next::Continue { run_id: "r1" });
    assert!(go_on.ends_with("goalcup continue r1"), "{go_on}");
    assert!(status_next(&Next::Upload).ends_with("goalcup upload"));
    let seal = status_next(&Next::Seal { upload: true });
    assert!(
        seal.contains("goalcup seal") && seal.ends_with("goalcup upload"),
        "{seal}"
    );
    assert!(!status_next(&Next::Seal { upload: false }).contains("upload"));
    let restart = status_next(&Next::Restart {
        connector_id: "some-cli",
        mode: Some(RunMode::Practice),
    });
    assert!(
        restart.contains("goalcup start --harness some-cli --practice"),
        "{restart}"
    );
    assert!(status_next(&Next::Problems).contains("problem"));
    assert!(status_next(&Next::Wait).contains("goalcup status"));
    // The refusal with no run starts with the command and keeps the phrase tests read.
    let none = no_run_yet();
    assert!(none.starts_with("Run goalcup setup"), "{none}");
    assert!(
        none.contains("no run has been started from this machine"),
        "{none}"
    );
}

#[test]
fn upload_messages_start_with_the_next_action_and_name_it() {
    // A refusal says what to do first.
    let refusals = [
        upload_no_run(),
        upload_local_run("r1"),
        upload_not_opened("e1"),
        upload_not_here("e1"),
        upload_refused("https://s.example", "e1", "401"),
        upload_incomplete("e1", 10, "the server answered 503"),
        upload_not_sealed(),
    ];
    for said in &refusals {
        let first = said.split(' ').next().unwrap();
        assert!(
            ["Run", "Name", "Check", "Nothing"].contains(&first),
            "next action first: {said}"
        );
    }
    assert!(upload_no_run().contains("nothing to upload"));
    assert!(upload_local_run("r1").contains("is local"));
    assert!(upload_incomplete("e1", 10, "503").contains("goalcup upload"));
    assert!(upload_refused("https://s.example", "e1", "401").contains("https://s.example"));
    assert!(upload_waiting("busy", 5).contains("5s"));
    assert!(upload_finished().contains(NOTHING_MORE));
    assert!(upload_nothing_left("r1").contains("r1"));
    // Entries: the way to reach one run comes first.
    assert!(entries_intro(false)[0].contains("goalcup status <part-id>"));
    assert!(entries_intro(true)[0].contains("goalcup start"));
}

/// Whatever is left to upload is sent with one command, `goalcup upload`, never with
/// `goalcup continue`; a seal is asked for only when the part needs one, and then the
/// upload after it.
#[test]
fn a_leftover_upload_has_one_command() {
    let open = Ending::Open {
        run_id: "r1",
        how: "exit code 1",
        until: "Sat 14 Nov, 10:12 UTC".into(),
    };
    let mut said = vec![
        status_next(&Next::Upload),
        problem_upload("40 min", "503", true),
    ];
    for ending in [
        open,
        Ending::TimeUp { practice: false },
        Ending::Replaced,
        Ending::Over { how: "exit code 0" },
    ] {
        said.extend(after_upload(&ending, false, false));
        said.extend(after_upload(&ending, false, true));
    }
    for line in said.iter().filter(|line| line.contains("upload")) {
        assert!(line.ends_with("goalcup upload"), "{line}");
        assert!(!line.contains(CONTINUE_HINT), "{line}");
    }
    let seal = upload_seal_missing("p1");
    assert!(
        seal.starts_with("Run goalcup seal p1, then goalcup upload."),
        "{seal}"
    );
    // A part still recording needs neither: its own goalcup sends the rest.
    let recording = upload_not_sealed();
    assert!(!recording.contains("goalcup "), "{recording}");
    // The stale-state line leaves the command to the Next line, which knows whether a seal
    // is needed.
    assert!(!problem_state_stale("2 h", Restart::Over).contains("goalcup seal"));
}

/// A network problem line names a firewall too, and a refusal while a run is open never
/// advises a start, which would replace the run.
#[test]
fn network_lines_name_a_firewall_and_no_start_is_advised_while_a_run_is_open() {
    for line in [
        problem_upload("40 min", "503", false),
        problem_upload("40 min", "503", true),
        problem_heartbeats("15 min", "timed out"),
    ] {
        assert!(line.contains("Little Snitch"), "{line}");
    }
    let open = home_busy("A", true);
    assert!(!open.contains("run goalcup start"), "{open}");
    assert!(!open.starts_with("Start"), "{open}");
}

#[test]
fn a_problem_line_ends_with_its_fix() {
    let who = Who {
        harness: "Some Agent",
        run_id: "r1",
        until: Some("Sat 14 Nov, 10:12 UTC"),
        guide: "https://example.test/HARNESS-SETUP.md",
    };
    assert!(
        problem_agent_exited(
            &who,
            "exit code 1",
            false,
            Restart::Continue { run_id: "r1" }
        )
        .ends_with("goalcup continue r1")
    );
    assert!(
        problem_state_stale("12 min", Restart::Continue { run_id: "r1" })
            .ends_with("goalcup continue r1")
    );
    assert!(problem_upload("40 min", "503", true).ends_with("goalcup upload"));
    assert!(problem_upload("40 min", "503", false).contains("Check the network"));
    assert!(problem_heartbeats("15 min", "503").contains("15 min"));
    assert!(problem_disk("3.2 GiB").contains("Free some space"));
    assert!(problem_sleep("no inhibitor").contains("power"));
    assert!(problem_recording_failed("r1").contains(SUPPORT_ADDRESS));
    assert!(agent_finished("Some Agent", true).contains("run is over"));
}

#[test]
fn every_refusal_says_what_to_do_first_and_has_no_rule_code() {
    let tmp = Path::new("/w");
    let refusals = [
        home_busy("Some Agent", true),
        home_busy("Some Agent", false),
        still_recording("Some Agent", "r1"),
        needs_terminal("Some Agent", "Nothing was created."),
        inside_work_folder(tmp, &tmp.join("x")),
        not_started_here(Some("r1")),
        not_started_here(None),
        local_only("r1"),
        run_over("r1"),
        last_part_closing("r1"),
        continued_elsewhere("r1", "e2", "e1"),
        approval_denied(),
        approval_expired(),
        approval_interrupted(),
        server_unreachable("https://s.example", "refused"),
    ];
    let verbs = [
        "Your", "Use", "Run", "Start", "Nothing", "Try", "Continue", "Check",
    ];
    for refusal in refusals {
        let first = refusal.split(' ').next().unwrap();
        assert!(verbs.contains(&first), "{refusal}");
        assert!(
            !refusal
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|w| w.len() >= 2
                    && (w.starts_with('R') || w.starts_with('D'))
                    && w[1..].chars().all(|c| c.is_ascii_digit())),
            "a rule code: {refusal}"
        );
        assert!(refusal.ends_with('.'), "{refusal}");
    }
    assert!(still_recording("A", "r1").contains("run r1 is still recording"));
    assert!(home_busy("A", false).contains("already recording"));
    // A second start by mistake is told nothing is needed first; exiting the agent to start
    // again comes last.
    let busy = home_busy("A", false);
    assert!(busy.starts_with("Your run is already going"), "{busy}");
    assert!(busy.find("Nothing to do").unwrap() < busy.find("exit A").unwrap());
    // "This replaces your run" is true only for a competition start that would replace an
    // open competition run.
    assert!(!busy.contains("replaces"), "{busy}");
    assert!(home_busy("A", true).contains("replaces your current competition run"));
    assert!(needs_terminal("A", "").contains("foreground of a terminal"));
    assert!(not_started_here(None).contains(NOT_STARTED_HERE));
}
