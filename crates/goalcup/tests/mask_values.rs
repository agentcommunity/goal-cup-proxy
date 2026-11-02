//! Exact values (PRD-05 section 9, sources 1 to 3): the project's `GH_TOKEN` value is masked in
//! all 11 forms, wrapped or not, whole and streamed, and so are the credential-named values of
//! the project's key files and of the harness environment, behind the name gate. Every value is
//! fake: built from pieces or drawn from a seeded generator.

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use common::{ALNUM, HEX, Rng, starred, stream_in};
use goalcup::mask::{Masker, SecretValue, ValueSource, launch_values, process_env};
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

/// Whether every byte of `value` is starred by `m`.
fn masks(m: &Masker, value: &str) -> bool {
    m.mask_whole(value.as_bytes())
        .bytes
        .iter()
        .all(|&b| b == b'*')
}

/// The `GH_TOKEN` of `.env.local` comes first, and every other usable `GH_TOKEN` line (an older
/// one in `.env`, an earlier one in the same file) is masked too. A value with a ` #` comment
/// after it is used; a file that is not a key file is not read.
#[test]
fn key_file_values_mask_every_gh_token_of_the_key_files() -> Res {
    let dir = tempfile::tempdir()?;
    let (a, c, e) = (
        ["Aa1", &"b".repeat(30)].concat(),
        ["Cc3", &"d".repeat(30)].concat(),
        ["Ee5", &"f".repeat(30)].concat(),
    );
    let (k, n) = (
        ["Kk7", &"m".repeat(30)].concat(),
        ["Nn9", &"p".repeat(30)].concat(),
    );
    std::fs::write(
        dir.path().join(".env"),
        format!("GH_TOKEN={a}\nSERVICE_KEY={k} # prod\n"),
    )?;
    std::fs::write(
        dir.path().join(".env.local"),
        format!("export GH_TOKEN=\"{e}\"\nOTHER=1\n  GH_TOKEN = '{c}'\n"),
    )?;
    std::fs::write(dir.path().join("notes.txt"), format!("GH_TOKEN={n}\n"))?;
    let values = launch_values(dir.path(), &BTreeMap::new());
    assert_eq!(values.len(), 4);
    assert!(values.iter().all(|v| v.source() == ValueSource::Workspace));
    let first = Masker::new(&[], values[..1].to_vec())?;
    assert!(masks(&first, &c));
    let m = Masker::new(&[], values)?;
    for value in [&a, &c, &e, &k] {
        assert!(masks(&m, value));
    }
    assert_eq!(m.mask_whole(n.as_bytes()).bytes, n.as_bytes());

    std::fs::write(dir.path().join(".env.local"), "GH_TOKEN=short_value_19c\n")?;
    let m = Masker::new(&[], launch_values(dir.path(), &BTreeMap::new()))?;
    assert_eq!(
        m.value_counts().workspace,
        2,
        "a value under 20 bytes is not used"
    );
    assert!(masks(&m, &a));
    assert!(launch_values(&dir.path().join("missing"), &BTreeMap::new()).is_empty());
    Ok(())
}

/// A FIFO named like a key file is never opened, so start never blocks; a symlinked key file
/// (a worktree's `.env`) is read.
#[cfg(unix)]
#[test]
fn a_fifo_key_file_is_skipped_and_a_symlinked_one_is_read() -> Res {
    let dir = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let v = ["Ln4", &"q".repeat(30)].concat();
    std::fs::write(outside.path().join("env"), format!("SERVICE_TOKEN={v}\n"))?;
    std::os::unix::fs::symlink(outside.path().join("env"), dir.path().join(".env"))?;
    let made = std::process::Command::new("mkfifo")
        .arg(dir.path().join(".env.pipe"))
        .status()?;
    assert!(made.success());
    let m = Masker::new(&[], launch_values(dir.path(), &BTreeMap::new()))?;
    assert_eq!(m.value_counts().workspace, 1);
    assert!(masks(&m, &v));
    Ok(())
}

