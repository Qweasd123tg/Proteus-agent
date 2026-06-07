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
bin_tmp="${bin_path}.tmp.$$"
rm -f "${bin_tmp}"
trap 'rm -f "${bin_tmp}"' EXIT HUP INT TERM
cat > "${bin_tmp}" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail

project_dir="__PROTEUS_PROJECT_DIR__"
proteus_bin="${project_dir}/target/release/proteus"
web_dir="${project_dir}/clients/web"
app_port="${PROTEUS_APP_PORT:-8787}"
web_port="${PROTEUS_WEB_PORT:-1420}"
session_token="${PROTEUS_SESSION_TOKEN:-}"

listener_pids_for_port() {
  port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -tiTCP:"${port}" -sTCP:LISTEN 2>/dev/null || true
    return
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltnp "sport = :${port}" 2>/dev/null \
      | sed -n 's/.*pid=\([0-9][0-9]*\).*/\1/p' \
      | sort -u
  fi
}

close_previous_app_server() {
  pids=$(listener_pids_for_port "${app_port}")
  if [ -z "${pids}" ]; then
    return
  fi

  for pid in ${pids}; do
    cmd=$(ps -p "${pid}" -o args= 2>/dev/null || true)
    case "${cmd}" in
      *proteus*" server http "*--port*" ${app_port}"*)
        echo "Closing previous Proteus app server on port ${app_port} (pid ${pid})..."
        kill "${pid}" >/dev/null 2>&1 || true
        ;;
      *)
        echo "Port ${app_port} is already in use by pid ${pid}: ${cmd}" >&2
        echo "Stop that process or set PROTEUS_APP_PORT to another port." >&2
        exit 1
        ;;
    esac
  done

  for _ in {1..30}; do
    if [ -z "$(listener_pids_for_port "${app_port}")" ]; then
      return
    fi
    sleep 0.1
  done

  echo "Previous Proteus app server did not release port ${app_port}." >&2
  exit 1
}

if [ ! -x "${proteus_bin}" ]; then
  echo "Proteus binary is missing; building release binary..." >&2
  "${project_dir}/install.sh"
elif find "${project_dir}/crates" "${project_dir}/plugins/default" "${project_dir}/Cargo.toml" "${project_dir}/Cargo.lock" -newer "${proteus_bin}" -print -quit | grep -q .; then
  echo "Proteus binary is stale; rebuilding release binary..." >&2
  "${project_dir}/install.sh"
fi

if [ "$#" -gt 0 ]; then
  exec "${proteus_bin}" "$@"
fi

if ! command -v trunk >/dev/null 2>&1; then
  echo "trunk is not installed. Run: cargo install trunk --locked" >&2
  exit 1
fi

if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
  echo "wasm32 target is missing. Run: rustup target add wasm32-unknown-unknown" >&2
  exit 1
fi

close_previous_app_server

workspace_cwd=$(pwd)
echo "Proteus workspace: ${workspace_cwd}"
echo "App server:        http://127.0.0.1:${app_port}"
if [ -n "${session_token}" ]; then
  echo "Web client:        http://127.0.0.1:${web_port}/?session=<redacted>"
  server_auth_args=(--token "${session_token}")
  open_web_url="http://127.0.0.1:${web_port}/?session=${session_token}"
else
  echo "Web client:        http://127.0.0.1:${web_port}/"
  server_auth_args=()
  open_web_url="http://127.0.0.1:${web_port}/"
fi
echo

"${proteus_bin}" --cwd "${workspace_cwd}" server http \
  --port "${app_port}" \
  "${server_auth_args[@]}" \
  --allow-origin "http://127.0.0.1:${web_port}" \
  --allow-origin "http://localhost:${web_port}" &
server_pid=$!

sleep 1
if ! kill -0 "${server_pid}" >/dev/null 2>&1; then
  wait "${server_pid}" 2>/dev/null || true
  echo "Proteus app server did not start. Port ${app_port} may already be in use." >&2
  exit 1
fi

cleanup() {
  kill "${server_pid}" >/dev/null 2>&1 || true
  wait "${server_pid}" 2>/dev/null || true
}
trap cleanup INT TERM EXIT

cd "${web_dir}"
(
  sleep 2
  if command -v xdg-open >/dev/null 2>&1; then
    xdg-open "${open_web_url}" >/dev/null 2>&1 || true
  elif command -v open >/dev/null 2>&1; then
    open "${open_web_url}" >/dev/null 2>&1 || true
  fi
) &
env -u NO_COLOR trunk serve --port "${web_port}"
WRAPPER
escaped_project_dir=$(printf '%s' "${project_dir}" | sed 's/[&|]/\\&/g')
sed -i "s|__PROTEUS_PROJECT_DIR__|${escaped_project_dir}|g" "${bin_tmp}"
chmod 755 "${bin_tmp}"
mv "${bin_tmp}" "${bin_path}"
trap - EXIT HUP INT TERM

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
case ":${PATH}:" in
  *:"${bin_dir}":*) ;;
  *) echo "Add this to your shell config if needed: export PATH=\"${bin_dir}:\$PATH\"" ;;
esac
