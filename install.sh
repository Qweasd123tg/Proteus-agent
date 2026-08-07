#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
bin_dir="${HOME}/.local/bin"
bin_path="${bin_dir}/proteus"
proteus_home="${PROTEUS_HOME:-${HOME}/.proteus}"
releases_dir="${proteus_home}/releases"
current_release="${proteus_home}/current"
config_home="${PROTEUS_CONFIG_HOME:-${HOME}/.config/Proteus-agent}"
configs_dir="${config_home}/configs"

cargo build --release --manifest-path "${project_dir}/Cargo.toml" \
  -p proteus-core \
  -p proteus-reference-worker

mkdir -p "${bin_dir}"
bin_tmp="${bin_path}.tmp.$$"
release_id=$(date -u +%Y%m%dT%H%M%SZ)-$$
release_tmp="${releases_dir}/.${release_id}.tmp"
release_dir="${releases_dir}/${release_id}"
current_tmp="${proteus_home}/.current.$$"
release_published=0
rm -f "${bin_tmp}" "${current_tmp}"
rm -rf "${release_tmp}"

cleanup_install() {
  status=$?
  trap - EXIT HUP INT TERM
  set +e
  rm -f "${bin_tmp}" "${current_tmp}"
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
#!/usr/bin/env bash
set -euo pipefail

project_dir="__PROTEUS_PROJECT_DIR__"
proteus_home="${PROTEUS_HOME:-${HOME}/.proteus}"
current_release="${proteus_home}/current"
proteus_bin="${current_release}/proteus"
export PATH="${current_release}:${PATH}"
web_dir="${project_dir}/clients/web"
inspector_dir="${project_dir}/clients/inspector"
app_port="${PROTEUS_APP_PORT:-8787}"
web_port="${PROTEUS_WEB_PORT:-1420}"
inspector_port="${PROTEUS_INSPECTOR_PORT:-1421}"
inspector_enabled="${PROTEUS_INSPECTOR:-1}"
session_token="${PROTEUS_SESSION_TOKEN:-}"
session_query_key="token"

generate_session_token() {
  if command -v uuidgen >/dev/null 2>&1; then
    uuidgen | tr -d '-' | tr '[:upper:]' '[:lower:]'
    return
  fi
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 16
    return
  fi
  od -An -N16 -tx1 /dev/urandom | tr -d ' \n'
}

# Локальный token-режим включён по умолчанию (см. docs/dogfood-gate.md,
# Blocking Bugs). Отключение — только явное: PROTEUS_NO_SESSION_TOKEN=1.
if [ -z "${session_token}" ] && [ "${PROTEUS_NO_SESSION_TOKEN:-0}" != "1" ]; then
  session_token=$(generate_session_token)
fi

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

close_previous_web_server() {
  pids=$(listener_pids_for_port "${web_port}")
  if [ -z "${pids}" ]; then
    return
  fi

  for pid in ${pids}; do
    cmd=$(ps -p "${pid}" -o args= 2>/dev/null || true)
    case "${cmd}" in
      *trunk*" serve"*)
        echo "Closing previous Proteus web server on port ${web_port} (pid ${pid})..."
        kill "${pid}" >/dev/null 2>&1 || true
        ;;
      *)
        echo "Port ${web_port} is already in use by pid ${pid}: ${cmd}" >&2
        echo "Stop that process or set PROTEUS_WEB_PORT to another port." >&2
        exit 1
        ;;
    esac
  done

  for _ in {1..30}; do
    if [ -z "$(listener_pids_for_port "${web_port}")" ]; then
      return
    fi
    sleep 0.1
  done

  echo "Previous Proteus web server did not release port ${web_port}." >&2
  exit 1
}

close_previous_inspector_server() {
  pids=$(listener_pids_for_port "${inspector_port}")
  if [ -z "${pids}" ]; then
    return
  fi

  for pid in ${pids}; do
    cmd=$(ps -p "${pid}" -o args= 2>/dev/null || true)
    case "${cmd}" in
      *trunk*" serve"*)
        echo "Closing previous Proteus inspector server on port ${inspector_port} (pid ${pid})..."
        kill "${pid}" >/dev/null 2>&1 || true
        ;;
      *)
        echo "Port ${inspector_port} is already in use by pid ${pid}: ${cmd}" >&2
        echo "Stop that process or set PROTEUS_INSPECTOR_PORT to another port." >&2
        exit 1
        ;;
    esac
  done

  for _ in {1..30}; do
    if [ -z "$(listener_pids_for_port "${inspector_port}")" ]; then
      return
    fi
    sleep 0.1
  done

  echo "Previous Proteus inspector server did not release port ${inspector_port}." >&2
  exit 1
}

if [ ! -x "${proteus_bin}" ]; then
  echo "Proteus binary is missing; building release binary..." >&2
  "${project_dir}/install.sh"
elif find "${project_dir}/crates" "${project_dir}/modules/reference" "${project_dir}/Cargo.toml" "${project_dir}/Cargo.lock" -newer "${proteus_bin}" -print -quit | grep -q .; then
  echo "Proteus binary is stale; rebuilding release binary..." >&2
  "${project_dir}/install.sh"
fi

server_config_args=()
original_args=("$@")
if [ "$#" -gt 0 ]; then
  case "$#" in
    1)
      case "$1" in
        --config=*) server_config_args=("$1") ;;
        *) exec "${proteus_bin}" "${original_args[@]}" ;;
      esac
      ;;
    2)
      case "$1" in
        --config) server_config_args=("$1" "$2") ;;
        *) exec "${proteus_bin}" "${original_args[@]}" ;;
      esac
      ;;
    *)
      exec "${proteus_bin}" "${original_args[@]}"
      ;;
  esac
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
close_previous_web_server
if [ "${inspector_enabled}" != "0" ]; then
  close_previous_inspector_server
