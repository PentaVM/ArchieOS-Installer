"""Small stdlib child used to test Rust framing/admission, not upstream behavior."""
import hashlib
import json
import sys
import time
from pathlib import Path

case = Path(sys.argv[1]).name
request = json.loads(sys.stdin.readline())
if case == "hang":
    time.sleep(0.6)
    Path(sys.argv[2]).write_text("child survived cancellation")
    time.sleep(10)
    raise SystemExit(0)
if case == "oversized":
    sys.stdout.write("x" * (request["limits"]["max_frame_bytes"] + 1))
    sys.stdout.flush()
    time.sleep(10)
    raise SystemExit(0)
prompt = "Fixture prompt from admitted synthetic data"
event = {"type": "inference", "protocol_version": 1, "request_id": "stage-1",
         "run_id": request["run_id"], "task_id": request["task_id"], "domain": request["domain"],
         "focus_id": request["focus_id"], "objective_ref": request["objective_ref"],
         "policy_revision_ref": request["policy_revision_ref"], "stage": "base1", "role": "solver",
         "response_format": "artifact", "prompt": prompt,
         "input_sha256": hashlib.sha256(prompt.encode()).hexdigest(),
         "reserved_input_tokens": len(prompt.encode()) + 256,
         "max_output_tokens": request["limits"]["per_call_output_tokens"],
         "reserved_cost_micros": request["limits"]["per_call_cost_micros"],
         "evidence_refs": [e["reference"] for e in request["evidence"]],
         "deadline_unix_ms": request["limits"]["deadline_unix_ms"]}
if case == "domain":
    event["domain"] = "personal/other"
elif case == "policy":
    event["policy_revision_ref"] = "policy-stale"
elif case == "digest":
    event["input_sha256"] = "0" * 64
elif case == "reservation":
    event["reserved_cost_micros"] = 0
elif case == "stage":
    event["stage"] = "c1"
elif case == "extra":
    event["authority"] = "model-granted"
encoded = json.dumps(event)
if case == "duplicate":
    encoded = encoded[:-1] + ', "domain":"personal/other"}'
print(encoded, flush=True)
if not sys.stdin.readline():
    raise SystemExit(1)
print(json.dumps({"type": "result", "protocol_version": 1, "status": "failed",
                  "error_code": "fixture_terminal",
                  "upstream_commit": "1cfe254e8d25b15f37048c3730134686abe60335", "usage": None}), flush=True)
