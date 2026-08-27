#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/proteus-alpha-smoke.XXXXXX")
bin_dir="${smoke_root}/bin"
runtime_home="${smoke_root}/runtime"
config_home="${smoke_root}/config"
output_dir="${smoke_root}/output"

cleanup() {
  case "${smoke_root}" in
    */proteus-alpha-smoke.*) rm -rf -- "${smoke_root}" ;;
    *) echo "Refusing to remove unexpected smoke directory: ${smoke_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

mkdir -p "${output_dir}"
export PROTEUS_BIN_DIR="${bin_dir}"
export PROTEUS_HOME="${runtime_home}"
export PROTEUS_CONFIG_HOME="${config_home}"

run_and_capture() {
  label=$1
  output=$2
  shift 2
  if "$@" >"${output}" 2>&1; then
    return
  fi
  echo "alpha smoke failed during ${label}" >&2
  sed -n '1,240p' "${output}" >&2
  exit 1
}

require_text() {
  expected=$1
  output=$2
  if grep -F -- "${expected}" "${output}" >/dev/null; then
    return
  fi
  echo "alpha smoke did not find '${expected}' in ${output}" >&2
  sed -n '1,240p' "${output}" >&2
  exit 1
}

run_and_capture install "${output_dir}/install.txt" "${project_dir}/install.sh"

proteus="${bin_dir}/proteus"
test -x "${proteus}"
test -L "${runtime_home}/current"
test -x "${runtime_home}/current/proteus"
test -x "${runtime_home}/current/proteus-reference-worker"
test -f "${config_home}/configs/codex-explore.config.toml"
test -f "${config_home}/configs/codex-coder.config.toml"
test -f "${config_home}/configs/fragments/codex-peer-runtime.toml"
test -f "${config_home}/configs/fragments/codex-explore-peer.toml"
test -f "${config_home}/configs/fragments/codex-coder-peer.toml"
test -f "${config_home}/configs/prompts/codex-explore.md"
test -f "${config_home}/configs/prompts/codex-coder.md"

if find -H "${runtime_home}/current" -maxdepth 1 -type f \
  \( -name '*.so' -o -name '*.dylib' -o -name '*.dll' \) | grep . >/dev/null; then
  echo "alpha release unexpectedly contains a native extension library" >&2
  exit 1
fi

run_and_capture version "${output_dir}/version.txt" "${proteus}" --version
require_text "proteus 0.1.0-alpha.1" "${output_dir}/version.txt"

run_and_capture init "${output_dir}/init.txt" "${proteus}" init safe
test -f "${config_home}/configs/config.toml"

run_and_capture doctor "${output_dir}/doctor.txt" "${proteus}" doctor
require_text "config loaded" "${output_dir}/doctor.txt"
require_text "process component" "${output_dir}/doctor.txt"

run_and_capture assembly-plan "${output_dir}/assembly-plan.txt" \
  "${proteus}" inspect plan
require_text "Assembly plan v2" "${output_dir}/assembly-plan.txt"
require_text "status: ready" "${output_dir}/assembly-plan.txt"
require_text "workflow: coding.single_loop" "${output_dir}/assembly-plan.txt"

run_and_capture topology "${output_dir}/topology.txt" \
  "${proteus}" inspect topology --format runtime
require_text "workflow        -> coding.single_loop" "${output_dir}/topology.txt"
require_text "[process" "${output_dir}/topology.txt"

run_and_capture fake-profile "${output_dir}/fake-profile.txt" \
  "${proteus}" --cwd "${project_dir}" "alpha smoke"
require_text "Fake final answer." "${output_dir}/fake-profile.txt"

external_config="${config_home}/configs/python-agent.toml"
cp "${project_dir}/examples/configs/proteus.process-agent.example.toml" "${external_config}"

run_and_capture external-doctor "${output_dir}/external-doctor.txt" \
  "${proteus}" --config "${external_config}" --cwd "${project_dir}" doctor
require_text "process component python-agent" "${output_dir}/external-doctor.txt"

run_and_capture external-topology "${output_dir}/external-topology.txt" \
  "${proteus}" --config "${external_config}" --cwd "${project_dir}" \
  inspect topology --format runtime
require_text "workflow        -> python_agent_loop" "${output_dir}/external-topology.txt"

run_and_capture external-component "${output_dir}/external-component.txt" \
  "${proteus}" --config "${external_config}" --cwd "${project_dir}" \
  "alpha external component demo"
require_text "Fake final answer." "${output_dir}/external-component.txt"

run_and_capture collaboration-tools "${output_dir}/collaboration-tools.txt" \
  "${proteus}" --config codex tools list
require_text "spawn_agent" "${output_dir}/collaboration-tools.txt"
require_text "send_message" "${output_dir}/collaboration-tools.txt"
require_text "followup_task" "${output_dir}/collaboration-tools.txt"

run_and_capture collaboration-process "${output_dir}/collaboration-process.txt" \
  env PROTEUS_TEST_BINARY="${proteus}" cargo test \
  --manifest-path "${project_dir}/Cargo.toml" \
  -p proteus-core --test process_agent_control \
  process_agents_route_bounded_messages_without_cross_delivery -- --exact
require_text "test result: ok" "${output_dir}/collaboration-process.txt"

run_and_capture process-peer-surfaces "${output_dir}/process-peer-surfaces.txt" \
  env PROTEUS_TEST_BINARY="${proteus}" cargo test \
  --manifest-path "${project_dir}/Cargo.toml" \
  -p proteus-core --test process_agent_pool \
  process_peers_derive_distinct_tool_surfaces_from_child_configs -- --exact
require_text "test result: ok" "${output_dir}/process-peer-surfaces.txt"

echo "alpha smoke passed: isolated install, init, doctor, assembly plan, fake profile, topology, external Python component, process-agent messaging and peer-owned tool surfaces"
