#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
case "${1:-}" in
  dev|build) ;;
  *) echo "Использование: ./scripts/desktop.sh dev|build" >&2; exit 2 ;;
esac
cd "$project_dir/clients/app"
node scripts/dependencies.mjs
npm run "$1"
