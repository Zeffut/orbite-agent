"""A small MCP server over stdio, for tests: two tools, and a file where it
writes every call it carried out. Usage: fake_mcp.py <calls file> [modern]
With "modern" it speaks the protocol of 2026-07-28 only: no greeting, and every
request must say its version."""
import json
import os
import sys

TOOLS = [
    {
        "name": "read-note",
        "title": "Read a note",
        "description": "Reads the note of that name.",
        "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "delete_note",
        "description": "Deletes the note of that name. It cannot be recovered.",
        "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        "annotations": {"destructiveHint": True},
    },
]


MODERN = len(sys.argv) > 2 and sys.argv[2] == "modern"


def answer(message):
    method, params = message.get("method"), message.get("params") or {}
    if MODERN:
        if method == "initialize":
            return None
        if (params.get("_meta") or {}).get("io.modelcontextprotocol/protocolVersion") != "2026-07-28":
            return None
        if method == "server/discover":
            return {"resultType": "complete", "supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}}}
    elif method == "server/discover":
        return None
    if method == "initialize":
        return {"protocolVersion": params.get("protocolVersion"), "capabilities": {"tools": {}},
                "serverInfo": {"name": "notes", "version": "1"}}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        with open(sys.argv[1], "a") as calls:
            calls.write(json.dumps(params) + "\n")
        name = params["arguments"].get("name")
        if name == "crash":
            sys.exit(1)
        if name == "flood":
            # One endless line, as a server gone wrong would write.
            while True:
                sys.stdout.write("x" * 1_000_000)
                sys.stdout.flush()
        if name == "spawn":
            # A program of its own, which outlives it unless stopped with it.
            import subprocess
            child = subprocess.Popen(["sleep", "300"])
            return {"content": [{"type": "text", "text": str(child.pid)}]}
        if name == "missing":
            return {"content": [{"type": "text", "text": "There is no such note."}], "isError": True}
        text = f"{params['name']} {name}: token {os.environ.get('NOTES_TOKEN', 'none')}"
        return {"content": [{"type": "text", "text": text}, {"type": "text", "text": "second part"}]}
    return None


for line in sys.stdin:
    message = json.loads(line)
    if "id" not in message:
        continue
    result = answer(message)
    reply = {"jsonrpc": "2.0", "id": message["id"]}
    if result is None:
        reply["error"] = {"code": -32601, "message": "Method not found"}
    else:
        reply["result"] = result
    # A notification first, as real servers send: a client must skip it.
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "working"}}) + "\n")
    sys.stdout.write(json.dumps(reply) + "\n")
    sys.stdout.flush()
