//! Exact values (PRD-05 section 9, default build): the project's `GH_TOKEN` value is masked in
//! all 11 forms, wrapped or not, whole and streamed. The value is fake and built from pieces.

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use common::{starred, stream_in};
use goalcup::mask::{Masker, SecretValue, ValueSource, workspace_values};
use goalcup_format::StreamKind;
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
    (format!("{lead}{form}{tail}").into_bytes(), at..at + form.len())
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
        for text in [STANDARD_NO_PAD.encode(&plain), URL_SAFE_NO_PAD.encode(&plain)] {
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
