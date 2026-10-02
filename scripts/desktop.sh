#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_dir/clients/app"
case "${1:-}" in
  dev|build) ;;
  *) echo "Использование: ./scripts/desktop.sh dev|build" >&2; exit 2 ;;
esac
if [[ "$1" == build || ! -x node_modules/.bin/tauri || package-lock.json -nt node_modules/.package-lock.json ]]; then
  npm ci --no-audit --no-fund
fi
render_dir="$project_dir/clients/app/ui/rendering"
if [[ "$1" == build || ! -f "$render_dir/node_modules/.package-lock.json" || "$render_dir/package-lock.json" -nt "$render_dir/node_modules/.package-lock.json" ]]; then
  npm ci --prefix "$render_dir" --ignore-scripts --no-audit --no-fund
fi
npm run "$1"
