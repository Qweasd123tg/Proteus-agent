#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
bin_dir="${HOME}/.local/bin"
bin_path="${bin_dir}/agent"
tui_bin_path="${bin_dir}/agent-tui"
plugins_dir="${HOME}/.agent/plugins"

cargo build --release --manifest-path "${project_dir}/Cargo.toml" --features context-pack/plugin-entrypoint,memory-pack/plugin-entrypoint,policy-pack/plugin-entrypoint,renderer-pack/plugin-entrypoint

mkdir -p "${bin_dir}"
cat > "${bin_path}" <<EOF
#!/usr/bin/env sh
exec "${project_dir}/target/release/modular-agent" "\$@"
EOF
chmod 755 "${bin_path}"

cat > "${tui_bin_path}" <<EOF
#!/usr/bin/env sh
exec "${project_dir}/target/release/agent-tui" "\$@"
EOF
chmod 755 "${tui_bin_path}"

# Install plugins. File I/O, git helpers, and shell are required for a typical
# coding workflow; other sample plugins are optional proofs.
mkdir -p "${plugins_dir}"
install_plugin() {
  plugin="$1"
  source_dir="$2"
  src_so="${project_dir}/target/release/lib$(printf '%s' "${plugin}" | tr '-' '_').so"
  if [ ! -f "${src_so}" ]; then
    return
  fi
  dest_dir="${plugins_dir}/${plugin}"
  mkdir -p "${dest_dir}"
  cp "${src_so}" "${dest_dir}/"
  if [ -f "${project_dir}/${source_dir}/plugin.toml" ]; then
    cp "${project_dir}/${source_dir}/plugin.toml" "${dest_dir}/"
  fi
}

for plugin in file-tools git-tools shell-tool rg-search direct-patch coding-workflow context-pack memory-pack policy-pack renderer-pack hello-renderer hello-tool hello-policy-patch sqlite-memory; do
  install_plugin "${plugin}" "plugins/default/${plugin}"
done

echo "Installed: ${bin_path}"
echo "Installed: ${tui_bin_path}"
echo "Plugins:   ${plugins_dir}"
echo "Next:      ${bin_path} init coding && ${bin_path} doctor"
case ":\${PATH}:" in
  *:"${bin_dir}":*) ;;
  *) echo "Add this to your shell config if needed: export PATH=\"${bin_dir}:\\\$PATH\"" ;;
esac