fi

workspace_cwd=$(pwd)
echo "Proteus workspace: ${workspace_cwd}"
echo "App server:        http://127.0.0.1:${app_port}"
# Клиенты умеют читать app-server/inspector/chat origin из query и
# sessionStorage; без параметров web ходил бы на default 8787 даже при
# PROTEUS_APP_PORT, а inspector строил бы ссылку «Чат» на default 1420.
encoded_app_origin="http%3A%2F%2F127.0.0.1%3A${app_port}"
encoded_inspector_origin="http%3A%2F%2F127.0.0.1%3A${inspector_port}"
encoded_web_origin="http%3A%2F%2F127.0.0.1%3A${web_port}"
common_query="server=${encoded_app_origin}&inspector=${encoded_inspector_origin}"
inspector_query="server=${encoded_app_origin}&chat=${encoded_web_origin}"
if [ -n "${session_token}" ]; then
  echo "Web client:        http://127.0.0.1:${web_port}/?${session_query_key}=<redacted>&${common_query}"
  if [ "${inspector_enabled}" != "0" ]; then
    echo "Inspector:         http://127.0.0.1:${inspector_port}/?${session_query_key}=<redacted>&${inspector_query}"
  fi
  server_auth_args=(--token "${session_token}")
  open_web_url="http://127.0.0.1:${web_port}/?${session_query_key}=${session_token}&${common_query}"
else
  echo "Web client:        http://127.0.0.1:${web_port}/?${common_query}"
  if [ "${inspector_enabled}" != "0" ]; then
    echo "Inspector:         http://127.0.0.1:${inspector_port}/?${inspector_query}"
  fi
  server_auth_args=()
  open_web_url="http://127.0.0.1:${web_port}/?${common_query}"
fi
echo

"${proteus_bin}" \
  "${server_config_args[@]}" \
  --cwd "${workspace_cwd}" \
  server http \
  --port "${app_port}" \
  "${server_auth_args[@]}" \
  --allow-origin "http://127.0.0.1:${web_port}" \
  --allow-origin "http://localhost:${web_port}" \
  --allow-origin "http://127.0.0.1:${inspector_port}" \
  --allow-origin "http://localhost:${inspector_port}" &
server_pid=$!

sleep 1
if ! kill -0 "${server_pid}" >/dev/null 2>&1; then
  server_status=0
  wait "${server_pid}" 2>/dev/null || server_status=$?
  if [ "${server_status}" -eq 0 ]; then
    server_status=1
  fi
  echo "Proteus app server exited during startup. See the error above." >&2
  echo "For startup diagnostics, run: ${proteus_bin} ${server_config_args[*]} doctor" >&2
  exit "${server_status}"
fi

inspector_pid=""
if [ "${inspector_enabled}" != "0" ]; then
  (
    cd "${inspector_dir}"
    env -u NO_COLOR trunk serve --port "${inspector_port}"
  ) &
  inspector_pid=$!

  sleep 1
  if ! kill -0 "${inspector_pid}" >/dev/null 2>&1; then
    wait "${inspector_pid}" 2>/dev/null || true
    kill "${server_pid}" >/dev/null 2>&1 || true
    wait "${server_pid}" 2>/dev/null || true
    echo "Proteus inspector server did not start. Port ${inspector_port} may already be in use." >&2
    exit 1
  fi
fi

cleanup() {
  kill "${server_pid}" >/dev/null 2>&1 || true
  if [ -n "${inspector_pid}" ]; then
    kill "${inspector_pid}" >/dev/null 2>&1 || true
    wait "${inspector_pid}" 2>/dev/null || true
  fi
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

# Stage the host and reference worker before the `current` symlink makes the
# release visible.
mkdir -p "${release_tmp}"
cp "${project_dir}/target/release/proteus" "${release_tmp}/proteus"
cp "${project_dir}/target/release/proteus-reference-worker" "${release_tmp}/proteus-reference-worker"
chmod 755 "${release_tmp}/proteus"
chmod 755 "${release_tmp}/proteus-reference-worker"

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
install_config "opencode.config.toml" "configs/opencode.config.toml"
install_config "proteus.provider.example.toml" "configs/proteus.provider.example.toml"

# Prompt-файлы обновляются при каждой установке: это код профиля, а не
# пользовательские правки (в отличие от configs, которые не перезаписываются).
mkdir -p "${configs_dir}/prompts"
install_prompt() {
  source_path="${project_dir}/configs/prompts/$1"
  dest_path="${configs_dir}/prompts/$1"
  # configs_dir может быть симлинком на репозиторный configs/ — тогда source
  # и dest являются одним файлом и копирование не нужно.
  if [ "${source_path}" -ef "${dest_path}" ]; then
    return
  fi
  cp "${source_path}" "${dest_path}"
}
install_prompt "codex-default.md"
install_prompt "opencode-default.md"

echo "Installed: ${bin_path}"
echo "Release:   ${release_dir}"
echo "Worker:    ${current_release}/proteus-reference-worker"
echo "Configs:   ${configs_dir}"
echo "Next:      ${bin_path} init coding && ${bin_path} doctor"
case ":${PATH}:" in
  *:"${bin_dir}":*) ;;
  *) echo "Add this to your shell config if needed: export PATH=\"${bin_dir}:\$PATH\"" ;;
esac
