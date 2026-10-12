#!/bin/sh
# Run every local check for one connector and print one PASS/FAIL/SKIPPED line per step.
#
# Usage:
#     sh scripts/check-connector.sh [--allow-no-kit] <connector-id>
#
# Steps, in order (all run, so one report shows every problem):
#   1. files: the kit case crates/goalcup-cli/tests/fixtures/harness-kit/<id>/ holds
#      harness.toml, stand-in.py and expect.toml, and no template placeholder is left: no
#      `<...>` outside comments in the three TOML files, no `<` in the stand-in's EDIT block.
#      A missing connectors/<id>/harness.toml is a usage error (exit 2) before anything runs.
#   2. pins: `sh scripts/pin-manifests.sh`, then the pin diff against HEAD is printed. It fails
#      when the diff touches another connector's line; a new or changed line for <id> is
#      expected and must be committed with the connector.
#   3. schema: `cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::`.
#   4. goldens: the CLI and daemon `harness_contract_golden` tests, without regeneration (an
#      exported GOALCUP_REGEN_HARNESS_GOLDEN is unset first). On a failure the regeneration
#      command is printed; review its diff before committing.
#   5. kit (macOS only): `cargo build`, then `cargo test -p goalcup-cli --test full
#      harness_contract_kit::`. Elsewhere the kit compiles to zero tests, so the step is SKIPPED
#      and the script exits non-zero unless --allow-no-kit is given.
#
# Exit status: 0 when every step passed (or only the kit was skipped with --allow-no-kit),
# 1 when a step failed or the kit was skipped without --allow-no-kit, 2 on a usage error.
# Cargo honours CARGO_TARGET_DIR and the rest of its environment as usual.
set -u
# The script's own path, made absolute before the `cd` so `--help` can still read the header.
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P) || exit 2
self=$script_dir/$(basename "$0")
CDPATH='' cd -- "$(dirname -- "$0")/.." || exit 2
# "Without regeneration" must hold even when the caller's shell still exports the variable
# (the skill tells contributors to set it for one command): with it set, the golden tests
# rewrite a differing golden and pass, so a broken golden would be reported as PASS.
unset GOALCUP_REGEN_HARNESS_GOLDEN

usage() {
  echo "usage: sh scripts/check-connector.sh [--allow-no-kit] <connector-id>" >&2
  exit 2
}

allow_no_kit=no
id=''
for arg in "$@"; do
  case $arg in
    --allow-no-kit) allow_no_kit=yes ;;
    -h | --help)
      # The comment block after the shebang, up to the first line that is not a comment.
      awk 'NR == 1 { next } !/^#/ { exit } { print }' "$self" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*)
      echo "check-connector: unknown option '$arg'" >&2
      usage
      ;;
    *)
      if [ -n "$id" ]; then
        echo "check-connector: one connector id only (got '$id' and '$arg')" >&2
        usage
      fi
      id=$arg
      ;;
  esac
done
[ -n "$id" ] || usage
# The schema's id grammar: ^[a-z][a-z0-9-]{1,47}$.
case $id in
  [a-z]*) ;;
  *) echo "check-connector: '$id' is not a connector id ([a-z][a-z0-9-]{1,47})" >&2; usage ;;
esac
case $id in
  *[!a-z0-9-]*) echo "check-connector: '$id' is not a connector id ([a-z][a-z0-9-]{1,47})" >&2
    usage ;;
