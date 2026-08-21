#!/bin/sh
set -eu

mode=${1:?process compactor fixture mode is required}
module_id=${2:-fixture}
component_id=${3:-compactor-fixture}
marker=${4:-}

rpc_id() {
    printf '%s\n' "$1" | sed -n 's/^[[:space:]]*{"id":[[:space:]]*\([^,}]*\),.*/\1/p'
}

IFS= read -r initialize_request
initialize_id=$(rpc_id "$initialize_request")
if [ "$mode" = "slow_initialize" ]; then
    sleep 0.4
fi
if [ "$mode" = "mismatch" ]; then
    slot=search
else
    slot=compactor
fi
printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$initialize_id,\"result\":{\"protocol_version\":\"v2\",\"component_id\":\"$component_id\",\"exports\":[{\"slot\":\"$slot\",\"module_id\":\"$module_id\",\"contract_version\":\"v1\",\"composition\":\"select_one\",\"module_features\":[]}]}}"

while IFS= read -r compact_request; do
    request_id=$(rpc_id "$compact_request")
    case "$mode" in
        exit)
            exit 9
            ;;
        exit_once)
            if [ ! -e "$marker" ]; then
                : > "$marker"
                exit 9
            fi
            ;;
        error)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"error\":{\"code\":-32000,\"message\":\"fixture compaction failure\"}}"
            continue
            ;;
        invalid)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"messages\":[],\"changed\":false,\"summary\":null,\"token_estimate\":null,\"metadata\":null}}"
            continue
            ;;
    esac

    printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"output\":{\"messages\":[],\"changed\":false,\"summary\":null,\"token_estimate\":null,\"metadata\":{\"fixture\":true}}}}"
done
