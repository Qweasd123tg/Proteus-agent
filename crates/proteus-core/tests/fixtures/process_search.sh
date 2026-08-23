#!/bin/sh
set -eu

mode=${1:?process search fixture mode is required}
module_id=${2:-fixture}
component_id=${3:-search-fixture}

# The host emits compact envelopes with the top-level id first. This fixture
# deliberately avoids a JSON parser: strict envelope shaping is covered by the
# protocol crate, while this script only supplies slot-level swap evidence.
rpc_id() {
    printf '%s\n' "$1" | sed -n 's/^[[:space:]]*{"id":[[:space:]]*\([^,}]*\),.*/\1/p'
}

IFS= read -r initialize_request
initialize_id=$(rpc_id "$initialize_request")
if [ "$mode" = "slow_initialize" ]; then
    sleep 0.4
fi
if [ "$mode" = "mismatch" ]; then
    slot=memory
else
    slot=search
fi
printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$initialize_id,\"result\":{\"protocol_version\":\"v3\",\"component_id\":\"$component_id\",\"exports\":[{\"slot\":\"$slot\",\"module_id\":\"$module_id\",\"contract_version\":\"v1\",\"composition\":\"select_one\",\"module_features\":[]}]}}"

while IFS= read -r search_request; do
    request_id=$(rpc_id "$search_request")
    case "$mode" in
        exit)
            exit 9
            ;;
        error)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"error\":{\"code\":-32000,\"message\":\"fixture search failure\"}}"
            ;;
        invalid)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":[]}"
            ;;
        *)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"chunks\":[{\"source\":\"process:$module_id\",\"path\":\"sample.txt\",\"content\":\"hit from $module_id\",\"score\":1.0,\"metadata\":{\"fixture\":true}}]}}"
            ;;
    esac
done