/// A value already in use and sent again (here first as a harness value, which is not used) is
/// still masked and counted once.
#[test]
fn a_value_after_its_unused_twin_is_still_used() -> Res {
    let v = ["Hh2", &"r".repeat(30)].concat();
    let m = Masker::new(
        &[],
        vec![
            SecretValue::new(ValueSource::Harness, v.clone().into_bytes()),
            SecretValue::new(ValueSource::Workspace, v.clone().into_bytes()),
            SecretValue::new(ValueSource::Env, v.clone().into_bytes()),
        ],
    )?;
    let counts = m.value_counts();
    assert_eq!(
        (counts.workspace, counts.env, counts.harness, counts.skipped),
        (1, 0, 0, 1)
    );
    assert!(masks(&m, &v));
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

/// A value of 256 bytes drawn from `"`, `\`, `<` and `&` has forms near 1,792 bytes. The trie
/// uses it where a per-form regex could hit a compile limit: it is counted, masked in its
/// nested HTML-safe form, and the stream equals `mask_whole`.
#[test]
fn a_value_of_256_escape_bytes_is_used_and_masked() -> Res {
    let mut rng = Rng(0x0E5C_0256);
    let value = rng.chars(b"\"\\<&ABCabc012", 256);
    let m = masker(&value)?;
    let c = m.value_counts();
    assert_eq!((c.workspace, c.skipped), (1, 0));
    let twice = html_safe(&html_safe(&value));
    let input = format!("{{\"t\":\"{twice}\"}}\n").into_bytes();
    let whole = m.mask_whole(&input);
    assert!(starred(&whole, 6..6 + twice.len()));
    let cuts: Vec<usize> = (1..input.len()).step_by(97).collect();
    assert_eq!(stream_in(&m, StreamKind::Native, &input, &cuts), whole);
    Ok(())
}

/// A push cut at every byte of a wrapped value, including between CR and LF and inside an
/// escaped break, streams exactly like `mask_whole`, and no value byte leaves raw.
#[test]
fn a_wrapped_value_cut_at_every_byte_streams_like_mask_whole() -> Res {
    let v = token();
    let m = masker(&v)?;
    for brk in ["\r\n", "\\r\\n", "\\\\r\\\\n", "\\r\\\\n"] {
        let input = format!("x {}{brk}{} y\n", &v[..10], &v[10..]).into_bytes();
        let whole = m.mask_whole(&input);
        assert!(starred(&whole, 2..2 + 10) && !holds(&whole.bytes, &v[10..]));
        for cut in 1..input.len() {
            assert_eq!(
                stream_in(&m, StreamKind::Native, &input, &[cut]),
                whole,
                "cut {cut}"
            );
        }
    }
    Ok(())
}

/// Value B is value A plus 10 bytes (a trie end node with children), and a value written twice
/// back to back: both are masked whole, with no raw tail and no raw second copy.
#[test]
fn a_value_inside_a_longer_value_and_back_to_back_values_are_masked() -> Res {
    let mut rng = Rng(0x1B51_DE00);
    let a = rng.chars(ALNUM, 24);
    let b = format!("{a}{}", rng.chars(ALNUM, 10));
    let m = Masker::new(
        &[],
        [&a, &b]
            .iter()
            .map(|v| SecretValue::new(ValueSource::Workspace, v.as_bytes().to_vec()))
            .collect(),
    )?;
    let input = format!("k={b} {a}{a} end\n").into_bytes();
    let whole = m.mask_whole(&input);
    let second = 2 + b.len() + 1;
    assert!(starred(&whole, 2..2 + b.len()));
    assert!(starred(&whole, second..second + 2 * a.len()));
    let cuts: Vec<usize> = (1..input.len()).collect();
    assert_eq!(stream_in(&m, StreamKind::Native, &input, &cuts), whole);
    Ok(())
}

/// One to eight backslashes before each of 32 values, repeated to 256 KiB: every `\u` form
/// starts on a backslash, which also starts escaped breaks. The stream never fails closed and
/// equals `mask_whole`.
#[test]
fn backslash_runs_before_values_never_fail_closed() -> Res {
    let mut rng = Rng(0xBA5E_0032);
    let values: Vec<String> = (0..32).map(|_| rng.chars(ALNUM, 256)).collect();
    let m = Masker::new(
        &[],
        values
            .iter()
            .map(|v| SecretValue::new(ValueSource::Workspace, v.clone().into_bytes()))
            .collect(),
    )?;
    let mut input = String::new();
    let mut i = 0;
    while input.len() < 256 * 1024 {
        input.push_str(&"\\".repeat(1 + i % 8));
        input.push_str(&values[i % 32]);
        input.push_str(" \\u0041\\n\n");
        i += 1;
    }
    let input = input.into_bytes();
    let whole = m.mask_whole(&input);
    assert!(values.iter().all(|v| !holds(&whole.bytes, v)));
    let cuts: Vec<usize> = (1..input.len() / 4096).map(|k| k * 4096).collect();
    let got = stream_in(&m, StreamKind::Native, &input, &cuts);
    assert_eq!(got, whole);
    let fail_closed = m.mask_rules().len() - 1;
    assert!(got.spans.iter().all(|s| s.rule != fail_closed));
    Ok(())
}

/// An invalid UTF-8 byte right before a value, right after it, and right after a second copy
/// that a raw LF break wraps: the value is masked whole and streamed, the bad byte stays raw.
#[test]
fn a_value_next_to_invalid_utf8_is_masked_whole_and_streamed() -> Res {
    let v = token();
    let m = masker(&v)?;
    let mut input = b"a\xff".to_vec();
    input.extend_from_slice(v.as_bytes());
    input.extend_from_slice(b"\xff b ");
    input.extend_from_slice(&v.as_bytes()[..8]);
    input.extend_from_slice(b"\n");
    input.extend_from_slice(&v.as_bytes()[8..]);
    input.extend_from_slice(b"\xff\n");
    let whole = m.mask_whole(&input);
    assert!(starred(&whole, 2..2 + v.len()));
    assert_eq!(whole.bytes[1], 0xff);
    assert_eq!(whole.bytes[2 + v.len()], 0xff);
    let cuts: Vec<usize> = (1..input.len()).collect();
    assert_eq!(stream_in(&m, StreamKind::Native, &input, &cuts), whole);
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
    assert!(launch_values(dir.path(), &env).is_empty());
    let m = Masker::new(&[], launch_values(dir.path(), &env))?;
    let counts = m.value_counts();
    assert_eq!(
        (counts.workspace, counts.env, counts.harness, counts.skipped),
        (0, 0, 0, 0)
    );
    Ok(())
}

/// Source 2: credential-named environment variables are used; a value in both the environment
/// and `.env` counts once, as `workspace`.
#[test]
fn environment_values_are_used_and_a_shared_value_counts_once() -> Res {
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
    ]);
    let m = Masker::new(&[], launch_values(dir.path(), &env))?;
    let counts = m.value_counts();
    assert_eq!(
        (counts.workspace, counts.env, counts.harness, counts.skipped),
        (3, 1, 0, 0)
    );
    for value in [&gh, &provider, &gateway, &own] {
        assert!(masks(&m, value));
    }
    Ok(())
}

