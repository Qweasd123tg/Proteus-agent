#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
canary_root=$(mktemp -d "${TMPDIR:-/tmp}/proteus-install-canary.XXXXXX")
cleanup() {
  case "${canary_root}" in
    */proteus-install-canary.*) rm -rf -- "${canary_root}" ;;
    *) echo "Unexpected canary directory: ${canary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

# Named-profile lookup must succeed beside the inherited override, otherwise
# the old smoke exits at doctor before reaching the unsafe init operation.
cp -R "${project_dir}/configs/." "${canary_root}/"
canary_config="${canary_root}/config.toml"
cp "${canary_config}" "${canary_root}/original.toml"

status=0
PROTEUS_CONFIG_PATH="${canary_config}" \
  sh "${project_dir}/scripts/install-smoke.sh" || status=$?
if ! cmp -s "${canary_config}" "${canary_root}/original.toml"; then
  echo "install smoke changed the inherited external configuration" >&2
  exit 1
fi
if [ "${status}" -ne 0 ]; then
  exit "${status}"
fi
echo "install smoke preserved the inherited external configuration"
