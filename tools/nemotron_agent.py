#!/usr/bin/env python3
"""
ArchieOS Nemotron Agentic v1
Autonomous Engineering & Reasoning Agent powered by NVIDIA Nemotron-3 Super on Nebius Token Factory.
"""

import os
import sys
import json
import subprocess
import urllib.request
import urllib.error
from pathlib import Path

# Fix Windows console encoding
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

# Load .env file manually without external dependencies
def load_env():
    env_path = Path(__file__).resolve().parent.parent / ".env"
    if env_path.exists():
        with open(env_path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#") and "=" in line:
                    key, val = line.split("=", 1)
                    key = key.strip()
                    val = val.strip().strip("'").strip('"')
                    if key not in os.environ:
                        os.environ[key] = val

load_env()

API_KEY = os.environ.get("NEBIUS_TOKEN_FACTORY_KEY") or os.environ.get("NEBIUS_API_KEY")
BASE_URL = os.environ.get("ARCHIE_NEBIUS_BASE_URL", "https://api.tokenfactory.nebius.com/v1").rstrip("/")
MODEL = "nvidia/nemotron-3-super-120b-a12b"

if not API_KEY:
    print("\033[91m[Error] NEBIUS_TOKEN_FACTORY_KEY not found in .env or environment.\033[0m")
    sys.exit(1)

# Project root directory for safety and path resolution
PROJECT_ROOT = Path(__file__).resolve().parent.parent

# Define Agent Tools
TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read the contents of a file relative to the ArchieOS project root.",
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
                    "path": {"type": "string", "description": "Relative file path"},
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

def execute_tool(name, args):
    """Executes local functions requested by Nemotron."""
    print(f"\033[94m⚙ [Tool Call] {name}({json.dumps(args)})\033[0m")
    try:
        if name == "read_file":
            target = PROJECT_ROOT / args["path"]
            if not target.exists():
                return f"Error: File '{args['path']}' does not exist."
            with open(target, "r", encoding="utf-8", errors="replace") as f:
                content = f.read()
            return content[:15000] # bound content size

        elif name == "write_file":
            target = PROJECT_ROOT / args["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            with open(target, "w", encoding="utf-8") as f:
                f.write(args["content"])
            return f"Successfully wrote {len(args['content'])} bytes to {args['path']}."

        elif name == "list_directory":
            target = PROJECT_ROOT / args.get("path", ".")
            if not target.is_dir():
                return f"Error: '{args.get('path')}' is not a directory."
            items = os.listdir(target)
            return json.dumps(items[:100])

        elif name == "run_command":
            cmd = args["command"]
            proc = subprocess.run(cmd, shell=True, cwd=PROJECT_ROOT, capture_output=True, text=True, timeout=60)
            out = proc.stdout + ("\n" + proc.stderr if proc.stderr else "")
            return out.strip() if out.strip() else "(Command executed with no output, exit code: 0)"

        elif name == "archie_privacy_check":
            data = args["data"].lower()
            if any(k in data for k in ["secret", "private_key", "password", "token", "api_key"]):
                return "PrivacyGrade::Secret -> DISQUALIFIED: Cannot disclose to cloud. Restricted to local node."
            return "PrivacyGrade::Internal -> APPROVED for Nebius Token Factory."

        else:
            return f"Unknown tool: {name}"
    except Exception as e:
        return f"Tool Execution Error: {str(e)}"

def call_nemotron(messages):
    """Sends payload to Nebius Token Factory completions API."""
    url = f"{BASE_URL}/chat/completions"
    payload = {
        "model": MODEL,
        "messages": messages,
        "tools": TOOLS,
        "temperature": 0.2,
        "max_tokens": 1500
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
When the user asks you to inspect, modify, debug, or build features in ArchieOS:
1. Reason carefully about the task (explain your plan).
2. Invoke the necessary tools to inspect existing code or run tests.
3. Validate actions against ArchieOS invariants (Zero-Secret Leak, Strict Boundary).
4. Provide precise, actionable results.
"""

def main():
    print("=" * 70)
    print("\033[92m🧠 ArchieOS Nemotron Agentic v1\033[0m")
    print(f"Backend: \033[96mNebius Token Factory\033[0m | Model: \033[93m{MODEL}\033[0m")
    print("Zero-Secret Leak & Privacy Spine Active.")
    print("Type 'exit' or 'quit' to end session.")
    print("=" * 70)

    history = [{"role": "system", "content": SYSTEM_PROMPT}]

    # Optional single command line prompt mode
    if len(sys.argv) > 1:
        user_input = " ".join(sys.argv[1:])
        run_loop(history, user_input)
        return

    while True:
        try:
            user_input = input("\n\033[1;37mUser > \033[0m").strip()
            if not user_input:
                continue
            if user_input.lower() in ["exit", "quit"]:
                print("Exiting Nemotron Agentic session.")
                break
            run_loop(history, user_input)
        except (KeyboardInterrupt, EOFError):
            print("\nSession interrupted.")
            break

def run_loop(history, user_input):
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
                except:
                    fn_args = {}
                result = execute_tool(fn_name, fn_args)
                history.append({
                    "role": "tool",
                    "tool_call_id": tool_call["id"],
                    "content": str(result)
                })
            # Continue the loop so Nemotron processes the tool outputs
            continue
        else:
            if content:
                print(f"\n\033[92mNemotron Agentic >\033[0m {content.strip()}")
            break

if __name__ == "__main__":
    main()
