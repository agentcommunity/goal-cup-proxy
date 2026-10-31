# Adding a harness

The test pad is the one way to add a harness. A harness is data, a connector file and a pad file,
and the pad runs it and prints one table. Nothing is copied here:

- [The skill](../skills/adding-a-capture-harness/SKILL.md): the steps, for you or your coding agent.
- [Reviewing a connector pull request](../CONTRIBUTING.md#reviewing-a-connector-pull-request): the checklist.
- [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md): every key of a connector and the rule that refuses it.

## Check first

Before you write a connector, read the harness's facts sheet in [harnesses](harnesses/README.md).
It says what the harness signs in with and what goalcup cannot record or stop. A harness that keeps
its provider key only in an environment variable, or that has no way to make its model calls go
through the recording proxy, is the usual blocker.

## Do I need a core change?

A core change is needed when the pad refuses a connector with the reason of a closed rule. Those
rules are in
[`capture_policy.rs`](../crates/goalcup-protocol/src/capture_policy.rs) and
[`harness.rs`](../crates/goalcup-protocol/src/harness.rs), and
[CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md#load-checks) lists the refusals. Never spell around a rule.
Open an issue that names the rule and links your facts sheet, and write no connector until a
maintainer lands the change as a separate core pull request with a [DECISIONS](DECISIONS.md) entry.
