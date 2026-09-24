#!/usr/bin/env sh
# Tauri beforeDev/beforeBuild 用。lockfile と利用可能な PM から起動する。
set -e
cd "$(dirname "$0")/.."

run_with() {
  pm="$1"
  shift
  if command -v "$pm" >/dev/null 2>&1; then
    exec "$pm" run "$@"
  fi
}

if [ -f bun.lock ] || [ -f bun.lockb ]; then
  run_with bun "$@"
fi
if [ -f pnpm-lock.yaml ]; then
  run_with pnpm "$@"
fi
if [ -f package-lock.json ]; then
  run_with npm "$@"
fi

run_with bun "$@"
run_with npm "$@"
run_with pnpm "$@"

echo "[tauri-run] bun, npm, or pnpm is required to run: $*" >&2
exit 1
