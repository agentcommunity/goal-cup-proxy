//! `capture.model_host_from` (D62): the model service the harness's own config names is the one
//! host a run records, read at the start and again at each continue; every other host stays an
//! opaque tunnel; and a value Goal Cup cannot record is refused before anything is created.
//!
//! Fixture connectors only: the rig's headless `m2-fake` and chat-mode `m2-fake-tui`, each with
//! its `[[capture.hosts]]` row taken out and `model_host_from` put in. The stand-in model hosts
//! are `*.example.test` names that the recording proxy connects to a local upstream through a test
//! build's address table, so nothing leaves this machine.

use crate::common::m2_cli_support::{CHAT_ID, FAKE_ID, Hooked, Launcher, Rig, Upstream};
use crate::common::support;
use crate::common::upload_server::{ServerOptions, UploadServer};
use goalcup_protocol::capture::{CaptureRecord, CaptureSource};
use goalcup_protocol::harness::{CAPTURE_HOMES_DIR, ModelHostFrom, Origin};

const RESERVE: u64 = 4 * 1024 * 1024;
const MODEL_HOST: &str = "models.example.test";
const OTHER_HOST: &str = "github.example.test";
/// The refusal's advice, after the reason.
const CANNOT_RECORD: &str = "Goal Cup records only an https model service that is not on this \
                             computer.";

/// Make the rig connector `id` read its model host from `config.yaml` `model.base_url` and list
/// no host of its own. Returns the field as loaded.
pub(crate) fn reads_model_host(rig: &Rig, id: &str) -> ModelHostFrom {
    let path = rig.connectors.join(id).join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let at = text
        .find("[[capture.hosts]]")
        .expect("the rig connector lists a host");
    let text = text[..at].replacen(
        "[capture]\n",
        "[capture]\nmodel_host_from = { file = \"config.yaml\", key = \"model.base_url\" }\n",
        1,
    );
    std::fs::write(&path, text).unwrap();
    goalcup_protocol::harness::load_dir(&rig.connectors.join(id), Origin::Dev)
        .unwrap()
        .manifest
        .capture
        .model_host_from
        .expect("the field loads")
}

/// Give the rig connector `id` one default host of its own, `host`, as a shipped connector lists:
/// the host the harness uses while its settings name no service. [`reads_model_host`] takes the
/// connector's hosts away, so a test that wants one adds it back after.
pub(crate) fn add_default_host(rig: &Rig, id: &str, host: &str) {
    let path = rig.connectors.join(id).join("harness.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "\n[[capture.hosts]]\nhost = \"{host}\"\nevidence_paths = [\"/v1/responses\"]\n\
         protocol_hint = \"openai-responses\"\n"
    ));
    std::fs::write(&path, text).unwrap();
}

fn home(rig: &Rig, id: &str) -> std::path::PathBuf {
    rig.cfg.join(CAPTURE_HOMES_DIR).join(id)
}

/// Held by each [`headless_run`] for the whole run. The recording proxy's address table is one
/// per process and maps each stand-in name to one upstream, so under plain `cargo test` (every
/// test a thread of one process) a second run pointed the first run's names at its own
/// upstream, whose certificate the first run's harness does not trust.
static ADDRESS_TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One headless run of the fake that reads its model host from `https://<MODEL_HOST>/v1`, with
/// `steps` as what the harness does. The upstream answers for both stand-in names.
fn headless_run(steps: impl FnOnce(&Upstream) -> String) -> (Rig, String, Vec<CaptureRecord>) {
    let _table = ADDRESS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let rig = Rig::new();
    let from = reads_model_host(&rig, FAKE_ID);
    support::write_model_host_config(
        &home(&rig, FAKE_ID),
        &from,
        &format!("https://{MODEL_HOST}/v1"),
    );
    let upstream = Upstream::start_named(&rig.runtime, &[MODEL_HOST, OTHER_HOST]);
    let local = std::net::SocketAddr::from(([127, 0, 0, 1], upstream.port));
    for host in [MODEL_HOST, OTHER_HOST] {
        goalcup_cli::proxy::connect::resolve_for_test(host, local);
    }
    rig.tell("upstream.pem", &upstream.root_pem());
    rig.tell("stop", "");
    rig.tell("steps.sh", &steps(&upstream));
    let goal = rig.run_folder();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = std::thread::scope(|s| {
        rig.start(
            s,
            &client,
            rig.options(&goal, RESERVE),
            Some(upstream.roots()),
        )
        .join()
        .unwrap()
    });
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    (rig, out, records)
}

