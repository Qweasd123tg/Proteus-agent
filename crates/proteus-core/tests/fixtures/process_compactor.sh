#!/bin/sh
set -eu

mode=${1:?process compactor fixture mode is required}
marker=${2:-}

IFS= read -r _initialize_request
if [ "$mode" = "slow_initialize" ]; then
    sleep 0.4
fi
if [ "$mode" = "mismatch" ]; then
    slot=search
else
    slot=compactor
fi
printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocol_version\":\"v0\",\"slot\":\"$slot\",\"module_id\":\"fixture\",\"contract_version\":\"v0\"}}"

request_id=2
while IFS= read -r _compact_request; do
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
            request_id=$((request_id + 1))
            continue
            ;;
        invalid)
            printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"messages\":[],\"changed\":false,\"summary\":null,\"token_estimate\":null,\"metadata\":null}}"
            request_id=$((request_id + 1))
            continue
            ;;
    esac

    printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$request_id,\"result\":{\"output\":{\"messages\":[],\"changed\":false,\"summary\":null,\"token_estimate\":null,\"metadata\":{\"fixture\":true}}}}"
    request_id=$((request_id + 1))
done