/// The guard, in the reader: of 35 usable values, the 20 file values then 12 environment values
/// are kept, and one count-only entry carries the 3 left out into `skipped`. `Debug` of the
/// masker and of every value prints no value byte.
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
    assert_eq!(values.len(), 33);
    let last = Masker::new(&[], values[32..].to_vec())?.value_counts();
    assert_eq!((last.workspace, last.env, last.skipped), (0, 0, 3));
    let kept: Vec<&String> = file.iter().chain(env.values().take(12)).collect();
    for (value, want) in values[..32].iter().zip(&kept) {
        let one = Masker::new(&[], vec![value.clone()])?;
        assert!(
            one.mask_whole(want.as_bytes())
                .bytes
                .iter()
                .all(|&b| b == b'*')
        );
    }
    let all: Vec<&String> = file.iter().chain(env.values()).collect();
    for value in &values {
        let shown = format!("{value:?}");
        assert!(all.iter().all(|v| !shown.contains(v.as_str())));
    }
    let m = Masker::new(&[], values)?;
    let counts = m.value_counts();
    assert_eq!(
        (counts.workspace, counts.env, counts.harness, counts.skipped),
        (20, 12, 0, 3)
    );
    let shown = format!("{m:?}");
    assert!(all.iter().all(|v| !shown.contains(v.as_str())));
    Ok(())
}

