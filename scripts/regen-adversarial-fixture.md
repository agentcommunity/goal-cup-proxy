# Regenerating the adversarial fixture

`crates/goalcup-verifier/tests/fixtures/adversarial/` is **generated and committed**. It is
derived deterministically from `spikes/claude-code-cli/fixtures/` by
`crates/goalcup-verifier/tests/support/adversarial.rs`.

To regenerate:

    GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test adversarial -- --ignored regenerate

Then read the diff before committing. Every byte is a pure function of the committed input,
so an unexpected diff means the generator changed, not the fixture.

Two rules:

- The generator **reads** `spikes/` and never writes there. Nothing under `spikes/` is ever
  modified — it is the measurement (AGENTS.md).
- The generator also **deletes** any retired input it finds in the directory
  (`main.jsonl`, `subagent.jsonl`, `hooks.jsonl`, `events.jsonl`) and rewrites `acks.jsonl`
  from scratch. The trace declares one chain and nothing else, so a file the descriptor
  does not name has no business sitting beside it — a reader who found one would have to
  guess whether the verifier had read it.
- `cargo test` alone never rewrites the fixture: the regenerate test is `#[ignore]`d **and**
  gated on `GOALCUP_REGEN=1`, so neither an ordinary run nor
  `cargo test -- --include-ignored` can rewrite it. A change that alters the fixture fails
  `the_checked_in_fixture_is_what_the_generator_produces` instead of quietly moving the
  goalposts.

`expected-verdict.json` is **not** generated. It is written by hand from a real run and read
by eye, which is the point: a machine-written expectation cannot catch a machine-written
mistake. After regenerating, run

    cargo run -p goalcup-verifier --bin goalcup-verify -- crates/goalcup-verifier/tests/fixtures/adversarial

and compare its `review_queue`, `checks` and `seal_attribution` against the file before
editing it. `finding_codes` is compared for **set equality**, so a new finding code fails the
test until someone has decided whether it is the truth about the planted bytes.

What is planted, and why: see the doc comment at the top of `tests/support/adversarial.rs`.