/// A request through the recording proxy the harness was given, to `url`, trusting `ca` (a variable
/// or a file), with its HTTP status in `$M2_FAKE_OUT/<name>`.
fn curl(url: &str, ca: &str, name: &str) -> String {
    format!(
        "curl -sS --http1.1 --max-time 10 --noproxy '' --proxy \"$HTTPS_PROXY\" --cacert {ca} \
         -H 'content-type: application/json' --data '{{\"model\":\"m\"}}' -o /dev/null \
         -w '%{{http_code}}' {url} > \"$M2_FAKE_OUT/{name}\" 2>>\"$M2_FAKE_OUT/curl.err\"\n"
    )
}

/// Every wire `Open` record's host and path.
fn wire_opens(records: &[CaptureRecord]) -> Vec<(String, String, Option<String>)> {
    records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Open {
                source: CaptureSource::WireRequest,
                metadata,
                ..
            } => Some((
                metadata.host.clone().unwrap_or_default(),
                metadata.path.clone().unwrap_or_default(),
                metadata.protocol_hint.clone(),
            )),
            _ => None,
        })
        .collect()
}

/// The config names `https://models.example.test/v1`: the run approves
/// `models.example.test:443`, and a model call to it is decrypted and becomes a chain record
/// with the chat-completions hint.
#[test]
fn the_model_host_from_the_harness_config_is_recorded() {
    let (rig, out, records) = headless_run(|_| {
        curl(
            &format!("https://{MODEL_HOST}/v1/chat/completions"),
            "\"$CODEX_CA_CERTIFICATE\"",
            "model.status",
        )
    });
    assert!(
        out.contains(&format!(
            "model service: {MODEL_HOST}:443 (model.base_url in config.yaml)"
        )),
        "{out}"
    );
    assert_eq!(
        rig.harness_file("curl.err").unwrap_or_default(),
        "",
        "the harness trusted the recording proxy's leaf for the model host"
    );
    assert_eq!(rig.harness_file("model.status").as_deref(), Some("200"));
    assert!(
        wire_opens(&records).contains(&(
            MODEL_HOST.to_string(),
            "/v1/chat/completions".to_string(),
            Some("openai-chat-completions".to_string())
        )),
        "{:?}",
        wire_opens(&records)
    );
}

/// A CONNECT to any other host is relayed as a tunnel the recording proxy never opens: the harness
/// sees the upstream's own certificate, the tunnel is counted in `proxy admission:`, the chain
/// holds no record of it, and the entry's sealed-connection list names its host and bytes.
#[test]
fn other_hosts_stay_opaque_tunnels() {
    let (rig, out, records) = headless_run(|_| {
        format!(
            "{}{}",
            curl(
                &format!("https://{MODEL_HOST}/v1/chat/completions"),
                "\"$CODEX_CA_CERTIFICATE\"",
                "model.status",
            ),
            curl(
                &format!("https://{OTHER_HOST}/repos/x"),
                "\"$M2_FAKE_OUT/upstream.pem\"",
                "other.status",
            ),
        )
    });
    assert_eq!(rig.harness_file("curl.err").unwrap_or_default(), "");
    assert_eq!(
        rig.harness_file("other.status").as_deref(),
        Some("200"),
        "the other host answered with its own certificate, unread"
    );
    let admission = out
        .lines()
        .find(|line| line.starts_with("proxy admission:"))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(
        admission.contains(" tunneled_not_approved=1"),
        "{admission}"
    );
    let opens = wire_opens(&records);
    assert!(
        opens.iter().any(|(host, ..)| host == MODEL_HOST),
        "{opens:?}"
    );
    assert!(
        !opens.iter().any(|(host, ..)| host == OTHER_HOST),
        "{opens:?}"
    );
    // The tunnel is named only in the sealed-connection list and its one finish line (D74).
    let sealed = out
        .lines()
        .find(|line| line.starts_with("sealed hosts: "))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(
        sealed.contains(&format!("{OTHER_HOST}:443 x1 ")),
        "{sealed}"
    );
    assert_eq!(out.matches(OTHER_HOST).count(), 1, "{out}");
    let lists: Vec<_> = walk(&goalcup_cli::run::runs_dir(&rig.cfg))
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("tunnels-"))
        })
        .collect();
    let [list] = lists.as_slice() else {
        panic!("one list per entry: {lists:?}")
    };
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(list)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["format"], "goalcup-tunnels-v1");
    let other = lines
        .iter()
        .find(|line| line["host"] == OTHER_HOST)
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert_eq!(other["reason"], "not_model_host");
    assert!(other["up_bytes"].as_u64().unwrap() > 0, "{other}");
}