/// Model ids and hosts under credential names are not secrets: starring them would break usage
/// reading. A UUID-style key and a mixed-case token still pass.
#[test]
fn model_ids_and_hosts_do_not_pass_the_name_gate() -> Res {
    let mut rng = Rng(0x0DE1);
    let uuid = [8, 4, 4, 4, 12]
        .iter()
        .map(|&n| rng.chars(HEX, n))
        .collect::<Vec<_>>()
        .join("-");
    let mixed = ["Mx", &rng.chars(ALNUM, 30)].concat();
    let names = [
        ("X_AUTH_MODEL", "vendor-model-4-5-20990101"),
        ("API_KEY_BASE", "gateway.example.test/api/v1"),
        (
            "DEFAULT_TOKEN_MODEL",
            "accounts/vendor/models/model-v3p1-8b-instruct",
        ),
    ];
    let mut text: String = names.iter().map(|(n, v)| format!("{n}={v}\n")).collect();
    text.push_str(&format!("SERVICE_KEY={uuid}\nSERVICE_TOKEN={mixed}\n"));
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(".env"), text)?;
    let env: BTreeMap<String, String> = names
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    let empty = tempfile::tempdir()?;
    assert!(launch_values(empty.path(), &env).is_empty());
    let values = launch_values(dir.path(), &env);
    let m = Masker::new(&[], values)?;
    let counts = m.value_counts();
    assert_eq!(
        (counts.workspace, counts.env, counts.harness, counts.skipped),
        (2, 0, 0, 0)
    );
    for kept in [&uuid, &mixed] {
        assert!(
            m.mask_whole(kept.as_bytes())
                .bytes
                .iter()
                .all(|&b| b == b'*')
        );
    }
    for (_, refused) in names {
        assert_eq!(m.mask_whole(refused.as_bytes()).bytes, refused.as_bytes());
    }
    Ok(())
}

/// Point 3: a `.env` of more than 64 KiB, 200 distinct gated values of 256 bytes (one with a
/// quote and a backslash) among duplicates and ordinary lines, and 30 gated environment values,
/// still starts. Each reader keeps the first 32 file values plus one count-only entry, the line
/// stays below `watchdog::MAX_LINE`, the values cross it unchanged and the masker builds with
/// every distinct value past 32 in `skipped`.
#[test]
fn a_64_kib_env_file_starts() -> Res {
    let mut rng = Rng(0x64E1);
    let special = ["Qz\"7\\w", &rng.chars(ALNUM, 250)].concat();
    let mut distinct = vec![special];
    distinct.extend((0..199).map(|_| rng.chars(ALNUM, 256)));
    let mut text = String::new();
    for (i, v) in distinct.iter().enumerate() {
        text.push_str(&format!("SERVICE_{i}_KEY={v}\n"));
    }
    let mut line = 0;
    while text.len() < 64 * 1024 + 4096 {
        let v = &distinct[rng.below(distinct.len())];
        text.push_str(&format!("COPY_{line}_TOKEN={v}\n"));
        text.push_str(&format!(
            "LOG_{line}_LEVEL=info,{}=debug\n",
            rng.chars(ALNUM, 30)
        ));
        line += 1;
    }
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(".env"), &text)?;
    let env: BTreeMap<String, String> = (0..30)
        .map(|i| (format!("TOOL_{i:02}_API_KEY"), rng.chars(ALNUM, 64)))
        .collect();
    let file_skipped = (distinct.len() - 32) as u32;
    for (values, skipped) in [
        (launch_values(dir.path(), &BTreeMap::new()), file_skipped),
        (launch_values(dir.path(), &env), file_skipped + 30),
    ] {
        let json = serde_json::to_string(&values)?;
        assert!(json.len() < goalcup::watchdog::MAX_LINE);
        let back: Vec<SecretValue> = serde_json::from_str(&json)?;
        assert_eq!(back, values);
        let m = Masker::new(&[], back)?;
        let counts = m.value_counts();
        assert_eq!(
            (counts.workspace, counts.env, counts.skipped),
            (32, 0, skipped)
        );
        for value in &distinct[1..32] {
            assert!(masks(&m, value));
        }
        assert!(json_masked(&m, &distinct[0])?);
    }
    Ok(())
}

