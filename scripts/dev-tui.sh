#!/usr/bin/env bash
# Source-watching TUI launcher for macOS and Linux. No watcher installation needed.
set -eu
cd "$(dirname "$0")/.."
if [[ ${1:-} == --help ]]; then
  echo 'Usage: ./scripts/dev-tui.sh [hydrant-optimizer arguments...]'
  echo 'Watches src/, Cargo.toml, Cargo.lock, and .cargo/. q quits. Ctrl+C stops.'
  exit 0
fi
if [[ ! -t 0 || ! -t 1 ]]; then
  echo 'dev-tui requires an interactive terminal.' >&2
  exit 1
fi
saved_tty=$(stty -g)
# Preserve the actual terminal descriptor (opening /dev/tty breaks macOS polling).
exec 3<&0
child=''
builder=''
# Fix the output directory so CARGO_TARGET_DIR cannot make us launch a stale binary.
target="$PWD/target/tui-dev"
mkdir -p "$target"
build_log="$target/build.log"
run_dir=$(mktemp -d "$target/session.XXXXXX")
restore_terminal() {
  stty "$saved_tty" 2>/dev/null || true
  printf '\033[?1000l\033[?1002l\033[?1003l\033[?1015l\033[?1006l\033[?1049l\033[?25h\033[0m'
}
stop_child() {
  if [[ -n $child ]]; then
    kill "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
    child=''
    restore_terminal
  fi
}
cleanup() {
  trap - EXIT INT TERM HUP
  if [[ -n $builder ]]; then
    kill "$builder" 2>/dev/null || true
    wait "$builder" 2>/dev/null || true
  fi
  stop_child
  restore_terminal
  rm -f "$run_dir/app"
  rmdir "$run_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
fingerprint() {
  # Content hashes detect atomic saves, deletions and same-timestamp edits.
  { printf '%s\n' Cargo.toml Cargo.lock
    find src -type f
    if [[ -d .cargo ]]; then find .cargo -type f; fi
  } | LC_ALL=C sort | while IFS= read -r file; do
    if [[ -f $file ]]; then cksum "$file"; fi
  done | cksum
}
last=''
while true; do
  # Honor quit even if a compilation is still running.
  if [[ -n $child ]] && ! kill -0 "$child" 2>/dev/null; then
    code=0
    wait "$child" || code=$?
    child=''
    exit "$code"
  fi
  if [[ -n $builder ]] && ! kill -0 "$builder" 2>/dev/null; then
    code=0
    wait "$builder" || code=$?
    builder=''
    if [[ $code == 0 && $(fingerprint) == "$last" ]]; then
      stop_child
      # Never execute the build output directly: linking/copying a new build
      # must not overwrite the executable still displayed in the terminal.
      cp "$target/debug/hydrant-optimizer" "$run_dir/app"
      "$run_dir/app" "$@" <&3 &
      child=$!
    elif [[ $code != 0 ]]; then
      printf '\nBuild failed. Keeping the previous TUI. Save to retry.\n' >> "$build_log"
    fi
  fi
  current=$(fingerprint)
  if [[ -z $builder && $current != "$last" ]]; then
    # Debounce an editor/agent's burst of writes.
    sleep 0.3
    last=$(fingerprint)
    cargo build --locked --bin hydrant-optimizer --target-dir "$target" >"$build_log" 2>&1 &
    builder=$!
  fi
  sleep 0.1
done
