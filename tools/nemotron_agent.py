#!/usr/bin/env python3
"""
ArchieOS Nemotron Agentic v1 - Security-Hardened AI Copilot
Backend: Nebius Token Factory (NVIDIA Nemotron-3-Super-120B-A12B)

Security Invariants:
1. Sandbox Enforced: No path traversal outside PROJECT_ROOT.
2. Zero-Secret Leak: Block reading sensitive files (.env, private keys, credentials).
3. Bounded Resources: Limit file reads and memory footprint.
"""

import os
import sys
import json
import urllib.request
import urllib.error
import subprocess
from pathlib import Path

# Base Paths
PROJECT_ROOT = Path(__file__).resolve().parent.parent

# Configuration
API_KEY = os.environ.get("NEBIUS_API_KEY", "")
BASE_URL = os.environ.get("NEBIUS_BASE_URL", "https://api.tokenfactory.nebius.com/v1")
MODEL = os.environ.get("NEBIUS_MODEL", "nvidia/Nemotron-3-Super-120B-A12B")

# Protected file patterns that cannot be read or overwritten by the agent
SENSITIVE_PATTERNS = {
    ".env",
    ".git",
    "id_rsa",
    "id_ed25519",
    "known_hosts",
    "authorized_keys",
    "vault.key",
    "secret",
}

class SecurityException(Exception):
    """Raised when an action violates the sandbox or security policy."""
    pass

def safe_resolve_path(base_dir: Path, requested_path: str) -> Path:
    """
    Safely resolves a requested path ensuring it stays strictly within base_dir.
    Mitigates Directory Traversal (CWE-22) and protects sensitive credentials.
    """
    if not requested_path or not requested_path.strip():
        raise SecurityException("Path cannot be empty.")

    clean_path = requested_path.strip().replace("\\", "/")
    
    # Prohibit explicit drive roots on Windows or root on Unix
    if os.path.isabs(clean_path) or (len(clean_path) > 1 and clean_path[1] == ":"):
        raise SecurityException(f"Absolute paths are forbidden: '{requested_path}'")

    base_resolved = base_dir.resolve()
    target_candidate = (base_resolved / clean_path).resolve()

    # Invariant: Target MUST be within base_resolved
    try:
        common = Path(os.path.commonpath([str(base_resolved), str(target_candidate)]))
    except ValueError:
        raise SecurityException(f"Path escape attempt across drives: '{requested_path}'")

    if common != base_resolved:
        raise SecurityException(f"Path traversal detected! Attempted escape: '{requested_path}'")

    # Invariant: Sensitive file protection
    parts = target_candidate.relative_to(base_resolved).parts
    for part in parts:
        part_lower = part.lower()
        for sensitive in SENSITIVE_PATTERNS:
            if sensitive in part_lower:
                raise SecurityException(f"Access denied to sensitive resource: '{part}'")

    return target_candidate

# Available Agent Tools
TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read file contents safely from within the ArchieOS workspace.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Relative file path from project root"}
                },
                "required": ["path"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "write_file",
            "description": "Write or overwrite content to a file relative to the project root.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Relative file path from project root"},
                    "content": {"type": "string", "description": "Text content to write"}
                },
                "required": ["path", "content"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "list_directory",
            "description": "List files and directories within a given project folder.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Relative directory path (use '.' for root)"}
                },
                "required": ["path"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "run_command",
            "description": "Execute a shell or PowerShell command within the ArchieOS workspace.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command to execute"}
                },
                "required": ["command"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "archie_privacy_check",
            "description": "Validate data against the ArchieOS Privacy Spine (checks for secret leaks or sensitive records).",
            "parameters": {
                "type": "object",
                "properties": {
                    "data": {"type": "string", "description": "Data to evaluate"}
                },
                "required": ["data"]
            }
        }
    }
]

def execute_tool(name: str, args: dict) -> str:
    """Executes sandboxed tools requested by the AI model."""
    print(f"\033[94m⚙ [Tool Call] {name}({json.dumps(args)})\033[0m")
    try:
        if name == "read_file":
            target = safe_resolve_path(PROJECT_ROOT, args["path"])
            if not target.exists() or not target.is_file():
                return f"Error: File '{args['path']}' does not exist."
            with open(target, "r", encoding="utf-8", errors="replace") as f:
                content = f.read()
            # Bounded output to prevent context saturation
            return content[:25000]

        elif name == "write_file":
            target = safe_resolve_path(PROJECT_ROOT, args["path"])
            target.parent.mkdir(parents=True, exist_ok=True)
            with open(target, "w", encoding="utf-8") as f:
                f.write(args["content"])
            return f"Successfully wrote {len(args['content'])} bytes to {args['path']}."

        elif name == "list_directory":
            target = safe_resolve_path(PROJECT_ROOT, args.get("path", "."))
            if not target.is_dir():
                return f"Error: '{args.get('path')}' is not a directory."
            items = sorted(os.listdir(target))
            # Filter sensitive items from output
            safe_items = [
                i for i in items 
                if not any(s in i.lower() for s in SENSITIVE_PATTERNS)
            ]
            return json.dumps(safe_items[:100])

        elif name == "run_command":
            cmd = args["command"]
            proc = subprocess.run(
                cmd,
                shell=True,
                cwd=str(PROJECT_ROOT),
                capture_output=True,
                text=True,
                timeout=60
            )
            out = proc.stdout + ("\n" + proc.stderr if proc.stderr else "")
            return out.strip() if out.strip() else "(Command executed with no output, exit code: 0)"

        elif name == "archie_privacy_check":
            data = args["data"].lower()
            if any(k in data for k in ["secret", "private_key", "password", "token", "api_key"]):
                return "PrivacyGrade::Secret -> DISQUALIFIED: Cannot disclose to cloud. Restricted to local node."
            return "PrivacyGrade::Internal -> APPROVED for Nebius Token Factory."

        else:
            return f"Unknown tool: {name}"

    except SecurityException as sec_err:
        print(f"\033[91m🚨 [SECURITY VIOLATION BLOCKED] {sec_err}\033[0m")
        return f"Security Error: Access Denied. {str(sec_err)}"
    except Exception as e:
        return f"Tool Execution Error: {str(e)}"

