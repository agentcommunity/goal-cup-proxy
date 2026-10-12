use goalcup_verifier::atif::{
    Agent, Extra, GoalcupExtra, Message, Observation, ObservationResult, Provenance, RawLink,
    SCHEMA_VERSION, Step, StepSource, ToolCall, Trajectory,
};

fn sample() -> Trajectory {
    let mut t = Trajectory::new(Agent {
        name: "claude-code-cli".into(),
        version: "2.1.259".into(),
        model_name: Some("claude-opus-5".into()),
        tool_definitions: vec!["Bash".into(), "Agent".into()],
        extra: None,
    });
    t.session_id = Some("95addb64-9dac-4046-8ef6-8aa53fa0c7f1".into());

    t.push(Step {
        step_id: 0,
        timestamp: Some("2026-09-03T07:42:00.446Z".into()),
        source: StepSource::User,
        model_name: None,
        message: Message::Text(
            "create hello.py that prints the current date, run it, then tell me the output".into(),
        ),
        reasoning_content: None,
        tool_calls: vec![],
        observation: None,
        metrics: None,
        llm_call_count: None,
        is_copied_context: None,
        extra: Some(Extra {
            goalcup: GoalcupExtra {
                raw: vec![RawLink {
                    store: "main".into(),
                    seq: 6,
                    sha256: "ab".repeat(32),
                }],
                provenance: Some(Provenance {
                    origin: Some("human".into()),
                    source: Some("typed".into()),
                    pair_key: Some("58740b59-3dfe-46fb-8704-7d049d21a7ed".into()),
                }),
            },
        }),
    });

    t.push(Step {
        step_id: 0,
        timestamp: Some("2026-09-03T07:42:03.774Z".into()),
        source: StepSource::Agent,
        model_name: Some("claude-opus-5".into()),
        message: Message::Text("I'll create hello.py.".into()),
        reasoning_content: None,
        tool_calls: vec![ToolCall {
            tool_call_id: "toolu_01GYp4NwUCxtcSchG1dYWYia".into(),
            function_name: "Bash".into(),
            arguments: serde_json::json!({"command": "python3 hello.py"}),
            extra: None,
        }],
        observation: Some(Observation {
            results: vec![ObservationResult {
                source_call_id: Some("toolu_01GYp4NwUCxtcSchG1dYWYia".into()),
                content: Some(Message::Text("2026-09-03".into())),
                subagent_trajectory_ref: vec![],
                extra: None,
            }],
        }),
        metrics: None,
        llm_call_count: Some(1),
        is_copied_context: None,
        extra: None,
    });
    t
}

#[test]
fn step_ids_are_assigned_from_one() {
    let t = sample();
    assert_eq!(t.steps[0].step_id, 1);
    assert_eq!(t.steps[1].step_id, 2);
    assert_eq!(t.schema_version, SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, "ATIF-v1.8");
}

#[test]
fn writer_is_deterministic_and_omits_empty_optionals() {
    let a = sample().to_json().unwrap();
    let b = sample().to_json().unwrap();
    assert_eq!(
        a, b,
        "two writes of the same trajectory must be byte-identical"
    );
    assert!(
        !a.contains("\"notes\""),
        "None optionals are omitted, got {a}"
    );
    assert!(
        !a.contains("\"subagent_trajectories\""),
        "empty vectors are omitted"
    );
    assert!(a.contains("\"schema_version\":\"ATIF-v1.8\""));
    assert!(a.contains("\"source\":\"user\""));
    assert!(a.contains("\"source\":\"agent\""));
    assert!(a.contains("\"tool_call_id\":\"toolu_01GYp4NwUCxtcSchG1dYWYia\""));
    assert!(a.contains("\"source_call_id\":\"toolu_01GYp4NwUCxtcSchG1dYWYia\""));
}

#[test]
fn raw_links_and_provenance_ride_in_extra_only() {
    let json: serde_json::Value = serde_json::from_str(&sample().to_json().unwrap()).unwrap();
    let step = &json["steps"][0];
    assert_eq!(step["extra"]["goalcup"]["raw"][0]["store"], "main");
    assert_eq!(step["extra"]["goalcup"]["raw"][0]["seq"], 6);
    assert_eq!(step["extra"]["goalcup"]["provenance"]["origin"], "human");
    assert!(
        step.get("origin").is_none(),
        "provenance must not leak outside extra"
    );
    assert!(step.get("goalcup").is_none());
}

#[test]
fn round_trips_through_serde() {
    let t = sample();
    let text = t.to_json().unwrap();
    let back: Trajectory = serde_json::from_str(&text).unwrap();
    assert_eq!(back.to_json().unwrap(), text);
}