esac
if [ ${#id} -lt 2 ] || [ ${#id} -gt 48 ]; then
  echo "check-connector: '$id' is not a connector id ([a-z][a-z0-9-]{1,47})" >&2
  usage
fi

if [ ! -f "connectors/$id/harness.toml" ]; then
  echo "check-connector: no connectors/$id/harness.toml; ids here:" \
    "$(ls connectors 2>/dev/null | tr '\n' ' ')" >&2
  exit 2
fi

# One target directory per checkout: cargo does not put the checkout path in a workspace
# crate's build hash, so a CARGO_TARGET_DIR shared with another checkout can run that
# checkout's test binaries, which then read and write the other checkout's files.
# Cargo resolves a relative CARGO_TARGET_DIR against its working directory, which is the repo
# root here. A target-dir set in a cargo config file is not detected.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  target=$CARGO_TARGET_DIR
  case $target in
    /*) ;;
    *) target=$PWD/$target ;;
  esac
  # Resolve symlinks and `..` when the directory exists; a missing one with `..` warns.
  if [ -d "$target" ]; then
    target=$(cd "$target" && pwd -P)
  fi
  here=$(pwd -P)
  case $target in
    */../* | */..) inside=no ;;
    "$PWD" | "$PWD"/* | "$here" | "$here"/*) inside=yes ;;
    *) inside=no ;;
  esac
  case $inside in
    yes) ;;
    *)
      echo "check-connector: WARNING: CARGO_TARGET_DIR=$CARGO_TARGET_DIR is outside this checkout;"
      echo "  if another checkout or worktree uses it too, results may come from that checkout"
      ;;
  esac
fi

passed=0
failed=0
skipped=0
summary=''
note() { summary="$summary
$1"; }
pass() { passed=$((passed + 1)); note "check-connector: PASS $1"; echo "== PASS $1"; }
fail() { failed=$((failed + 1)); note "check-connector: FAIL $1"; echo "== FAIL $1"; }
step() { echo; echo "== $1"; }

# 1. Files.
step "files: the kit case for this connector"
kit=crates/goalcup-cli/tests/fixtures/harness-kit/$id
missing=''
for f in "$kit/harness.toml" "$kit/stand-in.py" "$kit/expect.toml"; do
  [ -f "$f" ] || missing="$missing $f"
done
if [ -z "$missing" ]; then
  # Placeholders left from the templates, for example the stand-in's b"kit <HARNESS_ID> echo".
  left=$(grep -nE '^[^#]*<[^>]+>' "connectors/$id/harness.toml" "$kit/harness.toml" \
    "$kit/expect.toml"
    awk '/^# ---- EDIT/ { e = 1 } /^# ---- end EDIT/ { e = 0 }
      e && /^[^#]*</ { print FILENAME ":" FNR ":" $0 }' "$kit/stand-in.py")
  if [ -z "$left" ]; then
    pass "files"
  else
    echo "template placeholders left:"
    echo "$left"
    fail "files (template placeholders left)"
  fi
else
  echo "missing:$missing"
  echo "copy the templates in skills/adding-a-capture-harness/templates/ (see SKILL.md)"
  fail "files (missing:$missing)"
fi

# 2. Pins.
step "pins: sh scripts/pin-manifests.sh"
pins="crates/goalcup-protocol/pins/connector-dirs.txt
crates/goalcup-verifier/pins/connector-manifests.txt"
if sh scripts/pin-manifests.sh; then
  # shellcheck disable=SC2086
  if ! diff=$(git diff HEAD -- $pins 2>/dev/null); then
    echo "git diff is unavailable here, so the pin diff was not checked"
    pass "pins (regenerated; diff not checked)"
  elif [ -z "$diff" ]; then
    echo "no pin change against HEAD"
    pass "pins"
  else
    echo "$diff"
    others=$(printf '%s\n' "$diff" | sed -n 's/^[+-]\([a-z][a-z0-9-]*\) [0-9a-f]\{64\}$/\1/p' |
      grep -v -x -F -- "$id" | sort -u | tr '\n' ' ')
    if [ -n "$others" ]; then
      echo "the pin diff also changes: $others"
      # shellcheck disable=SC2086
      changed=$(git diff --name-only HEAD -- $pins | tr '\n' ' ')
      echo "pin files were regenerated in place; \`git checkout -- ${changed% }\` undoes that,"
      echo "and also drops your own uncommitted pin line: re-run this script afterwards"
      fail "pins (other connectors changed: $others)"
    else
      echo "pin changes above are for '$id' only: commit them with the connector"
      pass "pins (uncommitted change for $id)"
    fi
  fi
else
  fail "pins (scripts/pin-manifests.sh failed)"
fi

# 3. Schema and pin tests.
step "schema: cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::"
if cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::; then
  pass "schema"
else
  fail "schema (field rules: docs/CONNECTOR-SCHEMA.md)"
fi

# 4. Goldens, without regeneration. The output is kept so the failing files can be named.
log=$(mktemp "${TMPDIR:-/tmp}/check-connector.XXXXXX")
trap 'rm -f "$log"' EXIT
for crate in goalcup-cli goalcup-daemon; do
  step "goldens: cargo test -p $crate --test full harness_contract_golden::"
  { cargo test -p "$crate" --test full harness_contract_golden:: 2>&1; echo "exit=$?"; } | tee "$log"
  if [ "$(tail -n 1 "$log")" = exit=0 ]; then
    pass "goldens ($crate)"
  else
    files=$(awk '/ differs from what the code now produces/ || / is missing \(/ {
      n = $1; sub(/.*\//, "", n); print n }' "$log" | sort -u | tr '\n' ' ')
    echo "golden files that differ: ${files:-(see the output above)}"
    echo "a file for '$id' (<id>.json, or a row for '$id'): regenerate, then review the diff:"
    echo "  GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p $crate --test full harness_contract_golden::"
    echo "a file that does not mention '$id' did not change because of your connector: do not"
    echo "commit its regenerated form; restore it with git checkout and name it in the PR"
    fail "goldens ($crate): ${files:-see output}"
  fi
done

# 5. The harness kit, macOS only.
os=$(uname -s 2>/dev/null || echo unknown)
if [ "$os" = Darwin ]; then
  step "kit: cargo build && cargo test -p goalcup-cli --test full harness_contract_kit::"
  if cargo build && cargo test -p goalcup-cli --test full harness_contract_kit::; then
    pass "kit"
  else
    fail "kit"
  fi
else
  skipped=$((skipped + 1))
  echo
  echo "################################################################################"
  echo "SKIPPED: harness kit is macOS-only; a maintainer runs it"
  echo "  ($os: harness_contract_kit compiles to zero tests here, and the daemon's live"
  echo "  goldens <id>.daemon.json and native-stream-labels.json are macOS-only too)"
  echo "################################################################################"
  note "check-connector: SKIPPED kit (macOS-only; a maintainer runs it)"
  if [ "$allow_no_kit" = no ]; then
    failed=$((failed + 1))
    note "check-connector: FAIL kit skipped without --allow-no-kit"
  fi
fi

printf '%s\n' "$summary"
echo "check-connector: $id: $passed passed, $failed failed, $skipped skipped"
[ "$failed" -eq 0 ] || exit 1
exit 0