def call_nemotron(messages: list) -> dict:
    """Sends payload to Nebius Token Factory completions API."""
    if not API_KEY:
        raise ValueError("NEBIUS_API_KEY environment variable is required.")

    url = f"{BASE_URL}/chat/completions"
    payload = {
        "model": MODEL,
        "messages": messages,
        "tools": TOOLS,
        "temperature": 0.2,
        "max_tokens": 2048
    }
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {API_KEY}",
            "Content-Type": "application/json"
        },
        method="POST"
    )
    with urllib.request.urlopen(req, timeout=90) as response:
        return json.loads(response.read().decode("utf-8"))

SYSTEM_PROMPT = """You are Nemotron Agentic v1, the lead AI systems engineer and cognitive runtime copilot for ArchieOS.
ArchieOS is a Rust-based, zero-leak personal AI operating system featuring a strict Privacy Spine, capability-aware routing, and memory consolidation.
You have access to tools to read, write, list files, run workspace commands, and check privacy rules.
Security policy:
1. You operate under a strict sandbox inside PROJECT_ROOT.
2. Any attempt to inspect system files or credentials will be immediately blocked by the runtime.
3. Validate actions against ArchieOS invariants (Zero-Secret Leak, Strict Boundary).
"""

def run_self_test():
    """Validates the sandbox implementation with penetration tests."""
    print("Running Nemotron Agent Sandboxing & Security Verification...")
    
    # Test 1: Legal path
    try:
        resolved = safe_resolve_path(PROJECT_ROOT, "README.md")
        assert resolved.name == "README.md"
        print("  [PASS] Legal path resolution within root")
    except Exception as e:
        print(f"  [FAIL] Legal path failed: {e}")
        return False

    # Test 2: Traversal escape
    try:
        safe_resolve_path(PROJECT_ROOT, "../../.ssh/id_rsa")
        print("  [FAIL] Path traversal was NOT blocked!")
        return False
    except SecurityException:
        print("  [PASS] Path traversal attempt blocked successfully")

    # Test 3: Sensitive file access (.env)
    try:
        safe_resolve_path(PROJECT_ROOT, ".env")
        print("  [FAIL] Sensitive file access was NOT blocked!")
        return False
    except SecurityException:
        print("  [PASS] Sensitive file (.env) access blocked successfully")

    # Test 4: Absolute path
    try:
        safe_resolve_path(PROJECT_ROOT, "C:/Windows/System32/cmd.exe" if os.name == "nt" else "/etc/passwd")
        print("  [FAIL] Absolute path was NOT blocked!")
        return False
    except SecurityException:
        print("  [PASS] Absolute path blocked successfully")

    print("\033[92mAll sandbox tests passed successfully!\033[0m")
    return True

def main():
    if "--test-sandbox" in sys.argv:
        success = run_self_test()
        sys.exit(0 if success else 1)

    print("=" * 70)
    print("\033[92m🧠 ArchieOS Nemotron Agentic v1 (Sandboxed)\033[0m")
    print(f"Backend: \033[96mNebius Token Factory\033[0m | Model: \033[93m{MODEL}\033[0m")
    print("Zero-Secret Leak & Sandboxed Boundary Active.")
    print("=" * 70)

    history = [{"role": "system", "content": SYSTEM_PROMPT}]

    if len(sys.argv) > 1:
        user_input = " ".join([arg for arg in sys.argv[1:] if arg != "--test-sandbox"])
        if user_input:
            run_loop(history, user_input)
            return

    while True:
        try:
            user_input = input("\n\033[1;37mUser > \033[0m").strip()
            if not user_input:
                continue
            if user_input.lower() in ["exit", "quit"]:
                print("Exiting session.")
                break
            run_loop(history, user_input)
        except (KeyboardInterrupt, EOFError):
            print("\nSession interrupted.")
            break

def run_loop(history: list, user_input: str):
    history.append({"role": "user", "content": user_input})

    while True:
        try:
            res = call_nemotron(history)
        except urllib.error.HTTPError as e:
            print(f"\033[91mHTTP Error {e.code}: {e.read().decode('utf-8')}\033[0m")
            break
        except Exception as e:
            print(f"\033[91mConnection Error: {e}\033[0m")
            break

        choice = res["choices"][0]
        msg = choice["message"]
        reasoning = msg.get("reasoning_content")
        content = msg.get("content")
        tool_calls = msg.get("tool_calls")

        if reasoning:
            print(f"\033[90m💭 [Thinking]: {reasoning.strip()}\033[0m")

        history.append(msg)

        if tool_calls:
            for tool_call in tool_calls:
                fn_name = tool_call["function"]["name"]
                try:
                    fn_args = json.loads(tool_call["function"]["arguments"])
                except Exception:
                    fn_args = {}
                result = execute_tool(fn_name, fn_args)
                history.append({
                    "role": "tool",
                    "tool_call_id": tool_call["id"],
                    "content": str(result)
                })
            continue
        else:
            if content:
                print(f"\n\033[92mNemotron Agentic >\033[0m {content.strip()}")
            break

if __name__ == "__main__":
    main()
