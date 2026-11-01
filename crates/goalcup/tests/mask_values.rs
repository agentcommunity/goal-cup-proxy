//! Exact values (PRD-05 section 9, sources 1 to 3): the project's `GH_TOKEN` value is masked in
//! all 11 forms, wrapped or not, whole and streamed, and so are the credential-named values of
//! the project's key files and of the harness environment, behind the name gate. Every value is
//! fake: built from pieces or drawn from a seeded generator.

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use common::{ALNUM, Rng, starred, stream_in};
use goalcup::mask::{
    Masker, SecretValue, ValueSource, env_values, launch_values, workspace_values,
};
use goalcup_format::StreamKind;
use std::collections::BTreeMap;
use std::ops::Range;

type Res = Result<(), Box<dyn std::error::Error>>;

/// A fake token with no known shape, so only the exact-value rule can mask it.
fn token() -> String {
    ["Vq7", "Lm2", "-x.Y~", "Zk4Rt9Wp3Ns", "+Hd/8Qe=", "Jc5"].concat()
}

fn masker(value: &str) -> Result<Masker, goalcup::mask::MaskError> {
    Masker::new(
        &[],
        vec![SecretValue::new(
            ValueSource::Workspace,
            value.as_bytes().to_vec(),
        )],
    )
}

/// `form` between `lead` and `tail`, and where it sits.
fn around(lead: &str, form: &str, tail: &str) -> (Vec<u8>, Range<usize>) {
    let at = lead.len();
    (
        format!("{lead}{form}{tail}").into_bytes(),
        at..at + form.len(),
    )
}

/// The 11 forms of `v`, each inside some text a harness might write, with the range of the
/// form: the bytes that depend on `v` alone.
fn texts(v: &str) -> Vec<(Vec<u8>, Range<usize>)> {
    let hex = |upper: bool, escape: bool| -> String {
        v.bytes()
            .map(|b| match (upper, escape) {
                (true, false) => format!("%{b:02X}"),
                (false, false) => format!("%{b:02x}"),
                (true, true) => format!("\\u00{b:02X}"),
                (false, true) => format!("\\u00{b:02x}"),
            })
            .collect()
    };
    let mut out = vec![
        around("GH_TOKEN=", v, "\n"),
        around("url?t=", &hex(true, false), "&x=1"),
        around("url?t=", &hex(false, false), "&x=1"),
        around("{\"t\":\"", &hex(true, true), "\"}"),
        around("{\"t\":\"", &hex(false, true), "\"}"),
    ];
    for lead in 0..3 {
        let mut plain = b"env"[..lead].to_vec();
        plain.extend(format!("{v} and more text").bytes());
        // Character `i` holds bits `6i..6i + 6`: it depends on `v` alone when both its bytes do.
        let (first, past) = ((lead * 8).div_ceil(6), (lead + v.len()) * 8 / 6);
        for text in [
            STANDARD_NO_PAD.encode(&plain),
            URL_SAFE_NO_PAD.encode(&plain),
        ] {
            let (input, _) = around("b64 ", &text, " end");
            out.push((input, 4 + first..4 + past));
        }
    }
    out
}

#[test]
fn the_gh_token_is_masked_in_every_form_whole_and_streamed() -> Res {
    let v = token();
    let m = masker(&v)?;
    assert_eq!(m.value_counts().workspace, 1);
    let rules = m.mask_rules();
    let mut inputs = texts(&v);
    for brk in ["\n", "\r\n", "\\n", "\\\\n", "\\r\\n"] {
        // `v` with the break inside after its fifth byte.
        let form = format!("{}{brk}{}", &v[..5], &v[5..]);
        inputs.push(around("{\"out\":\"", &form, "\"}"));
    }
    for (input, form) in &inputs {
        let shown = String::from_utf8_lossy(input);
        let whole = m.mask_whole(input);
        assert_eq!(whole.bytes.len(), input.len());
        assert!(starred(&whole, form.clone()), "{shown:?}");
        assert!(
            whole.spans.iter().any(|s| rules[s.rule] == "run_secret"),
            "{shown:?}"
        );
        for split in 0..=input.len() {
            let got = stream_in(&m, StreamKind::Native, input, &[split]);
            assert_eq!(got, whole, "split {split}");
        }
    }
    Ok(())
}

