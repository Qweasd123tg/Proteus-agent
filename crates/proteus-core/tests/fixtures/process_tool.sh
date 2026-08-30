#!/bin/sh
set -eu

module_id=${1:-execution-boundary-tools}
component_id=${2:-execution-boundary-tool-component}

rpc_id() {
    printf '%s\n' "$1" | sed -n 's/^[[:space:]]*{"id":[[:space:]]*\([^,}]*\),.*/\1/p'
}

IFS= read -r initialize_request
initialize_id=$(rpc_id "$initialize_request")
printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$initialize_id,\"result\":{\"protocol_version\":\"v3\",\"component_id\":\"$component_id\",\"exports\":[{\"slot\":\"tool\",\"module_id\":\"$module_id\",\"contract_version\":\"v2\",\"composition\":\"ordered_many\",\"module_features\":[]}]}}"

while IFS= read -r request; do
    request_id=$(rpc_id "$request")
    case "$request" in
        *'"method":"list"'*)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"result\":[{\"name\":\"detached_probe\",\"description\":\"Detached process tool architecture probe\",\"input_schema\":{\"type\":\"object\",\"properties\":{},\"additionalProperties\":false},\"surface\":{\"kind\":\"function\",\"strict\":false,\"output_schema\":null},\"safety\":\"ReadOnly\",\"timeout_ms\":3000,\"metadata\":{\"fixture\":true}}]}}"
            ;;
        *'"method":"invoke"'*'"agent":null'*)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"result\":{\"call_id\":\"detached-call\",\"ok\":true,\"output\":\"detached process tool result\",\"content\":[],\"error\":null,\"metadata\":{\"saw_detached_attribution\":true}}}}"
            ;;
        *'"method":"invoke"'*)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"error\":{\"code\":-32000,\"message\":\"tool invocation was not detached\"}}"
            ;;
        *)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"error\":{\"code\":-32601,\"message\":\"unknown fixture method\"}}"
            ;;
    esac
done
