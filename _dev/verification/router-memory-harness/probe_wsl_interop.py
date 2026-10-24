"""Bounded, credential-free WSL control: Windows cmd exits immediately."""
import json
import os
import pathlib
import socket
import subprocess
import sys
import time

interop = os.environ.get("WSL_INTEROP", "")
explicit = len(sys.argv) > 1 and sys.argv[1] == "fresh"
child_env = None
if explicit:
    # Select only the current process's live interop socket, if one exists.
    # The directory contains control socket names, not user content or credentials.
    own_session = os.getsid(0)
    candidate = pathlib.Path("/run/WSL") / (str(own_session) + "_interop")
    if candidate.is_socket():
        child_env = os.environ.copy()
        child_env["WSL_INTEROP"] = str(candidate)
flags = {
    "wsl_interop_env_present": bool(interop),
    "wsl_interop_target_exists": os.path.exists(interop),
    "windows_cmd_exists": pathlib.Path("/mnt/c/Windows/System32/cmd.exe").is_file(),
    "inference_tested": False,
    "current_session_socket_available": child_env is not None,
    "explicit_current_session_route": explicit,
}
try:
    flags["wsl_interop_target_is_socket"] = pathlib.Path(interop).is_socket() if interop else False
except OSError:
    flags["wsl_interop_target_is_socket"] = False
start = time.monotonic()
try:
    child = subprocess.run(["/mnt/c/Windows/System32/cmd.exe", "/d", "/c", "exit", "0"],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15,
        env=child_env)
    flags.update({"cmd_exit_code": child.returncode, "stdout_bytes": len(child.stdout),
        "stderr_bytes": len(child.stderr),
        "wsl_accept_vsock_timeout": b"UtilAcceptVsock:250: accept4 failed 110" in child.stderr})
except subprocess.TimeoutExpired:
    flags["cmd_timed_out"] = True
flags["elapsed_seconds_rounded"] = round(time.monotonic() - start)
print(json.dumps(flags, sort_keys=True))
