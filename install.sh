#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
bin_dir="${HOME}/.local/bin"
bin_path="${bin_dir}/proteus"
plugins_dir="${HOME}/.proteus/plugins"

cargo build --release --manifest-path "${project_dir}/Cargo.toml" \
  -p proteus-core \
  -p file-tools \
  -p git-tools \
  -p shell-tool \
  -p rg-search \
  -p direct-patch \
  -p coding-workflow \
  -p context-pack \
  -p memory-pack \
  -p policy-pack \
  -p renderer-pack \
  -p sqlite-memory \
  --features context-pack/plugin-entrypoint,memory-pack/plugin-entrypoint,policy-pack/plugin-entrypoint,renderer-pack/plugin-entrypoint

mkdir -p "${bin_dir}"
cat > "${bin_path}" <<EOF
#!/usr/bin/env bash
set -euo pipefail

project_dir="${project_dir}"
proteus_bin="\${project_dir}/target/release/proteus"
web_dir="\${project_dir}/clients/web"
app_port=8787
web_port="\${PROTEUS_WEB_PORT:-1420}"
session_token="\${PROTEUS_SESSION_TOKEN:-\$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')}"

if [ ! -x "\${proteus_bin}" ]; then
  echo "Proteus binary is missing; building release binary..." >&2
  "\${project_dir}/install.sh"
elif find "\${project_dir}/crates" "\${project_dir}/plugins/default" "\${project_dir}/Cargo.toml" "\${project_dir}/Cargo.lock" -newer "\${proteus_bin}" -print -quit | grep -q .; then
  echo "Proteus binary is stale; rebuilding release binary..." >&2
  "\${project_dir}/install.sh"
fi

if [ "\$#" -gt 0 ]; then
  exec "\${proteus_bin}" "\$@"
fi

if ! command -v trunk >/dev/null 2>&1; then
  echo "trunk is not installed. Run: cargo install trunk --locked" >&2
  exit 1
fi

if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
  echo "wasm32 target is missing. Run: rustup target add wasm32-unknown-unknown" >&2
  exit 1
fi

workspace_cwd=\$(pwd)
echo "Proteus workspace: \${workspace_cwd}"
echo "App server:        http://127.0.0.1:\${app_port}"
echo "Web client:        http://127.0.0.1:\${web_port}/?session=<redacted>"
echo

"\${proteus_bin}" --cwd "\${workspace_cwd}" server http --port "\${app_port}" --token "\${session_token}" &
server_pid=\$!

sleep 1
if ! kill -0 "\${server_pid}" >/dev/null 2>&1; then
  wait "\${server_pid}" 2>/dev/null || true
  echo "Proteus app server did not start. Port \${app_port} may already be in use." >&2
  exit 1
fi

cleanup() {
  kill "\${server_pid}" >/dev/null 2>&1 || true
  wait "\${server_pid}" 2>/dev/null || true
}
trap cleanup INT TERM EXIT

cd "\${web_dir}"
open_web_url="http://127.0.0.1:\${web_port}/?session=\${session_token}"
(
  sleep 2
  if command -v xdg-open >/dev/null 2>&1; then
    xdg-open "\${open_web_url}" >/dev/null 2>&1 || true
  elif command -v open >/dev/null 2>&1; then
    open "\${open_web_url}" >/dev/null 2>&1 || true
  fi
) &
env -u NO_COLOR trunk serve --port "\${web_port}"
EOF
chmod 755 "${bin_path}"

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

for plugin in file-tools git-tools shell-tool rg-search direct-patch coding-workflow context-pack memory-pack policy-pack renderer-pack sqlite-memory; do
  install_plugin "${plugin}" "plugins/default/${plugin}"
done

echo "Installed: ${bin_path}"
echo "Plugins:   ${plugins_dir}"
echo "Next:      ${bin_path} init coding && ${bin_path} doctor"
case ":\${PATH}:" in
  *:"${bin_dir}":*) ;;
  *) echo "Add this to your shell config if needed: export PATH=\"${bin_dir}:\\\$PATH\"" ;;
esac
