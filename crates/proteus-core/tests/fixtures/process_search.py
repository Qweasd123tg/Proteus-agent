#!/usr/bin/env python3
import json
import sys


mode = sys.argv[1]
initialized = False

for line in sys.stdin:
    request = json.loads(line)
    request_id = request["id"]
    method = request["method"]
    if method == "initialize":
        slot = "memory" if mode == "mismatch" else "search"
        message = {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocol_version": "v0",
                "slot": slot,
                "module_id": "fixture",
                "contract_version": "v0",
            },
        }
        initialized = True
    elif method == "search" and initialized:
        if mode == "exit":
            raise SystemExit(9)
        if mode == "error":
            message = {
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32000, "message": "fixture search failure"},
            }
        elif mode == "invalid":
            message = {"jsonrpc": "2.0", "id": request_id, "result": []}
        else:
            query = request["params"]
            message = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "chunks": [
                        {
                            "source": "process:fixture",
                            "path": "sample.txt",
                            "content": f"hit {query['text']}",
                            "score": 1.0,
                            "metadata": {"fixture": True},
                        }
                    ]
                },
            }
    else:
        message = {
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32601, "message": "unexpected method"},
        }
    print(json.dumps(message, separators=(",", ":")), flush=True)
