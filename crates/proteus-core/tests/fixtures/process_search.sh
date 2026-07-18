#!/bin/sh
set -eu

mode=${1:?process search fixture mode is required}

# ProcessSession starts request ids at 1. This fixture deliberately avoids a
# JSON parser: protocol shaping belongs to Proteus tests, not to the host
# language available on the machine running them.
IFS= read -r _initialize_request
if [ "$mode" = "slow_initialize" ]; then
    sleep 0.4
fi
if [ "$mode" = "mismatch" ]; then
    slot=memory
else
    slot=search
fi
printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocol_version\":\"v0\",\"slot\":\"$slot\",\"module_id\":\"fixture\",\"contract_version\":\"v0\"}}"

request_id=2
while IFS= read -r _search_request; do
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
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"chunks\":[{\"source\":\"process:fixture\",\"path\":\"sample.txt\",\"content\":\"hit needle\",\"score\":1.0,\"metadata\":{\"fixture\":true}}]}}"
            ;;
    esac
    request_id=$((request_id + 1))
done
