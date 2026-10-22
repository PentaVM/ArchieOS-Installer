"""Bounded account-only host diagnostic; never reads credentials/config files."""
import asyncio
import json
import os
import sys
import tempfile

EXE = r"C:\Users\Penta\AppData\Local\OpenAI\Codex\bin\896a0166b774037a\codex.exe"
if os.name != "nt":
    EXE = "/mnt/c/Users/Penta/AppData/Local/OpenAI/Codex/bin/896a0166b774037a/codex.exe"

CONFIG = {
    "model_provider": "openai", "web_search": "disabled", "mcp_servers": {}, "plugins": {},
    "features.shell_tool": False, "features.apps": False, "features.plugins": False,
    "features.remote_plugin": False, "features.multi_agent": False, "features.hooks": False,
    "features.memories": False, "features.goals": False, "features.shell_snapshot": False,
    "features.skill_mcp_dependency_install": False, "tools.view_image": False,
    "project_doc_max_bytes": 0, "sandbox_mode": "read-only", "approval_policy": "never",
}

async def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "rust"
    config = dict(CONFIG)
    if mode == "baseline":
        for key in ("model_provider", "features.skill_mcp_dependency_install", "tools.view_image", "sandbox_mode", "approval_policy"):
            config.pop(key)
    elif mode == "minimal":
        config = {}
    args = [EXE, "app-server", "--stdio"]
    for key in sorted(config):
        args += ["-c", key + "=" + json.dumps(config[key], separators=(",", ":"))]
    result = {"host": os.name, "config": mode, "inference_tested": False}
    child = None
    err = b""
    out_bytes = 0
    err_task = None
    # Win child can use the shared drvfs directory as cwd through WSL interop.
    proof_dir = os.path.dirname(os.path.abspath(__file__))
    with tempfile.TemporaryDirectory(prefix="codex-host-public-", dir=proof_dir) as isolated:
        try:
            child = await asyncio.create_subprocess_exec(*args, cwd=isolated,
                stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE, limit=262144)
            async def bounded_stderr():
                return await child.stderr.read(65537)
            err_task = asyncio.create_task(bounded_stderr())
            async def rpc(i, method, params):
                nonlocal out_bytes
                child.stdin.write((json.dumps({"id": i, "method": method, "params": params}) + "\n").encode())
                await child.stdin.drain()
                while True:
                    raw = await asyncio.wait_for(child.stdout.readline(), 12)
                    out_bytes += len(raw)
                    if not raw:
                        raise EOFError()
                    if out_bytes > 2097152:
                        raise OverflowError()
                    message = json.loads(raw)
                    if message.get("id") == i:
                        if "error" in message:
                            raise RuntimeError("protocol_rejected")
                        return message["result"]
            await rpc(1, "initialize", {"clientInfo": {"name": "archie_host_probe", "version": "0.1.0"}})
            result["initialized"] = True
            child.stdin.write(b'{"method":"initialized","params":{}}\n')
            await child.stdin.drain()
            reply = await rpc(2, "account/read", {"refreshToken": False})
            account = reply.get("account") or {}
            routing = reply.get("workspaceRouting") or {}
            result.update({"account_read": True, "chatgpt_auth": account.get("type") == "chatgpt",
                "stable_account_identity_available": bool(routing.get("chatgptAccountId"))})
        except Exception as e:
            result["error_type"] = type(e).__name__
        finally:
            if child:
                result["exit_before_cleanup"] = child.returncode
                if child.returncode is None:
                    child.kill()
                await child.wait()
                result["exit_code"] = child.returncode
            if err_task:
                err = await asyncio.wait_for(err_task, 3)
            result["stdout_bytes"] = out_bytes
            result["stderr_bytes_capped"] = len(err)
            lower = err.lower()
            # Fixed flags only: never print arbitrary stderr, paths, account IDs, or environment.
            result["stderr_categories"] = [name for name, needle in {
                "config_error": b"error loading config", "invalid_config": b"invalid config",
                "parse_error": b"parse", "unexpected_argument": b"unexpected argument",
                "unknown_argument": b"unrecognized", "invalid_value": b"invalid value",
                "toml_error": b"toml", "file_not_found": b"no such file",
                "access_denied": b"access is denied", "cwd_error": b"current directory",
                "interop_error": b"interop", "windows_error": b"os error",
                "wsl_accept_vsock_timeout": b"utilacceptvsock:250: accept4 failed 110",
                "config_mention": b"config", "failed_mention": b"failed",
                "invalid_mention": b"invalid", "home_mention": b"home",
                "path_mention": b"path", "system_mention": b"system",
                "permission_mention": b"permission", "file_mention": b"file",
                "decode_mention": b"decode", "directory_mention": b"directory",
                "windows_mention": b"windows", "root_mention": b"root",
                "not_found_mention": b"not found", "specified_mention": b"specified",
                "nul_mention": b"nul", "pipe_mention": b"pipe", "socket_mention": b"socket",
                "certificate_mention": b"certificate", "panic_mention": b"panic",
                "pkey_mention": b"key", "stdio_mention": b"stdio",
                **{word + "_mention": word.encode() for word in (
                    "stdin", "stdout", "stderr", "stream", "handle", "initialize", "initialization",
                    "startup", "sandbox", "auth", "rustls", "telemetry", "ipc", "session", "utf",
                    "encoding", "enable", "console", "terminal", "start", "create", "open", "read",
                    "write", "log", "connection", "network", "unexpected", "input", "output",
                    "could", "not", "find", "service", "support", "operation", "argument", "parameter",
                    "win32", "exec", "spawn", "openai", "app-server", "tokio", "runtime", "bootstrap",
                    "closed", "resource", "wait", "process", "busy", "os", "error", "bad", "unable",
                    "comando", "correcto", "acceso", "operaci", "identificador", "lectura", "escritura",
                    "controlador", "funci", "incorrecto", "memoria", "iniciar", "entrada", "salida",
                    "obtener", "proceso", "manejador", "inv", "par", "val", "tuber", "especifica",
                    "archivo", "no", "directorio", "sistema", "recurso", "dispositivo", "permiso",
                    "retrieve", "get", "set", "missing", "broken", "socket2", "mio", "io", "poll",
                    "connect", "decode", "deserialize", "serialize", "format", "register", "event",
                )},
            }.items() if needle in lower]
            result["codex_home_env_present"] = "CODEX_HOME" in os.environ
            result["stderr_nul_bytes"] = err.count(b"\0")
            result["stderr_non_ascii_bytes"] = sum(b > 127 for b in err)
    print(json.dumps(result, sort_keys=True))

if __name__ == "__main__":
    try:
        asyncio.run(asyncio.wait_for(main(), 20))
    except Exception as e:
        print(json.dumps({"diagnostic_error": type(e).__name__}))
        sys.exit(1)
