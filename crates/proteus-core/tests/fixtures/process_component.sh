#!/bin/sh
set -eu

component_id=${1:-multi-fixture}
search_id=${2:-fixture-search}
compactor_id=${3:-fixture-compactor}
marker=${4:?component startup marker is required}

printf '%s\n' "$$" >> "$marker"

rpc_id() {
    printf '%s\n' "$1" | sed -n 's/^[[:space:]]*{"id":[[:space:]]*\([^,}]*\),.*/\1/p'
}

IFS= read -r initialize_request
initialize_id=$(rpc_id "$initialize_request")
printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$initialize_id,\"result\":{\"protocol_version\":\"v3\",\"component_id\":\"$component_id\",\"exports\":[{\"slot\":\"search\",\"module_id\":\"$search_id\",\"contract_version\":\"v1\",\"composition\":\"select_one\",\"module_features\":[]},{\"slot\":\"compactor\",\"module_id\":\"$compactor_id\",\"contract_version\":\"v1\",\"composition\":\"select_one\",\"module_features\":[]}]}}"

while IFS= read -r request; do
    request_id=$(rpc_id "$request")
    case "$request" in
        *'"method":"search"'*)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"chunks\":[{\"source\":\"shared-component\",\"path\":\"sample.txt\",\"content\":\"shared search\",\"score\":1.0,\"metadata\":{\"fixture\":true}}]}}"
            ;;
        *'"method":"compact"'*)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"output\":{\"messages\":[],\"changed\":false,\"summary\":null,\"token_estimate\":null,\"metadata\":{\"fixture\":true}}}}"
            ;;
        *)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"error\":{\"code\":-32601,\"message\":\"unknown fixture method\"}}"
            ;;
    esac
done