/// Whether `value` is starred where it sits inside a JSON string, and inside a JSON string
/// nested in one, as a `cat .env` tool result travels in a request body. The form starred is
/// the escaped one: `"` and `\` gain one backslash, then three.
fn json_masked(m: &Masker, value: &str) -> Result<bool, serde_json::Error> {
    let once = serde_json::to_string(value)?;
    let inner = &once[1..once.len() - 1];
    let twice = serde_json::to_string(inner)?;
    let inner2 = &twice[1..twice.len() - 1];
    let input = format!("{{\"p\":{once}}}");
    let nested = format!("{{\"c\":{}}}", serde_json::to_string(&input)?);
    let mut all = true;
    for (text, form) in [(&input, inner), (&nested, inner2)] {
        let Some(at) = text.find(form) else {
            return Ok(false);
        };
        let whole = m.mask_whole(text.as_bytes());
        all &= starred(&whole, at..at + form.len());
        let cuts: Vec<usize> = (1..text.len() / 5).map(|i| i * 5).collect();
        all &= stream_in(m, StreamKind::Native, text.as_bytes(), &cuts) == whole;
    }
    Ok(all)
}

/// Passwords holding `"` and `\` (source 3) are masked inside JSON strings, once and twice
/// escaped, whole and streamed.
#[test]
fn a_value_with_quotes_and_backslashes_is_masked_in_json() -> Res {
    let mut rng = Rng(0x0A57);
    let quoted = ["Qz\"7\\w", &rng.chars(ALNUM, 30)].concat();
    let edges = ["\\", &rng.chars(ALNUM, 24), "\"\"\\"].concat();
    let m = Masker::new(
        &[],
        [&quoted, &edges]
            .iter()
            .map(|v| SecretValue::new(ValueSource::Workspace, v.as_bytes().to_vec()))
            .collect(),
    )?;
    assert_eq!(m.value_counts().workspace, 2);
    for value in [&quoted, &edges] {
        assert!(json_masked(&m, value)?);
    }
    Ok(())
}

/// One byte that is not UTF-8 costs only its line, a leading BOM costs nothing, and a `#`
/// comment after a tab is cut off.
#[test]
fn a_key_file_with_a_bad_byte_or_a_bom_still_gives_its_values() -> Res {
    let mut rng = Rng(0xB0B0);
    let (gh, service, tabbed) = (
        rng.chars(ALNUM, 36),
        rng.chars(ALNUM, 26),
        rng.chars(ALNUM, 28),
    );
    let dir = tempfile::tempdir()?;
    let mut local = b"# caf\xe9 note\nGH_TOKEN=".to_vec();
    local.extend_from_slice(gh.as_bytes());
    local.extend_from_slice(format!("\nA_KEY={tabbed}\t# note\n").as_bytes());
    std::fs::write(dir.path().join(".env.local"), local)?;
    std::fs::write(
        dir.path().join(".env"),
        format!("\u{feff}SERVICE_KEY={service}\n"),
    )?;
    let m = Masker::new(&[], launch_values(dir.path(), &BTreeMap::new()))?;
    assert_eq!(m.value_counts().workspace, 3);
    for value in [&gh, &service, &tabbed] {
        assert!(masks(&m, value));
    }
    Ok(())
}

