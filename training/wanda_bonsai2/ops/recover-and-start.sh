#!/bin/bash
set -u
ROOT=/home/archie/wanda/run
WORKTREE=/mnt/c/Users/Penta/.codex/worktrees/wanda-bonsai9b/ArchieOS/training/wanda_bonsai2
CUTOFF="${1:-2026-10-23T17:00:00-03:00}"
MODE="${2:-auto}"

/opt/archie/iris-venv/bin/python - "$ROOT" "$WORKTREE" "$MODE" <<'PY'
import json, os, pathlib, signal, sys, time

root = pathlib.Path(sys.argv[1])
worktree = pathlib.Path(sys.argv[2])
mode = sys.argv[3]
sys.path.insert(0, str(worktree / "src"))
from archie_wanda.worker_status import _alive, _read

worker = _read(root / "worker.json")
alive = _alive(worker)

if mode == "restart" or not alive:
    print(f"[ArchieOS] Recuperando estado desde el último checkpoint (worker_alive={alive}, mode={mode})...")
    for p in pathlib.Path("/proc").glob("[0-9]*"):
        try:
            cmd = (p / "cmdline").read_bytes().replace(b"\0", b" ").decode("utf-8", "ignore")
            if "archie_wanda.cli worker" in cmd or "llama-server" in cmd:
                os.kill(int(p.name), signal.SIGTERM)
                print(f"[ArchieOS] Deteniendo proceso previo PID {p.name}")
        except Exception:
            pass
    time.sleep(2)

    # Reset starts=0 on any running or crash-failed jobs so power-offs never abort a job
    for sub in ("running", "failed"):
        for jf in sorted((root / "queue" / sub).glob("*.json")):
            try:
                d = json.loads(jf.read_text())
                if sub == "failed" and "crash/restart" not in str(d.get("last_error", "")):
                    continue
                d["starts"] = 0
                d.pop("last_error", None)
                target = root / "queue" / jf.name
                target.write_text(json.dumps(d, indent=2))
                jf.unlink()
                print(f"[ArchieOS] Trabajo recuperado desde último checkpoint: {jf.name} (starts=0)")
            except Exception as e:
                print(f"[ArchieOS] Aviso al recuperar {jf}: {e}")

    for jf in sorted((root / "queue").glob("*.json")):
        try:
            d = json.loads(jf.read_text())
            if d.get("starts", 0) > 0:
                d["starts"] = 0
                jf.write_text(json.dumps(d, indent=2))
        except Exception:
            pass

    (root / "control.json").write_text(json.dumps({"command": "run"}))
    print("[ArchieOS] control.json -> run. Listo para iniciar/reanudar worker.")
else:
    # Even if already alive, keep starts=0 on the active job so a sudden power cut won't penalize it
    for jf in sorted((root / "queue" / "running").glob("*.json")):
        try:
            d = json.loads(jf.read_text())
            if d.get("starts", 0) > 0:
                d["starts"] = 0
                jf.write_text(json.dumps(d, indent=2))
        except Exception:
            pass
    print(f"[ArchieOS] Worker activo (PID {worker.get('pid')}) detectado; checkpoints protegidos (starts=0).")
PY

bash "$WORKTREE/ops/ensure-worker.sh" "$CUTOFF" 2