/// Every file under `dir`.
fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// `absent = "defaults"` with no config at all: the start is not refused, the details say the
/// connector's own hosts are recorded, and its host row is opened as it is without the field.
#[test]
fn a_missing_setting_records_the_connector_hosts() {
    let rig = Rig::new();
    let path = rig.connectors.join(FAKE_ID).join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap().replacen(
        "[capture]\n",
        "[capture]\nmodel_host_from = { file = \"config.yaml\", key = \"model.base_url\", \
         absent = \"defaults\" }\n",
        1,
    );
    std::fs::write(&path, text).unwrap();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("stop", "");
    rig.tell("steps.sh", &upstream.curl(r#"{"n":1}"#));
    let goal = rig.run_folder();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = std::thread::scope(|s| {
        rig.start(
            s,
            &client,
            rig.options(&goal, RESERVE),
            Some(upstream.roots()),
        )
        .join()
        .unwrap()
    });
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        out.contains(
            "model service: the connector's default hosts (`model.base_url` not set in \
             config.yaml)"
        ),
        "{out}"
    );
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    let opens = wire_opens(&records);
    assert!(
        opens.iter().any(|(host, ..)| host == "localhost"),
        "{opens:?}"
    );
}

/// The shipped Claude Code and Codex connectors read a gateway from the participant's own
/// settings and, with none set (a subscription login, Codex's built-in provider), are not
/// refused: the run records their default hosts, `api.anthropic.com`, `chatgpt.com` (Codex's
/// ChatGPT login) and `api.openai.com` (Codex's API-key login, `/v1/responses`).
#[test]
fn the_shipped_claude_and_codex_rows_fall_back_without_a_setting() {
    let cfg = tempfile::tempdir().unwrap();
    let claude = support::connector("claude-code-cli");
    let codex = support::connector("codex-cli");
    let hosts = |c: &goalcup_cli::connector::HarnessConnector| -> Vec<String> {
        let hosts = &c.loaded.manifest.capture.hosts;
        hosts.iter().map(|h| h.host.clone()).collect()
    };
    assert_eq!(hosts(&claude), ["api.anthropic.com"]);
    assert_eq!(hosts(&codex), ["chatgpt.com", "api.openai.com"]);
    // The API-key row approves exactly the Responses path, with its decoder hint, as the run's
    // host rule does (`approved_hosts` turns `evidence_paths` into exact rules with this hint).
    let api_key_row = codex
        .loaded
        .manifest
        .capture
        .hosts
        .iter()
        .find(|h| h.host == "api.openai.com")
        .expect("the Codex API-key row");
    assert_eq!(api_key_row.evidence_paths, ["/v1/responses"]);
    assert!(api_key_row.prefix_paths.is_empty(), "{api_key_row:?}");
    assert_eq!(
        api_key_row.protocol_hint.as_deref(),
        Some("openai-responses")
    );
    let check = |c| goalcup_cli::model_host::check(cfg.path(), c);
    assert_eq!(check(&claude), Ok(None));
    assert_eq!(check(&codex), Ok(None));

    let home = |id: &str| {
        let home = cfg.path().join(CAPTURE_HOMES_DIR).join(id);
        std::fs::create_dir_all(&home).unwrap();
        home
    };
    let codex_config = home("codex-cli").join("config.toml");
    std::fs::write(&codex_config, "model_provider = \"openai\"\n").unwrap();
    assert_eq!(check(&codex), Ok(None), "a built-in provider has no table");

    std::fs::write(
        home("claude-code-cli").join("settings.json"),
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://ai-gateway.vercel.sh/claude-code"}}"#,
    )
    .unwrap();
    let m = check(&claude).unwrap().expect("the gateway is read");
    assert_eq!(
        (m.host.as_str(), m.base_path.as_str()),
        ("ai-gateway.vercel.sh", "/claude-code")
    );
    std::fs::write(
        &codex_config,
        "model_provider = \"vercel\"\n[model_providers.vercel]\n\
         base_url = \"https://ai-gateway.vercel.sh/codex/v1\"\nwire_api = \"responses\"\n",
    )
    .unwrap();
    let m = check(&codex).unwrap().expect("the gateway is read");
    assert_eq!(
        (m.host.as_str(), m.base_path.as_str()),
        ("ai-gateway.vercel.sh", "/codex/v1")
    );
}

/// No config, no key, a loopback or address-literal service, plain `http`, an authentication
/// host or a value that is not a URL: each start is refused with the one sentence, which shows
/// at most `scheme://host:port`, and nothing is created.
#[test]
fn a_missing_or_loopback_model_host_is_refused_at_start() {
    let rig = Rig::new();
    let from = reads_model_host(&rig, FAKE_ID);
    let home = home(&rig, FAKE_ID);
    let goal = rig.run_folder();
    let http = rig.client();
    let refused = |shown: &str, reason: &str, hidden: Option<&str>| {
        let (result, out) = std::thread::scope(|s| {
            rig.start(s, &http, rig.options(&goal, RESERVE), None)
                .join()
                .unwrap()
        });
        let error = result.expect_err(&out).to_string();
        assert!(
            error.contains(&format!(
                "{FAKE_ID} is set to use {shown}, which Goal Cup cannot record: {reason}"
            )),
            "{error}"
        );
        assert!(error.contains(CANNOT_RECORD), "{error}");
        assert!(
            error.contains(&format!("Run goalcup setup --harness {FAKE_ID}")),
            "{error}"
        );
        if let Some(hidden) = hidden {
            assert!(!error.contains(hidden), "{error}");
        }
        assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
        assert!(rig.harness_file("pid").is_none(), "no harness ran");
    };

    let unset = "`model.base_url` in config.yaml is not set";
    refused("no model service", unset, None);
    support::write_model_host_config(&home, &from, "");
    refused("no model service", unset, None);
    std::fs::write(home.join("config.yaml"), "model:\n  provider: custom\n").unwrap();
    refused("no model service", unset, None);
    const HTTP: &str = "it is plain http";
    const LOCAL: &str = "it is on this computer or the local network";
    const IP: &str = "it is an IP address";
    for (value, shown, reason, hidden) in [
        (
            "http://localhost:11434/v1",
            "http://localhost:11434",
            HTTP,
            None,
        ),
        ("https://localhost/v1", "https://localhost:443", LOCAL, None),
        (
            "https://ollama:11434/v1",
            "https://ollama:11434",
            LOCAL,
            None,
        ),
        (
            "https://127.0.0.1:8443/v1",
            "https://127.0.0.1:8443",
            LOCAL,
            None,
        ),
        ("https://[::1]:8080/v1", "https://[::1]:8080", LOCAL, None),
        (
            "https://203.0.113.7/v1",
            "https://203.0.113.7:443",
            IP,
            None,
        ),
        (
            "https://user:hunter2@llm.localhost:8443/v1?key=abc",
            "https://llm.localhost:8443",
            LOCAL,
            Some("hunter2"),
        ),
        (
            "http://models.example.test/v1",
            "http://models.example.test:80",
            HTTP,
            None,
        ),
        (
            "https://auth.openai.com/v1",
            "https://auth.openai.com:443",
            "it is a sign-in host",
            None,
        ),
        (
            "sk-not-a-url-0123456789",
            "an address Goal Cup cannot read",
            "`model.base_url` in config.yaml is not an http or https URL",
            Some("sk-not"),
        ),
    ] {
        support::write_model_host_config(&home, &from, value);
        refused(shown, reason, hidden);
    }
}

/// A participant who switched model service between segments: `goalcup continue` reads the
/// config again, and the new segment approves the new host.
#[test]
fn continue_rereads_the_model_host() {
    const OFFLINE: (&str, &str) = (goalcup_cli::commands::start::TEST_OFFLINE_UPSTREAMS, "1");
    const DISK: (&str, &str) = ("GOALCUP_TEST_DISK", "4194304,0,50");
    const BEATING: (&str, &str) = (
        "GOALCUP_TEST_UPLOAD_POLICY",
        "200,25,100,400,10000,268435456,3000,100",
    );
    let rig = Rig::chat();
    let from = reads_model_host(&rig, CHAT_ID);
    let home = home(&rig, CHAT_ID);
    support::write_model_host_config(&home, &from, "https://first.example.test/v1");
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());

    // Each segment's harness keeps the proxy CA it was handed. The CA is minted for the
    // segment's approved hosts and names them in its name constraints, so its DER holds the
    // approved host's name, and no other host is approved.
    let segment = |mut run: crate::common::m2_cli_support::OnPty| -> Vec<u8> {
        run.wait_shown("tui: ready");
        run.type_line("/exit");
        let status = run.wait();
        assert!(status.success(), "{status:?}\n{}", run.shown());
        let pem = rig
            .harness_file("tui.ca.pem")
            .expect("the harness was handed the proxy CA");
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap()
    };
    let names = |der: &[u8], host: &str| der.windows(host.len()).any(|w| w == host.as_bytes());
    let ca = segment(crate::common::m2_cli_support::OnPty::spawn(
        rig.chat_command(&[OFFLINE, DISK, BEATING], Launcher::Direct),
        None,
    ));
    assert!(
        names(&ca, "first.example.test"),
        "segment 0 approves the first host"
    );
    support::write_model_host_config(&home, &from, "https://second.example.test/api");
    let ca = segment(rig.continue_on_pty(&[OFFLINE, DISK, BEATING], None));
    assert!(
        names(&ca, "second.example.test") && !names(&ca, "first.example.test"),
        "segment 1 approves the new host, and not the old one"
    );

    let run_id = &server.run_ids()[0];
    assert_eq!(server.entry_ids(run_id).len(), 2, "two segments");
    let log = std::fs::read_to_string(
        goalcup_cli::run::run_dir(&rig.cfg, run_id).join(goalcup_cli::host_log::HOST_LOG_NAME),
    )
    .unwrap();
    let first = log
        .find("model service: first.example.test:443")
        .unwrap_or_else(|| panic!("{log}"));
    let second = log
        .find("model service: second.example.test:443")
        .unwrap_or_else(|| panic!("{log}"));
    assert!(first < second, "{log}");
    assert_eq!(
        log.matches("model service:").count(),
        2,
        "each segment names the one host it approved: {log}"
    );
    assert!(
        !log.contains("/api"),
        "only the host and port are logged: {log}"
    );
}