/// The inside of a JSON string as an HTML-safe encoder writes it (Go's `encoding/json` by
/// default): `"` and `\` gain a backslash, `<`, `>` and `&` become `<`, `>`,
/// `&`, every other printable byte stays raw.
fn html_safe(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '"' | '\\' => format!("\\{c}"),
            '<' | '>' | '&' => format!("\\u{:04x}", u32::from(c)),
            _ => c.to_string(),
        })
        .collect()
}

/// A password holding `&`, `<` and `>` (and a `"`) is masked inside a JSON body an HTML-safe
/// encoder wrote, and inside one such body nested in another, whole and streamed.
#[test]
fn a_value_with_html_bytes_is_masked_in_html_safe_json() -> Res {
    let mut rng = Rng(0x60E5);
    let values = [
        ["Pw&", &rng.chars(ALNUM, 24), "<x>"].concat(),
        ["a\"b&", &rng.chars(ALNUM, 22), "\\&"].concat(),
    ];
    let m = Masker::new(
        &[],
        values
            .iter()
            .map(|v| SecretValue::new(ValueSource::Workspace, v.as_bytes().to_vec()))
            .collect(),
    )?;
    assert_eq!(m.value_counts().workspace, 2);
    for value in &values {
        let once = html_safe(value);
        let twice = html_safe(&once);
        let body = format!("{{\"cmd\":\"cat .env\",\"out\":\"PASSWORD={once}\\n\"}}");
        let nested = format!("{{\"content\":\"{}\"}}", html_safe(&body));
        for (text, form) in [(&body, &once), (&nested, &twice)] {
            let at = text.find(form.as_str()).ok_or("form not in body")?;
            let whole = m.mask_whole(text.as_bytes());
            assert!(starred(&whole, at..at + form.len()));
            let cuts: Vec<usize> = (1..text.len() / 3).map(|i| i * 3).collect();
            assert_eq!(
                stream_in(&m, StreamKind::Native, text.as_bytes(), &cuts),
                whole
            );
        }
    }
    Ok(())
}

