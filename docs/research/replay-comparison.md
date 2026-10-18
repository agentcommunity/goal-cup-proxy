# Offline replay comparison

`scripts/research-replay.py` compares two trace directories with the same local
`goalcup-verify` binary and options. It is for artifacts already in hand:

```sh
python3 scripts/research-replay.py \
  --local /private/research/local-trace \
  --rebuilt /private/research/rebuilt-trace \
  --output /private/research/replay-2026-10-19 \
  --verifier /path/to/goalcup-verify \
  --dev-key-file /private/research/dev-public-key.txt
```

`--verifier` defaults to this checkout's `target/debug/goalcup-verify`.
`--dev-key-file` is optional and must contain the verifier's 64-character
hexadecimal public key. The helper records that a key file was supplied and a
hash of its normalized public-key text; it does not put the key in
`comparison.json`. The verifier receives the same key option for both
integrity and research-report runs on both traces.

The output directory must be new, have an existing parent, and sit outside both
trace directory trees. It is created with mode `0700`; preserved outputs use
mode `0600`. The directory contains each leg's integrity stdout and stderr,
research stderr and complete NDJSON report, plus `comparison.json`. The
comparison records verifier path and binary hash, both legs' exit codes and
integrity/analysis/delivery results, equality of validated record count/bytes/
head, observation line count and hash, full report hash, and paths of summary
metadata differences. Observation lines are compared in original order and
include `unknown` observations. No observations are deduplicated.
It validates each line as an observation followed by exactly one terminal
summary, and hashes/counts lines as it reads them. For each input, the summary's
validated record count, byte count, and head must match that same input's
integrity report; its exit code must also match the verifier process.

Exit codes are `0` only when all four verifier runs succeed and verified chain
identity and observation replay agree; `1` for incomplete/degraded evidence or
a comparison mismatch; `2` when either leg reports invalid evidence; and `3`
for invocation, process, or report failures. Full-report and delivery
differences are included for review, but do not change a successful chain and
observation comparison into retrieval success. The `remote_retrieval` field
always says the comparison did not establish retrieval. Actual staging fetches
remain a separate operation using [`fetch-run.sh`](../../scripts/fetch-run.sh)
and its logged retrieval evidence.

The helper reads and compares supplied artifacts. It does not fetch, repair,
rewrite, or deduplicate trace data, and its result is not evidence that a
particular server returned the rebuilt trace.