/// The real Hermes, set up as a participant's own setup leaves it for DeepSeek
/// (`model.base_url: https://api.deepseek.com/v1` in `config.yaml`, a made-up key in `.env`), run
/// by `goalcup start` on a terminal. The connector lists no host: the one it intercepts is the
/// one Hermes's config names, so its model call is decrypted and recorded (DeepSeek answers the
/// made-up key with 401). Reaches the network; `~/.hermes` is not touched.
///
/// `GOALCUP_TEST_HERMES_VENV` names the Hermes Agent 0.19.0 virtual environment (the folder with
/// `bin/python` and `bin/hermes`); Hermes runs with that venv's own Python.
#[test]
#[ignore = "runs the real installed Hermes against the real DeepSeek host; run by name"]
fn the_real_hermes_model_call_to_its_configured_service_is_recorded() {
    use std::os::unix::fs::PermissionsExt;
    let venv = std::path::PathBuf::from(
        std::env::var_os("GOALCUP_TEST_HERMES_VENV").expect("GOALCUP_TEST_HERMES_VENV"),
    );
    const ID: &str = "hermes-cli";
    let rig = Rig::chat();
    support::copy_tree(
        &support::connectors_dir().join(ID),
        &rig.connectors.join(ID),
    );
    let program = rig.root.join("bin").join("hermes");
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\nexec '{}' '{}' \"$@\"\n",
            venv.join("bin/python").display(),
            venv.join("bin/hermes").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let from = goalcup_protocol::harness::load_dir(&rig.connectors.join(ID), Origin::Pinned)
        .unwrap()
        .manifest
        .capture
        .model_host_from
        .expect("Hermes reads its model host");
    let home = home(&rig, ID);
    support::write_model_host_config(&home, &from, "https://api.deepseek.com/v1");
    std::fs::write(
        home.join("config.yaml"),
        "model:\n  default: deepseek-chat\n  provider: deepseek\n  base_url: https://api.deepseek.com/v1\n",
    )
    .unwrap();
    std::fs::write(
        home.join(".env"),
        "DEEPSEEK_API_KEY=sk-00000000000000000000000000000000\n",
    )
    .unwrap();
    std::fs::set_permissions(home.join(".env"), std::fs::Permissions::from_mode(0o600)).unwrap();

    let mut run = crate::common::m2_cli_support::OnPty::spawn(
        rig.chat_command_for(
            ID,
            &[
                ("GOALCUP_TEST_DISK", "4194304,0,50"),
                ("GOALCUP_TEST_LOCAL_WRAP_UP", "2000,100"),
            ],
            Launcher::Direct,
        ),
        None,
    );
    let sessions = home.join("sessions");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let dumped = || {
        std::fs::read_dir(&sessions).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("request_dump_"))
        })
    };
    // Hermes's banner (`Session: <id>`), then one message; the failed call leaves a dump.
    while !run.shown().contains("Session") {
        if let Ok(Some(status)) = run.child.try_wait() {
            panic!(
                "goalcup ended before Hermes opened: {status:?}\n{}",
                run.shown()
            );
        }
        assert!(std::time::Instant::now() < deadline, "{}", run.shown());
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    run.type_line("hello");
    while !dumped() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    run.type_line("/exit");
    let status = run.wait();
    let shown = run.shown();
    assert!(status.success(), "{status:?}\n{shown}");

    let entry = goalcup_cli::entries::latest_entry(&rig.cfg)
        .unwrap()
        .expect("an entry was recorded")
        .entry_id;
    let (_, records) = rig.read_trace(&entry);
    let opens = wire_opens(&records);
    eprintln!("wire opens: {opens:?}");
    let statuses: Vec<Option<u16>> = records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Open {
                source: CaptureSource::WireResponse,
                metadata,
                ..
            } => Some(metadata.status),
            _ => None,
        })
        .collect();
    eprintln!("response statuses: {statuses:?}");
    let log = std::fs::read_to_string(
        goalcup_cli::run::run_dir(
            &rig.cfg,
            &goalcup_cli::run::newest(&rig.cfg).unwrap().unwrap().run_id,
        )
        .join(goalcup_cli::host_log::HOST_LOG_NAME),
    )
    .unwrap_or_default();
    eprintln!(
        "host.log model line: {:?}",
        log.lines().find(|l| l.contains("model service"))
    );
    eprintln!(
        "host.log admission: {:?}",
        log.lines().find(|l| l.contains("proxy admission"))
    );
    assert!(
        opens.contains(&(
            "api.deepseek.com".to_string(),
            "/v1/chat/completions".to_string(),
            Some("openai-chat-completions".to_string())
        )),
        "{opens:?}\n{shown}"
    );
}