#[test]
fn workspace_values_reads_the_last_gh_token_of_the_first_key_file() -> Res {
    let dir = tempfile::tempdir()?;
    let a = ["Aa1", &"b".repeat(30)].concat();
    let c = ["Cc3", &"d".repeat(30)].concat();
    std::fs::write(dir.path().join(".env"), format!("GH_TOKEN={a}\n"))?;
    std::fs::write(
        dir.path().join(".env.local"),
        format!(
            "export GH_TOKEN=\"{}\"\nOTHER=1\n  GH_TOKEN = '{c}'\n",
            "e".repeat(25)
        ),
    )?;
    std::fs::write(dir.path().join("notes.txt"), format!("GH_TOKEN={a}\n"))?;
    let values = workspace_values(dir.path());
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].source(), ValueSource::Workspace);
    let m = Masker::new(&[], values)?;
    assert!(m.mask_whole(c.as_bytes()).bytes.iter().all(|&b| b == b'*'));
    assert_eq!(m.mask_whole(a.as_bytes()).bytes, a.as_bytes());

    std::fs::remove_file(dir.path().join(".env.local"))?;
    let only_env = workspace_values(dir.path());
    let m = Masker::new(&[], only_env)?;
    assert!(m.mask_whole(a.as_bytes()).bytes.iter().all(|&b| b == b'*'));

    std::fs::write(dir.path().join(".env.local"), "GH_TOKEN=short_value_19c\n")?;
    assert!(
        workspace_values(dir.path()).is_empty(),
        "a value under 20 bytes is not used"
    );
    assert!(workspace_values(&dir.path().join("missing")).is_empty());
    Ok(())
}

/// 32 values (the guard; the 33rd is skipped) do not make a stream with many
/// backslashes fail closed: every `\u` form starts on a backslash, and all forms of one start
/// are one candidate. The stream equals `mask_whole`, and a planted escaped form is masked.
#[test]
fn thirty_two_values_stream_escaped_text_without_failing_closed() -> Res {
    let mut rng = Rng(0xF2F2);
    let values: Vec<String> = (0..33).map(|_| rng.chars(ALNUM, 24)).collect();
    let m = Masker::new(
        &[],
        values
            .iter()
            .map(|v| SecretValue::new(ValueSource::Workspace, v.clone().into_bytes()))
            .collect(),
    )?;
    let counts = m.value_counts();
    assert_eq!((counts.workspace, counts.skipped), (32, 1));
    let escaped: String = values[7].bytes().map(|b| format!("\\u00{b:02x}")).collect();
    let mut input = String::new();
    while input.len() < 64 * 1024 {
        input.push_str(r#"{"text":"caf\u00e9 \"quoted\" \\ path\\to\nnext \t tab"}"#);
        input.push('\n');
    }
    input.push_str(&format!("{{\"t\":\"{escaped}\"}}\n"));
    let input = input.into_bytes();
    let whole = m.mask_whole(&input);
    let at = input.len() - escaped.len() - 3;
    assert!(starred(&whole, at..at + escaped.len()));
    let cuts: Vec<usize> = (1..input.len() / 1000).map(|i| i * 1000).collect();
    let got = stream_in(&m, StreamKind::Native, &input, &cuts);
    assert_eq!(got, whole);
    let fail_closed = m.mask_rules().len() - 1;
    assert!(got.spans.iter().all(|s| s.rule != fail_closed));
    Ok(())
}

/// Whether `value` appears in `bytes`.
fn holds(bytes: &[u8], value: &str) -> bool {
    bytes.windows(value.len()).any(|w| w == value.as_bytes())
}

/// The owner's case: `cat .env` with values no shape catches, and a password that also sits in
/// a printed `.git/config` URL, masked by exact value raw and JSON-escaped, whole and streamed.
#[test]
fn the_owners_cat_env_case_is_masked_by_exact_value() -> Res {
    let mut rng = Rng(0x0E7);
    let aws = rng.chars(ALNUM, 40);
    let encryption = rng.chars(ALNUM, 32);
    let stripe = ["sk_", "test_", &rng.chars(ALNUM, 24)].concat();
    let git = rng.chars(ALNUM, 24);
    let env_file = format!(
        "# keys\nAWS_SECRET_ACCESS_KEY={aws}\nexport ENCRYPTION_KEY=\"{encryption}\"\n\
         STRIPE_SECRET_KEY={stripe}\nGIT_PASSWORD='{git}'\nMODEL_NAME=some-model-name-2026-preview\n"
    );
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(".env"), &env_file)?;
    let m = Masker::new(&[], launch_values(dir.path(), &BTreeMap::new()))?;
    assert_eq!(m.value_counts().workspace, 4);
    let dump = format!(
        "$ cat .env\n{env_file}$ cat .git/config\n[remote \"origin\"]\n\
         \turl = https://bot:{git}@git.example.test/team/repo.git\n"
    );
    let escaped = format!(
        "{{\"type\":\"tool_result\",\"content\":{}}}",
        serde_json::to_string(&dump)?
    );
    for input in [dump.into_bytes(), escaped.into_bytes()] {
        let whole = m.mask_whole(&input);
        let cuts: Vec<usize> = (1..input.len() / 7).map(|i| i * 7).collect();
        let streamed = stream_in(&m, StreamKind::Native, &input, &cuts);
        assert_eq!(streamed, whole);
        assert_eq!(whole.bytes.len(), input.len());
        for value in [&aws, &encryption, &stripe, &git] {
            assert!(holds(&input, value));
            assert!(!holds(&whole.bytes, value));
        }
    }
    Ok(())
}

