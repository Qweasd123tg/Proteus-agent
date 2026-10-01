"""Reusable PreToolUse decision logic for a Codex/Claude-style command hook."""
import json
import sys

event = json.load(sys.stdin)
if event["tool_name"] == "apply_patch":
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": "Изменение файлов запрещено этим hook.",
    }}))