/// Ruling OWN-10: the launch reads the environment goalcup builds for the harness (source 2)
/// through `launch_values`, and with 32 worst-case values of 256 bytes (every byte after the tag
/// a `"` or `\`, so its JSON text is twice as long) plus left-out ones, the whole start line of
/// every built-in connector stays under `watchdog::MAX_LINE`, so the recorder can read it.
#[test]
fn the_start_line_with_32_long_values_fits() -> Res {
    use goalcup::connector::{self, Env, builtin};
    use goalcup::proxy::ca::RunCa;
    use goalcup::watchdog::{Cause, Exit, HarnessFacts, MAX_LINE, Prev, StartFacts, ToRecorder};
    use serde_json::to_value;
    use std::time::SystemTime;

    let dir = tempfile::tempdir()?;
    let project = dir.path().join("w");
    std::fs::create_dir_all(&project)?;
    let ca = RunCa::mint(&["example.org".to_string()])
        .map_err(|_| "no run CA")?
        .export();
    let mut vars = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
    let mut fakes = Vec::new();
    for i in 0..40 {
        let tail: String = (0..254)
            .map(|j| if j % 2 == 0 { '"' } else { '\\' })
            .collect();
        let value = format!("{i:02}{tail}");
        vars.insert(format!("FAKE_{i:02}_API_KEY"), value.clone());
        fakes.push(value);
    }
    let mut longest = 0;
    for (id, _) in builtin::CONNECTORS {
        let c = connector::load(id, None).map_err(|e| e.to_string())?;
        let env = Env {
            vars: vars.clone(),
            workspace: project.clone(),
            resume: true,
            run_start: SystemTime::now(),
            goalcup_home: dir.path().join("g"),
            server_host: "ingest.example.org".into(),
        };
        let plan = connector::plan(&c, &env, &dir.path().join("homes").join(id));
        let values = launch_values(&project, &plan.env);
        // The environment values reach the masker: the first fake is among them.
        let text = serde_json::to_string(&values)?;
        assert!(text.contains(&serde_json::to_string(&fakes[0])?));
        let facts = StartFacts {
            run: format!("r{}", "a".repeat(16)),
            entry: format!("e{}", "b".repeat(16)),
            segment: 2,
            entry_dir: dir.path().join("runs/r/e"),
            deadline: Some(u64::MAX),
            server_time: Some(u64::MAX),
            local_wall: u64::MAX,
            workspace: project.clone(),
            connector: to_value(&plan.connector)?,
            harness_version: Some("1.2.3 (fake)".into()),
            keep: true,
            build: Some("f".repeat(40)),
            hosts: to_value(&plan.hosts)?,
            hosts_sha256: "0".repeat(64),
            native: to_value(&plan.native)?,
            mask_rules: to_value(&plan.mask_rules)?,
            values: to_value(&values)?,
            ca: ca.clone(),
            prev: Some(Prev::Cause(Cause::NullMode {
                exits: 3,
                last: Some(Exit::Signal(9)),
            })),
            harness: Some(HarnessFacts {
                pid: u32::MAX,
                program: "/usr/local/bin/harness".into(),
                started: "2026-11-07T00:00:00Z".into(),
                start_id: u64::MAX,
            }),
            null_since: Some(u64::MAX),
            notes: plan.notes.clone(),
        };
        let line = serde_json::to_vec(&ToRecorder::Start(Box::new(facts)))?;
        longest = longest.max(line.len() + 1);
    }
    assert!(longest <= MAX_LINE, "{longest}");
    Ok(())
}

/// `recover` reads the values the live harness was started with, not the recovering terminal's
/// (point 4): a credential-named value only in the harness's own environment reaches the masker.
#[cfg(target_os = "linux")]
#[test]
fn recover_reads_the_harness_values_from_its_own_environment() -> Res {
    let v = ["Rc7", &"w".repeat(30)].concat();
    assert!(std::env::var_os("SERVICE_TOKEN").is_none());
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("SERVICE_TOKEN", &v)
        .spawn()?;
    let env = process_env(child.id());
    child.kill()?;
    child.wait()?;
    let env = env.ok_or("no environment")?;
    let dir = tempfile::tempdir()?;
    let m = Masker::new(&[], launch_values(dir.path(), &env))?;
    assert_eq!(m.value_counts().env, 1);
    assert!(masks(&m, &v));
    assert!(process_env(u32::MAX).is_none());
    Ok(())
}

/// Four synthetic 64-character base64 lines and a short last one, as the body of a PEM private
/// key (never a real key).
fn pem_body(seed: u64) -> Vec<String> {
    let mut rng = Rng(seed);
    let mut lines: Vec<String> = (0..4)
        .map(|_| [rng.chars(ALNUM, 31), "+/".into(), rng.chars(ALNUM, 31)].concat())
        .collect();
    lines.push([rng.chars(ALNUM, 6), "==".into()].concat());
    lines
}

/// A quoted multi-line PEM private key in `.env.local`, under a name with no credential word:
/// each usable body line is a workspace value (the short last line is not usable).
#[test]
fn a_multi_line_pem_key_in_a_key_file_gives_its_body_lines() -> Res {
    let body = pem_body(0x9E41);
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join(".env.local"),
        format!(
            "SIGNING_PEM=\"-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\"\nOTHER=1\n",
            body.join("\n")
        ),
    )?;
    let values = launch_values(dir.path(), &BTreeMap::new());
    assert_eq!(values.len(), 4);
    assert!(values.iter().all(|v| v.source() == ValueSource::Workspace));
    let m = Masker::new(&[], values)?;
    assert_eq!(m.value_counts().workspace, 4);
    for line in &body[..4] {
        assert!(masks(&m, line));
    }
    Ok(())
}

