#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
bin_dir="${PROTEUS_BIN_DIR:-${HOME}/.local/bin}"
bin_path="${bin_dir}/proteus"
proteus_home="${PROTEUS_HOME:-${HOME}/.proteus}"
releases_dir="${proteus_home}/releases"
current_release="${proteus_home}/current"
config_home="${PROTEUS_CONFIG_HOME:-${HOME}/.config/Proteus-agent}"
configs_dir="${config_home}/configs"

cargo build --release --manifest-path "${project_dir}/Cargo.toml" \
  -p proteus-core \
  -p proteus-reference-module

mkdir -p "${bin_dir}"
bin_tmp="${bin_path}.tmp.$$"
module_tmp="${bin_dir}/proteus-reference-module.tmp.$$"
release_id=$(date -u +%Y%m%dT%H%M%SZ)-$$
release_tmp="${releases_dir}/.${release_id}.tmp"
release_dir="${releases_dir}/${release_id}"
current_tmp="${proteus_home}/.current.$$"
release_published=0
rm -f "${bin_tmp}" "${module_tmp}" "${current_tmp}"
rm -rf "${release_tmp}"

cleanup_install() {
  status=$?
  trap - EXIT HUP INT TERM
  set +e
  rm -f "${bin_tmp}" "${module_tmp}" "${current_tmp}"
  rm -rf "${release_tmp}"
  if [ "${release_published}" -eq 0 ]; then
    rm -rf "${release_dir}"
  fi
  exit "${status}"
}

trap cleanup_install EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
cat > "${bin_tmp}" <<'WRAPPER'
#!/usr/bin/env sh
set -eu
proteus_home="${PROTEUS_HOME:-${HOME}/.proteus}"
export PATH="${proteus_home}/current:${PATH}"
exec "${proteus_home}/current/proteus" "$@"
WRAPPER
chmod 755 "${bin_tmp}"

# The provider's management commands use the same atomically selected snapshot.
cat > "${module_tmp}" <<'MODULE_WRAPPER'
#!/usr/bin/env sh
set -eu
proteus_home="${PROTEUS_HOME:-${HOME}/.proteus}"
exec "${proteus_home}/current/proteus-reference-module" "$@"
MODULE_WRAPPER
chmod 755 "${module_tmp}"

# Stage the host and reference module before the `current` symlink makes the
# build snapshot visible.
mkdir -p "${release_tmp}"
cp "${project_dir}/target/release/proteus" "${release_tmp}/proteus"
cp "${project_dir}/target/release/proteus-reference-module" "${release_tmp}/proteus-reference-module"
chmod 755 "${release_tmp}/proteus"
chmod 755 "${release_tmp}/proteus-reference-module"

mkdir -p "${releases_dir}"
mv "${release_tmp}" "${release_dir}"
ln -s "releases/${release_id}" "${current_tmp}"

# GNU `mv -T` and BSD `mv -h` spell the same atomic symlink replacement
# differently. Refuse a non-atomic unlink/link fallback on unknown platforms.
replace_current_link() {
  if [ ! -e "${current_release}" ] && [ ! -L "${current_release}" ]; then
    mv "${current_tmp}" "${current_release}"
    return
  fi
  if mv -Tf "${current_tmp}" "${current_release}" 2>/dev/null; then
    return
  fi
  if mv -h -f "${current_tmp}" "${current_release}" 2>/dev/null; then
    return
  fi
  echo "Cannot atomically replace ${current_release}: mv supports neither GNU -T nor BSD -h" >&2
  return 1
}

# Do not run a signal trap in the single command/builtin window between the
# atomic rename and the state bit used by cleanup_install.
trap '' HUP INT TERM
replace_current_link
release_published=1
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

mv "${bin_tmp}" "${bin_path}"
mv "${module_tmp}" "${bin_dir}/proteus-reference-module"
rm -f "${bin_dir}/proteus-reference-worker"

trap - EXIT HUP INT TERM

mkdir -p "${configs_dir}"
install_config() {
  dest_name="$1"
  source_path="$2"
  dest_path="${configs_dir}/${dest_name}"
  if [ -e "${dest_path}" ]; then
    return
  fi
  cp "${project_dir}/${source_path}" "${dest_path}"
}

install_config "codex.config.toml" "configs/codex.config.toml"
install_config "codex-chatgpt.config.toml" "configs/codex-chatgpt.config.toml"
install_config "context-search-chatgpt.config.toml" "configs/context-search-chatgpt.config.toml"
install_config "codex-chatgpt-explore.config.toml" "configs/codex-chatgpt-explore.config.toml"
install_config "codex-chatgpt-coder.config.toml" "configs/codex-chatgpt-coder.config.toml"
install_config "codex-explore.config.toml" "configs/codex-explore.config.toml"
install_config "codex-coder.config.toml" "configs/codex-coder.config.toml"
install_config "opencode.config.toml" "configs/opencode.config.toml"
install_config "proteus.provider.example.toml" "configs/proteus.provider.example.toml"

# Managed fragments и prompt-файлы обновляются при каждой установке: это код
# профиля, а не пользовательские правки (в отличие от named configs, которые
# не перезаписываются).
mkdir -p "${configs_dir}/fragments"
mkdir -p "${configs_dir}/prompts"
install_managed_config_asset() {
  relative_path="$1"
  source_path="${project_dir}/configs/${relative_path}"
  dest_path="${configs_dir}/${relative_path}"
  # configs_dir может быть симлинком на репозиторный configs/ — тогда source
  # и dest являются одним файлом и копирование не нужно.
  if [ "${source_path}" -ef "${dest_path}" ]; then
    return
  fi
  cp "${source_path}" "${dest_path}"
}
install_managed_config_asset "fragments/openai-proxy.toml"
install_managed_config_asset "fragments/openai-chatgpt.toml"
install_managed_config_asset "fragments/codex-runtime.toml"
install_managed_config_asset "fragments/codex-profile.toml"
install_managed_config_asset "fragments/codex-peer-runtime.toml"
install_managed_config_asset "fragments/codex-explore-peer.toml"
install_managed_config_asset "fragments/codex-coder-peer.toml"
install_managed_config_asset "prompts/codex-default.md"
install_managed_config_asset "prompts/codex-explore.md"
install_managed_config_asset "prompts/codex-coder.md"
install_managed_config_asset "prompts/opencode-default.md"
install_managed_config_asset "prompts/direct-patch.md"

# Optional presentation skill; preserve a user's own version with the same name.
mkdir -p "${proteus_home}/skills"
if [ ! -e "${proteus_home}/skills/interactive-response" ]; then
  cp -R "${project_dir}/configs/skills/interactive-response" "${proteus_home}/skills/"
fi

echo "Installed: ${bin_path}"
echo "Snapshot:  ${release_dir}"
echo "Module:    ${current_release}/proteus-reference-module"
echo "Configs:   ${configs_dir}"
echo "Next:      ${bin_path} --config codex-chatgpt doctor"
case ":${PATH}:" in
  *:"${bin_dir}":*) ;;
  *) echo "Add this to your shell config if needed: export PATH=\"${bin_dir}:\$PATH\"" ;;
esac
