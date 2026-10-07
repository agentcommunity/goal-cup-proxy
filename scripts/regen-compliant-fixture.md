# Regenerating the compliant fixture

`crates/goalcup-verifier/tests/fixtures/compliant/` is **generated and committed**. It is
written from nothing but the constants in
`crates/goalcup-verifier/tests/support/compliant.rs` and one file it copies,
`connectors/claude-code-cli/manifest.toml`.

To regenerate:

    GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test compliant -- --ignored regenerate

Then read the diff before committing. Every byte is a pure function of the generator, so an
unexpected diff means the generator changed, not the fixture.

Four rules:

- The regeneration has **two locks**, not one. `#[ignore]` keeps it out of an ordinary
  `cargo test`; `GOALCUP_REGEN=1` keeps it out of `cargo test -- --include-ignored`, which
  the gate runs to prove the ignored tests still compile and pass. Without the variable set
  the test runs, writes nothing, prints the command that would, and passes — so including
  the ignored tests cannot make the fixture rewrite itself and then agree with itself.

- Nothing is read from `spikes/`. The adversarial fixture derives from a real capture
  because its claim is about real bytes; this one's claim is about the rules, so every
  record is written in the generator where a reader can check it.
- `manifest.toml` in the fixture is a **byte copy** of the shipped connector manifest. That
  is what makes the trace self-contained and still resolve to `ManifestPin::Pinned` — the
  pin is `(connector id, sha256)`, and a copy has the same digest. A change to
  `connectors/claude-code-cli/manifest.toml` therefore fails
  `the_committed_bytes_are_what_the_generator_produces` until this fixture is regenerated,
  which is the same discipline `scripts/pin-manifests.sh` imposes on
  `CONNECTOR_MANIFEST_SHA256S`.
- `cargo test` alone never rewrites the fixture: the regenerate test is `#[ignore]`d, so a
  change that alters the fixture fails the byte comparison instead of quietly moving the
  goalposts.

## The pass claim needs `--dev`, and the file records what a plain run says

The expectation is still one sentence — `overall` is `pass`, the review queue is empty, all
thirteen checks are `pass` and the binary exits 0 — and `tests/compliant.rs` still asserts
exactly that, in
`a_compliant_trace_passes_every_check_under_dev_and_the_queue_is_empty` and
`the_binary_exits_zero_on_it_with_dev`. What moved on 2026-09-05 is the flag those two
sentences need.

This trace signs its receipt and its acknowledgements with the seed `[5u8; 32]`, which is
written in `tests/support/compliant.rs` so the fixtures can sign at all. Its verifying key
is `PLACEHOLDER_SERVER_PUBLIC_KEY`, the single entry in `SERVER_PUBLIC_KEYS`, so anybody
holding this repository can mint a receipt that resolves to `key_source: "pinned"` — the
strongest provenance a verdict has. V13 raises `placeholder_server_key` whenever that key
verified something, and only `--dev` suppresses it. A development check that the RULES pass
is exactly a `--dev` run.

So there IS an `expected-verdict.json` here now, and it records the OTHER reading: the
verdict a person gets by running the binary with no flags. `overall` is `flag`, `exit_code`
is 1, `key_source` and `manifest_pin` are both `pinned`, and `finding_codes` holds exactly
one entry, `placeholder_server_key`. `the_committed_expected_verdict_without_dev_is_a_placeholder_key_flag`
reads the file and asserts every one of those fields against a real run.

It is **not** a per-code expectation of the passing run — that would let a fixture which
stopped exercising a rule keep passing, which is why the pass claim stays where it is. It
is a one-code expectation of the failing half, so the Flag cannot change without a diff.
The file is written by hand from a real run, like the adversarial fixture's; `GOALCUP_REGEN=1
… regenerate` writes the seven files in `support::compliant::FILES` and never touches it.

To read the verdict yourself:

    cargo run -p goalcup-verifier --bin goalcup-verify -- crates/goalcup-verifier/tests/fixtures/compliant --dev

Without `--dev` the same command exits 1 and the queue holds the placeholder Flag.

Thirteen of its findings are `clean_gap` — one per twelve-hour stretch of the seven-day
entry window. That is deliberate: the daemon reports in every twelve hours, the connector
declares a 60-second heartbeat, so every half-day silence is a hole that the trace
positively accounts for. `clean_gap` became a Pass-level finding in task 13, and this
fixture is where that is measured. The cadence was hourly until the task-13 fix wave, which
made the same point with 167 repetitions of one sentence and a 95 KB verdict.

What the run is: see the doc comment at the top of `tests/support/compliant.rs`.
