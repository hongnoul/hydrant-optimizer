#!/usr/bin/env bash
# Native Unix/macOS verification. Requires Bash, Cargo, rustfmt, and Clippy.
# Installs only into a fresh isolated directory, never into the user's PATH.
set -euo pipefail

if (( $# > 1 )); then echo "Expected at most one option" >&2; exit 2; fi
live=false
case "${1:-}" in
  "") ;;
  --live) live=true ;;
  --help|-h)
    echo "Usage: bash scripts/verify-mvp.sh [--live]"
    echo "Default: local fixtures, native PTY, build/lint, isolated installed binary."
    echo "--live: also fetch public Hydrant data and run all three opt-in checks."
    echo "Logs: HYDRANT_VERIFY_DIR, JCODE_SCRATCH_DIR, or target/verification."
    exit 0 ;;
  *) echo "Unknown option: $1" >&2; exit 2 ;;
esac

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo"
base=${HYDRANT_VERIFY_DIR:-${JCODE_SCRATCH_DIR:-$repo/target/verification}}
mkdir -p "$base"
base=$(cd "$base" && pwd)
evidence=$(mktemp -d "$base/mvp.XXXXXX")
mkdir -p "$evidence/tmp" "$evidence/home"
export TMPDIR="$evidence/tmp"
export CARGO_TARGET_DIR="$repo/target"
trap 'code=$?; echo "Verification exit: $code. Evidence retained at: $evidence"' EXIT
printf 'check\texpected_exit\tactual_exit\n' > "$evidence/checks.tsv"
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$evidence/started-at.txt"
if command -v git >/dev/null 2>&1; then
  git rev-parse HEAD > "$evidence/revision.txt" 2>/dev/null || true
  git status --porcelain > "$evidence/worktree.txt" 2>/dev/null || true
fi

run() {
  local label=$1 expected=$2 code=0
  shift 2
  echo "Checking $label"
  "$@" > "$evidence/$label.stdout" 2> "$evidence/$label.stderr" || code=$?
  cat "$evidence/$label.stdout"
  cat "$evidence/$label.stderr" >&2
  printf '%s\t%s\t%s\n' "$label" "$expected" "$code" >> "$evidence/checks.tsv"
  if (( code != expected )); then
    echo "FAIL: $label returned $code, expected $expected" >&2
    exit 1
  fi
}

run format 0 cargo fmt --check
run tests 0 cargo test --locked
run clippy 0 cargo clippy --all-targets --locked -- -D warnings
run release 0 cargo build --release --locked
run release-regressions 0 cargo test --release --locked --test cli_workflow --test tui_acceptance
run install 0 cargo install --path . --locked --offline --root "$evidence/install"
installed="$evidence/install/bin/hydrant-optimizer"
isolated=(env -i "HOME=$evidence/home" PATH=/usr/bin:/bin "$installed")
fixture=(--data-dir "$evidence/data" --catalog "$repo/tests/fixtures/catalog.json" --term "$repo/tests/fixtures/term.json")
run installed-version 0 "${isolated[@]}" --version
run installed-export 0 "${isolated[@]}" "${fixture[@]}" --json --output "$evidence/fixture.ics" optimize A B --export
grep -q '"status": "optimal_known"' "$evidence/installed-export.stdout"
events=$(grep -c '^BEGIN:VEVENT' "$evidence/fixture.ics")
[[ $events == 6 ]]
cp "$evidence/fixture.ics" "$evidence/original.ics"
for format in text json; do
  args=(--output "$evidence/fixture.ics" optimize A C --export)
  if [[ $format == json ]]; then args+=(--json); fi
  run "infeasible-$format" 2 "${isolated[@]}" "${fixture[@]}" "${args[@]}"
  if [[ $format == json ]]; then
    grep -q '"status": "infeasible"' "$evidence/infeasible-json.stdout"
    grep -q '"export": null' "$evidence/infeasible-json.stdout"
  else
    grep -q 'Infeasible:' "$evidence/infeasible-text.stdout"
  fi
  cmp "$evidence/original.ics" "$evidence/fixture.ics"
done
run infeasible-new-file 2 "${isolated[@]}" "${fixture[@]}" --output "$evidence/missing.ics" optimize A C --export
[[ ! -e "$evidence/missing.ics" ]]

if $live; then
  run fresh-fetch 0 "${isolated[@]}" --data-dir "$evidence/live" --json refresh
  # Force this invocation's fresh cache, not a caller's retained snapshot.
  unset HYDRANT_REAL_CATALOG HYDRANT_REAL_TERM
  export HYDRANT_REAL_DATA_DIR="$evidence/live"
  run opt-in 0 cargo test --release --locked -- --ignored --nocapture
  run installed-live-export 0 "${isolated[@]}" --data-dir "$evidence/live" --offline --json --output "$evidence/live.ics" optimize 6.1200 18.01 --export
  grep -q '"status": "optimal_known"' "$evidence/installed-live-export.stdout"
  events=$(grep -c '^BEGIN:VEVENT' "$evidence/live.ics")
  (( events > 0 ))
  echo "Installed live workflow exported $events events."
else
  echo "SKIP: live Hydrant and snapshot checks (enable with --live)."
fi
echo "PASS: all selected MVP verification checks completed."
