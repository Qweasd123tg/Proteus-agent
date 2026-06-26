#!/bin/sh

# Minimal stdio MCP server for local Proteus smoke tests.
# Run through `sh examples/mcp/echo_server.sh`; no executable bit required.

calls=0

json_id() {
  printf '%s\n' "$1" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p'
}

json_string_field() {
  field=$1
  printf '%s\n' "$2" | sed -n "s/.*\"$field\":\"\\([^\"]*\\)\".*/\\1/p"
}

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

while IFS= read -r line; do
  id=$(json_id "$line")
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"proteus-echo","version":"0.1.0"}}}\n' "$id"
      ;;
    *'"method":"notifications/initialized"'*)
      ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Echo a message and show persistent MCP call count.","inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"additionalProperties":false}}]}}\n' "$id"
      ;;
    *'"method":"tools/call"'*)
      calls=$((calls + 1))
      message=$(json_string_field "message" "$line")
      if [ -z "$message" ]; then
        message="empty"
      fi
      message=$(json_escape "$message")
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"echo[%s]: %s"}],"structuredContent":{"calls":%s},"isError":false}}\n' "$id" "$calls" "$message" "$calls"
      ;;
    *)
      if [ -n "$id" ]; then
        printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"method not found"}}\n' "$id"
      fi
      ;;
  esac
done
