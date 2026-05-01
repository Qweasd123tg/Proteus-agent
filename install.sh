#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
bin_dir="${HOME}/.local/bin"
bin_path="${bin_dir}/agent"
plugins_dir="${HOME}/.agent/plugins"

cargo build --release --manifest-path "${project_dir}/Cargo.toml"

mkdir -p "${bin_dir}"
cat > "${bin_path}" <<EOF
#!/usr/bin/env sh
exec "${project_dir}/target/release/modular-agent" "\$@"
EOF
chmod 755 "${bin_path}"

# Install plugins. File I/O (file-tools) and shell (shell-tool) are required
# for a typical coding workflow; other sample plugins are optional proofs.
mkdir -p "${plugins_dir}"
for plugin in file-tools shell-tool rg-search direct-patch hello-renderer hello-tool hello-policy-patch sqlite-memory; do
  src_so="${project_dir}/target/release/lib$(printf '%s' "${plugin}" | tr '-' '_').so"
  if [ ! -f "${src_so}" ]; then
    continue
  fi
  dest_dir="${plugins_dir}/${plugin}"
  mkdir -p "${dest_dir}"
  cp "${src_so}" "${dest_dir}/"
  if [ -f "${project_dir}/plugins/${plugin}/plugin.toml" ]; then
    cp "${project_dir}/plugins/${plugin}/plugin.toml" "${dest_dir}/"
  fi
done

echo "Installed: ${bin_path}"
echo "Plugins:   ${plugins_dir}"
case ":\${PATH}:" in
  *:"${bin_dir}":*) ;;
  *) echo "Add this to your shell config if needed: export PATH=\"${bin_dir}:\\\$PATH\"" ;;
esac
