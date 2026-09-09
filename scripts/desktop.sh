#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_dir/clients/desktop"
case "${1:-}" in
  dev|build) ;;
  *) echo "Использование: ./scripts/desktop.sh dev|build" >&2; exit 2 ;;
esac
if [[ "$1" == build || ! -x node_modules/.bin/tauri || package-lock.json -nt node_modules/.package-lock.json ]]; then
  npm ci --no-audit --no-fund
fi
npm run "$1"