/// Ordinary values stay out of masking: names that are not credential-named, a URL and a path
/// under credential names, and a value under 20 bytes.
#[test]
fn ordinary_values_do_not_pass_the_name_gate() -> Res {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join(".env"),
        "MODEL_NAME=some-model-name-2026-preview\n\
         DATABASE_HOST=db-primary-internal-host-01.example.test\n\
         LOG_LEVEL=info,goalcup=debug,hyper=warn,tower=info\n\
         API_BASE_URL=https://api.example.test/v1/chat/completions\n\
         AUTH_URL=https://auth.example.test/oauth2/authorize\n\
         SSH_KEY_PATH=/home/participant/.ssh/id_ed25519_work\n\
         TOKEN_DIR=~/.config/some-tool/tokens/cache\n\
         SHORT_TOKEN=abc123def456ghi789\n",
    )?;
    let env = BTreeMap::from([
        (
            "PATH".to_string(),
            "/usr/local/bin:/usr/bin:/bin:/opt/tool/bin".to_string(),
        ),
        (
            "HOME".to_string(),
            "/home/participant/isolated-home".to_string(),
        ),
        (
            "OPENAI_BASE_URL".to_string(),
            "https://gateway.example.test/v1".to_string(),
        ),
    ]);
    assert!(workspace_values(dir.path()).is_empty());
    assert!(env_values(&env).is_empty());
    let m = Masker::new(&[], launch_values(dir.path(), &env))?;
    let counts = m.value_counts();
    assert_eq!(
        (counts.workspace, counts.env, counts.harness, counts.skipped),
        (0, 0, 0, 0)
    );
    Ok(())
}

/// Source 2: credential-named environment variables are used, after `GH_TOKEN` and before the
/// other key-file values; a value in both the environment and `.env` counts once.
#[test]
fn environment_values_are_used_once_in_section_nine_order() -> Res {
    let mut rng = Rng(0x5EC2);
    let (gh, provider, gateway, own) = (
        rng.chars(ALNUM, 36),
        rng.chars(ALNUM, 48),
        rng.chars(ALNUM, 30),
        rng.chars(ALNUM, 28),
    );
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join(".env.local"),
        format!("SERVICE_SECRET={own}\nGH_TOKEN={gh}\nAI_GATEWAY_API_KEY={gateway}\n"),
    )?;
    let env = BTreeMap::from([
        ("PROVIDER_API_KEY".to_string(), provider.clone()),
        ("GATEWAY_AUTH".to_string(), gateway.clone()),
        (
            "PATH".to_string(),
            "/usr/local/bin:/usr/bin:/bin:/opt/tool/bin".to_string(),
        ),
        (
            "HOME".to_string(),
            "/home/participant/isolated-home".to_string(),
        ),
    ]);
    assert_eq!(env_values(&env).len(), 2);
    let values = launch_values(dir.path(), &env);
    let sources: Vec<ValueSource> = values.iter().map(SecretValue::source).collect();
    assert_eq!(
        sources,
        [
            ValueSource::Workspace,
            ValueSource::Env,
            ValueSource::Env,
            ValueSource::Workspace,
            ValueSource::Workspace,
        ]
    );
    let m = Masker::new(&[], values)?;
    let counts = m.value_counts();
    assert_eq!(
        (counts.workspace, counts.env, counts.harness, counts.skipped),
        (2, 2, 0, 0)
    );
    for value in [&gh, &provider, &gateway, &own] {
        assert!(
            m.mask_whole(value.as_bytes())
                .bytes
                .iter()
                .all(|&b| b == b'*')
        );
    }
    Ok(())
}

/// The guard: of 35 usable values, 32 are used and the rest counted in `skipped`. `Debug` of
/// the masker and of every value prints no value byte.
#[test]
fn past_thirty_two_values_the_rest_are_skipped_and_nothing_prints() -> Res {
    let mut rng = Rng(0x3232);
    let file: Vec<String> = (0..20).map(|_| rng.chars(ALNUM, 24)).collect();
    let env: BTreeMap<String, String> = (0..15)
        .map(|i| (format!("TOOL_{i}_TOKEN"), rng.chars(ALNUM, 24)))
        .collect();
    let dir = tempfile::tempdir()?;
    let text: String = file
        .iter()
        .enumerate()
        .map(|(i, v)| format!("SERVICE_{i}_KEY={v}\n"))
        .collect();
    std::fs::write(dir.path().join(".env"), text)?;
    let values = launch_values(dir.path(), &env);
    assert_eq!(values.len(), 35);
    let all: Vec<&String> = file.iter().chain(env.values()).collect();
    for value in &values {
        let shown = format!("{value:?}");
        assert!(all.iter().all(|v| !shown.contains(v.as_str())));
    }
    let m = Masker::new(&[], values)?;
    let counts = m.value_counts();
    assert_eq!(counts.workspace + counts.env, 32);
    assert_eq!((counts.env, counts.skipped), (15, 3));
    let shown = format!("{m:?}");
    assert!(all.iter().all(|v| !shown.contains(v.as_str())));
    Ok(())
}