/// The A3 case: a tool output that dropped the BEGIN line (a redacting `sed` of `.env.local`)
/// holds only the body and the END line, JSON-escaped in a request body. Every body line is
/// masked by exact value, whole and streamed.
#[test]
fn a_pem_body_without_its_begin_line_is_masked() -> Res {
    let body = pem_body(0x9E42);
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join(".env.local"),
        format!(
            "export DEPLOY_PEM='-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----'\n",
            body.join("\n")
        ),
    )?;
    let m = Masker::new(&[], launch_values(dir.path(), &BTreeMap::new()))?;
    let printed = format!("{}\n-----END RSA PRIVATE KEY-----\"\n", body.join("\n"));
    let input = format!(
        "{{\"type\":\"tool_result\",\"content\":{}}}",
        serde_json::to_string(&printed)?
    )
    .into_bytes();
    let whole = m.mask_whole(&input);
    let cuts: Vec<usize> = (1..input.len() / 5).map(|i| i * 5).collect();
    assert_eq!(stream_in(&m, StreamKind::Native, &input, &cuts), whole);
    assert_eq!(whole.bytes.len(), input.len());
    for line in &body[..4] {
        assert!(holds(&input, line));
        assert!(!holds(&whole.bytes, line));
    }
    assert!(holds(&whole.bytes, "-----END RSA PRIVATE KEY-----"));
    Ok(())
}

/// A PEM key written on one line with escaped `\n` breaks, as many tools write a service
/// account key into `.env`, gives its body lines too.
#[test]
fn a_one_line_escaped_pem_key_gives_its_body_lines() -> Res {
    let body = pem_body(0x9E43);
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join(".env"),
        format!(
            "SA_PEM=\"-----BEGIN PRIVATE KEY-----\\n{}\\n-----END PRIVATE KEY-----\\n\"\n",
            body.join("\\n")
        ),
    )?;
    let m = Masker::new(&[], launch_values(dir.path(), &BTreeMap::new()))?;
    assert_eq!(m.value_counts().workspace, 4);
    for line in &body[..4] {
        assert!(masks(&m, line));
    }
    Ok(())
}

/// PEM body lines rank last: under the guard of 32 they never push out a named value, even
/// from `.env.local`, which is read first.
#[test]
fn pem_body_lines_rank_after_every_named_value() -> Res {
    let mut rng = Rng(0x9E44);
    let named: Vec<String> = (0..31).map(|_| rng.chars(ALNUM, 24)).collect();
    let env_value = rng.chars(ALNUM, 28);
    let body = pem_body(0x9E45);
    let dir = tempfile::tempdir()?;
    let text: String = named
        .iter()
        .enumerate()
        .map(|(i, v)| format!("SERVICE_{i}_KEY={v}\n"))
        .collect();
    std::fs::write(dir.path().join(".env"), text)?;
    std::fs::write(
        dir.path().join(".env.local"),
        format!(
            "SIGNING_PEM=\"-----BEGIN EC PRIVATE KEY-----\n{}\n-----END EC PRIVATE KEY-----\"\n",
            body.join("\n")
        ),
    )?;
    let env = BTreeMap::from([("TOOL_TOKEN".to_string(), env_value.clone())]);
    let values = launch_values(dir.path(), &env);
    assert_eq!(values.len(), 33);
    let kept: Vec<&String> = named.iter().chain(std::iter::once(&env_value)).collect();
    for (value, want) in values[..32].iter().zip(&kept) {
        let one = Masker::new(&[], vec![value.clone()])?;
        assert!(masks(&one, want));
    }
    let last = Masker::new(&[], values[32..].to_vec())?.value_counts();
    assert_eq!((last.workspace, last.env, last.skipped), (0, 0, 4));
    Ok(())
}
