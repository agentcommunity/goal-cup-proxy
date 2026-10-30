//! Exact values (PRD-05 section 9, default build): the project's `GH_TOKEN` value is masked in
//! all 11 forms, wrapped or not, whole and streamed, and nothing value-derived is ever printed.
//! The value is fake and built from pieces.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use goalcup::mask::{Masked, Masker, SecretValue, ValueSource, WrappedForms, workspace_values};
use goalcup_format::StreamKind;

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

/// The 11 forms of `v`, each inside some text a harness might write.
fn texts(v: &str) -> Vec<Vec<u8>> {
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
    let mut out: Vec<Vec<u8>> = vec![
        format!("GH_TOKEN={v}\n").into_bytes(),
        format!("url?t={}&x=1", hex(true, false)).into_bytes(),
        format!("url?t={}&x=1", hex(false, false)).into_bytes(),
        format!("{{\"t\":\"{}\"}}", hex(true, true)).into_bytes(),
        format!("{{\"t\":\"{}\"}}", hex(false, true)).into_bytes(),
    ];
    for lead in 0..3 {
        let mut plain = b"env"[..lead].to_vec();
        plain.extend(format!("{v} and more text").bytes());
        out.push(format!("b64 {} end", STANDARD_NO_PAD.encode(&plain)).into_bytes());
        out.push(format!("b64 {} end", URL_SAFE_NO_PAD.encode(&plain)).into_bytes());
    }
    out
}

/// `v` with `brk` inside after its fifth byte.
fn wrapped(v: &str, brk: &str) -> Vec<u8> {
    format!("{{\"out\":\"{}{brk}{}\"}}", &v[..5], &v[5..]).into_bytes()
}

fn stream_in(m: &Masker, input: &[u8], split: usize) -> Masked {
    let mut s = m.stream(StreamKind::Native);
    let mut all = Masked::default();
    for piece in [s.push(&input[..split]), s.push(&input[split..]), s.finish()] {
        let base = all.bytes.len();
        all.bytes.extend(piece.bytes);
        all.spans.extend(piece.spans.into_iter().map(|mut x| {
            x.start += base;
            x
        }));
    }
    all
}

#[test]
fn the_gh_token_is_masked_in_every_form_whole_and_streamed() -> Res {
    let v = token();
    let m = masker(&v)?;
    assert_eq!(m.value_counts().workspace, 1);
    let rules = m.mask_rules();
    let forms = WrappedForms::of(v.as_bytes()).ok_or("a usable value")?;
    let mut inputs = texts(&v);
    for brk in ["\n", "\r\n", "\\n", "\\\\n", "\\r\\n"] {
        inputs.push(wrapped(&v, brk));
    }
    for input in &inputs {
        assert!(forms.contains(input), "the planted text holds a form");
        let whole = m.mask_whole(input);
        assert_eq!(whole.bytes.len(), input.len());
        assert!(
            !forms.contains(&whole.bytes),
            "{:?}",
            String::from_utf8_lossy(input)
        );
        assert!(
            whole.spans.iter().any(|s| rules[s.rule] == "run_secret"),
            "{:?}",
            String::from_utf8_lossy(input)
        );
        for split in 0..=input.len() {
            assert_eq!(stream_in(&m, input, split), whole, "split {split}");
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

#[test]
fn nothing_value_derived_is_printed() -> Res {
    let v = token();
    let value = SecretValue::new(ValueSource::Workspace, v.as_bytes().to_vec());
    let m = Masker::new(&[], vec![value.clone()])?;
    let forms = WrappedForms::of(v.as_bytes()).ok_or("a usable value")?;
    let printed = [
        format!("{value:?}"),
        format!("{m:?}"),
        format!("{:?}", m.stream(StreamKind::Native)),
        format!("{forms:?}"),
        format!("{:?}", m.value_counts()),
        format!("{}", goalcup::mask::MaskError::Value),
        format!("{}", goalcup::mask::MaskError::Rule(0)),
    ];
    for text in &printed {
        assert!(!forms.contains(text.as_bytes()), "{text}");
        assert!(!text.contains(&v[..8]), "{text}");
        assert!(!text.contains(&v.len().to_string()), "{text}");
    }
    let bad = Masker::new(
        &[],
        vec![SecretValue::new(
            ValueSource::Env,
            b"has a space in it, so unusable".to_vec(),
        )],
    )?;
    assert_eq!(bad.value_counts().skipped, 1);
    Ok(())
}
