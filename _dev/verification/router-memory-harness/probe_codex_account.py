"""Read official account metadata only. Never read credential/config/token files."""
import asyncio
import json
import shutil
import sys
import tempfile


async def main():
    command = shutil.which("codex")
    if not command:
        print(json.dumps({"installed": False}))
        return
    args = []
    buffered = []
    for key, value in {
        "mcp_servers": {}, "plugins": {}, "features.apps": False,
        "features.plugins": False, "features.hooks": False,
        "features.remote_plugin": False, "features.shell_tool": False,
        "features.multi_agent": False, "features.memories": False,
        "features.goals": False, "features.shell_snapshot": False,
        "web_search": "disabled", "project_doc_max_bytes": 0,
    }.items():
        args += ["-c", key + "=" + json.dumps(value)]
    child = await asyncio.create_subprocess_exec(
        command, "app-server", "--stdio", *args,
        stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL, limit=262144,
    )
    total = 0

    async def send(data):
        child.stdin.write((json.dumps(data) + "\n").encode())
        await child.stdin.drain()

    async def rpc(identifier, method, params):
        nonlocal total
        await send({"id": identifier, "method": method, "params": params})
        while True:
            raw = await asyncio.wait_for(child.stdout.readline(), 12)
            total += len(raw)
            if not raw or total > 2097152:
                raise ValueError("invalid or oversized official protocol")
            message = json.loads(raw)
            if message.get("id") == identifier:
                if "error" in message:
                    raise ValueError("official protocol rejected request")
                return message["result"]
            if "id" in message and "method" in message:
                raise ValueError("unexpected server request")
            buffered.append(message)

    try:
        await rpc(1, "initialize", {"clientInfo": {"name": "archie_account_probe", "version": "0.1.0"}})
        await send({"method": "initialized", "params": {}})
        result = await rpc(2, "account/read", {"refreshToken": False})
        account = result.get("account") or {}
        routing = result.get("workspaceRouting") or {}
        report = {"installed": True, "auth_type": account.get("type"),
            "plan": account.get("planType"),
            "stable_account_identity_available": bool(routing.get("chatgptAccountId")),
            "inference_tested": False}
        if "--infer" in sys.argv:
            catalog = await rpc(3, "model/list", {"limit": 100})
            models = catalog.get("data") or []
            chosen = next((m for m in models if m.get("isDefault")), models[0] if models else {})
            model = chosen.get("model") or chosen.get("id")
            if not model:
                raise ValueError("official catalog is empty")
            with tempfile.TemporaryDirectory(prefix="archie-codex-public-proof-", ignore_cleanup_errors=True) as isolated:
                started = await rpc(4, "thread/start", {"model": model, "cwd": isolated,
                    "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": True,
                    "config": {"features.shell_tool": False, "features.apps": False,
                        "features.plugins": False, "features.hooks": False,
                        "features.multi_agent": False, "web_search": "disabled",
                        "tools.view_image": False, "mcp_servers": {}, "plugins": {}},
                    "developerInstructions": "This is a public synthetic text-only integration proof. Do not invoke tools."})
                thread = started["thread"]["id"]
                buffered.clear()
                active = await rpc(5, "turn/start", {"threadId": thread,
                    "input": [{"type": "text", "text": "Reply with exactly ARCHIE_ROUTE_OK."}],
                    "model": model, "approvalPolicy": "never", "sandboxPolicy": {"type": "readOnly"}})
                turn = active["turn"]["id"]
                texts = []
                while True:
                    if buffered:
                        event = buffered.pop(0)
                    else:
                        raw = await asyncio.wait_for(child.stdout.readline(), 60)
                        total += len(raw)
                        if not raw or total > 2097152:
                            raise ValueError("invalid or oversized turn events")
                        event = json.loads(raw)
                    if "method" in event and "id" in event:
                        raise ValueError("tool or auth request refused")
                    method = event.get("method")
                    params = event.get("params") or {}
                    if method == "item/completed" and params.get("threadId") == thread and params.get("turnId") == turn:
                        item = params.get("item") or {}
                        if item.get("type") == "agentMessage":
                            texts.append(item.get("text", ""))
                        elif item.get("type") not in ("userMessage", "reasoning"):
                            raise ValueError("unexpected tool item")
                    if method == "turn/completed":
                        if params.get("threadId") != thread or params.get("turn", {}).get("id") != turn:
                            raise ValueError("uncorrelated turn")
                        report.update({"inference_tested": True, "model": model,
                            "turn_status": params["turn"]["status"],
                            "synthetic_exact_match": "\n".join(texts).strip() == "ARCHIE_ROUTE_OK"})
                        break
        print(json.dumps(report))
    finally:
        if child.returncode is None:
            child.kill()
        await child.wait()


if __name__ == "__main__":
    try:
        asyncio.run(asyncio.wait_for(main(), 90 if "--infer" in sys.argv else 20))
    except Exception as error:
        print(json.dumps({"probe_failed": type(error).__name__}))
        sys.exit(1)
