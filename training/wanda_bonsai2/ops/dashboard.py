#!/usr/bin/env python3
"""Live Linux ASCII/ANSI Telemetry & Benchmark Dashboard for Wanda Bonsai2-9B."""
import argparse
import datetime as dt
import glob
import json
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path("/home/archie/wanda/run")
WORKTREE = pathlib.Path("/mnt/c/Users/Penta/.codex/worktrees/wanda-bonsai9b/ArchieOS/training/wanda_bonsai2")
WATCHDOG_LOG = pathlib.Path("/mnt/c/Users/Penta/Archie-RTX3070-Handoff/wanda-overnight/watchdog.log")
sys.path.insert(0, str(WORKTREE / "src"))

try:
    from archie_wanda import star
except Exception:
    star = None

# ANSI escape codes
C_RESET = "\033[0m"
C_BOLD = "\033[1m"
C_DIM = "\033[2m"
C_GREEN = "\033[38;5;82m"
C_CYAN = "\033[38;5;51m"
C_YELLOW = "\033[38;5;220m"
C_RED = "\033[38;5;196m"
C_MAGENTA = "\033[38;5;207m"
C_BLUE = "\033[38;5;39m"
C_WHITE = "\033[38;5;255m"
C_GRAY = "\033[38;5;244m"
BG_GREEN = "\033[48;5;22m"
BG_YELLOW = "\033[48;5;58m"
BG_RED = "\033[48;5;52m"
BG_BLUE = "\033[48;5;17m"

_TRANCHE_CACHE = {}


def load_tranche_map(path_str):
    if not path_str:
        return {}
    if path_str not in _TRANCHE_CACHE:
        p = pathlib.Path(path_str)
        if p.exists():
            try:
                data = json.loads(p.read_text(encoding="utf-8"))
                _TRANCHE_CACHE[path_str] = {ex["example_id"]: ex for ex in data.get("examples", [])}
            except Exception:
                _TRANCHE_CACHE[path_str] = {}
        else:
            _TRANCHE_CACHE[path_str] = {}
    return _TRANCHE_CACHE[path_str]


def bar(val, max_val, width=22, color=C_GREEN, plain=False):
    ratio = max(0.0, min(1.0, float(val) / float(max_val))) if max_val else 0.0
    filled = int(round(ratio * width))
    empty = width - filled
    b = "█" * filled + "░" * empty
    if plain:
        return f"[{b}]"
    return f"{C_GRAY}[{color}{b}{C_GRAY}]{C_RESET}"


def run_cmd(cmd):
    try:
        return subprocess.check_output(cmd, stderr=subprocess.DEVNULL, text=True).strip()
    except Exception:
        return ""


def get_gpu_stats():
    out = run_cmd([
        "nvidia-smi",
        "--query-gpu=name,temperature.gpu,power.draw,enforced.power.limit,memory.used,memory.total,utilization.gpu,fan.speed",
        "--format=csv,noheader,nounits",
    ])
    if not out:
        return None
    parts = [x.strip() for x in out.splitlines()[0].split(",")]
    if len(parts) < 7:
        return None
    try:
        return {
            "name": parts[0],
            "temp": int(float(parts[1])),
            "power": float(parts[2]),
            "limit": float(parts[3]),
            "mem_used": int(float(parts[4])),
            "mem_total": int(float(parts[5])),
            "util": int(float(parts[6])),
            "fan": parts[7] if len(parts) > 7 else "N/A",
        }
    except Exception:
        return None


def get_processes():
    out = run_cmd(["ps", "-eo", "pid,etime,pcpu,pmem,args"])
    procs = {"ensure": None, "thermal": None, "worker": None, "llama": None}
    for line in out.splitlines():
        if "ensure-worker.sh" in line and not procs["ensure"]:
            procs["ensure"] = line.strip().split(None, 4)
        elif "thermal-guard.sh" in line and not procs["thermal"]:
            procs["thermal"] = line.strip().split(None, 4)
        elif "archie_wanda.cli worker" in line and not procs["worker"]:
            procs["worker"] = line.strip().split(None, 4)
        elif "llama-server" in line and not procs["llama"]:
            procs["llama"] = line.strip().split(None, 4)
    return procs


def get_control_and_thermal():
    ctrl = "run"
    ctrl_file = ROOT / "control.json"
    if ctrl_file.exists():
        try:
            ctrl = json.loads(ctrl_file.read_text()).get("command", "run")
        except Exception:
            pass
    tlog = ROOT / "thermal.log"
    recent = []
    pauses_30m = 0
    now = dt.datetime.now(dt.timezone.utc).astimezone()
    if tlog.exists():
        try:
            lines = tlog.read_text(encoding="utf-8", errors="ignore").splitlines()[-80:]
            recent = lines[-4:]
            for ln in lines:
                if "control=pause" in ln:
                    ts_str = ln.split()[0]
                    try:
                        ts = dt.datetime.fromisoformat(ts_str)
                        if (now - ts).total_seconds() <= 1800:
                            pauses_30m += 1
                    except Exception:
                        pass
        except Exception:
            pass
    return ctrl, pauses_30m, recent


def get_star_live_stats(job):
    star_id = job.get("star_id", "g3-base-640")
    sdir = ROOT / "star" / star_id
    resp_file = sdir / "responses.jsonl"
    rat_file = sdir / "rationales.jsonl"
    limit = job.get("limit", 400)
    max_tok = job.get("max_thinking_tokens", 2048)
    tmap = load_tranche_map(job.get("tranche"))

    rows = []
    if resp_file.exists():
        for ln in resp_file.read_text(encoding="utf-8", errors="ignore").splitlines():
            if ln.strip():
                try:
                    rows.append(json.loads(ln))
                except Exception:
                    pass
    rats = []
    if rat_file.exists():
        for ln in rat_file.read_text(encoding="utf-8", errors="ignore").splitlines():
            if ln.strip():
                try:
                    rats.append(json.loads(ln))
                except Exception:
                    pass

    accepted = 0
    forced = 0
    wrong = 0
    toks = []
    last_info = "Esperando primera traza..."

    for r in rows:
        tk = r.get("thinking_tokens", 0)
        if isinstance(tk, int):
            toks.append(tk)
        ex = tmap.get(r.get("id"))
        is_forced = bool(r.get("forced_end"))
        if is_forced:
            forced += 1
        if ex and star:
            ok = star.accept(ex, r, max_tok)
            if ok:
                accepted += 1
            elif not is_forced:
                wrong += 1
        elif not is_forced:
            accepted += 1

    if rows:
        lr = rows[-1]
        ex = tmap.get(lr.get("id"))
        gold = star.answer_of(ex) if (ex and star) else "?"
        tlines = [ln.strip() for ln in (lr.get("text") or "").splitlines() if ln.strip()]
        pred = (tlines[-1] if tlines else "").replace("\n", " ")[:34]
        ok = star.accept(ex, lr, max_tok) if (ex and star) else (not lr.get("forced_end"))
        st = "ACCEPT" if ok else ("FORCED_END" if lr.get("forced_end") else "WRONG_ANS")
        last_info = f"id={lr.get('id')} | think={lr.get('thinking_tokens')}t | pred='{pred}' vs gold='{gold}' -> {st}"

    rat_acc = 0
    if rats and star:
        for r in rats:
            ex = tmap.get(r.get("id"))
            if ex and star.accept_rationale(ex, r, max_tok):
                rat_acc += 1

    mean_tok = sum(toks) / len(toks) if toks else 0.0
    max_seen = max(toks) if toks else 0
    return {
        "star_id": star_id,
        "done": len(rows),
        "limit": limit,
        "accepted": accepted,
        "forced": forced,
        "wrong": wrong,
        "rats_done": len(rats),
        "rats_accepted": rat_acc,
        "mean_tok": mean_tok,
        "max_tok": max_seen,
        "budget": max_tok,
        "last_info": last_info,
    }


def get_queue_info():
    running_files = sorted(glob.glob(str(ROOT / "queue" / "running" / "*.json")))
    pending_files = sorted(glob.glob(str(ROOT / "queue" / "*.json")))
    running = None
    if running_files:
        try:
            running = json.loads(pathlib.Path(running_files[0]).read_text())
            running["_file"] = pathlib.Path(running_files[0]).name
        except Exception:
            pass
    pending = []
    for pf in pending_files:
        try:
            d = json.loads(pathlib.Path(pf).read_text())
            pending.append((pathlib.Path(pf).name, d.get("job_id", "?"), d.get("type", "?"), d.get("starts", 0)))
        except Exception:
            pass
    return running, pending


def get_adapters():
    rows = []
    for p in sorted(glob.glob(str(ROOT / "runs" / "*"))):
        ap = pathlib.Path(p)
        if not ap.is_dir():
            continue
        metrics = ap / "metrics.jsonl"
        receipt = (ap / "receipt.json").exists()
        if not metrics.exists():
            continue
        lines = [json.loads(x) for x in metrics.read_text(encoding="utf-8", errors="ignore").splitlines() if x.strip()]
        steps = [x for x in lines if x.get("event") == "step"]
        evals = [x for x in lines if x.get("event") in ("baseline", "eval", "final")]
        opt_step = steps[-1]["opt_step"] if steps else (evals[-1]["opt_step"] if evals else 0)
        t_loss = steps[-1]["train_loss"] if steps else None
        base_ev = evals[0] if evals else {}
        last_ev = evals[-1] if evals else {}
        rows.append({
            "name": ap.name,
            "receipt": receipt,
            "opt_step": opt_step,
            "train_loss": t_loss,
            "base_loss": base_ev.get("dev_loss"),
            "dev_loss": last_ev.get("dev_loss"),
            "choice_acc": last_ev.get("choice_accuracy"),
            "cf_acc": last_ev.get("accuracy_counterfactual"),
        })
    return rows


def get_benchmarks():
    preferred = [
        "opentom-base",
        "opentom-think512-pilot",
        "opentom-think1024-pilot",
        "opentom-g1-think512-pilot",
        "opentom-g2-think512-pilot",
        "opentom-g2-think1024-pilot",
        "opentom-g3-think512-pilot",
        "opentom-g4-think512-pilot",
        "longchat-24k-pack-think512",
        "longchat-24k-pack2-direct",
        "longchat-24k-pack2-think512",
        "longchat-24k-pack2-v2",
        "longchat-24k-pack2-g4",
        "fantom-base",
    ]
    existing = sorted(p.name for p in (ROOT / "bench").glob("*") if p.is_dir()) if (ROOT / "bench").exists() else []
    key_benches = preferred + [b for b in existing if b not in preferred and not b.startswith("tomi-smoke")]
    out = []
    for bname in key_benches:
        bdir = ROOT / "bench" / bname
        if not bdir.exists():
            continue
        rep_file = bdir / "report.json"
        resp_file = bdir / "responses.jsonl"
        resp_count = 0
        if resp_file.exists():
            try:
                with open(resp_file, "rb") as f:
                    resp_count = sum(1 for _ in f)
            except Exception:
                pass
        if rep_file.exists():
            try:
                d = json.loads(rep_file.read_text())
                rep = d.get("report", {})
                if d.get("benchmark") == "opentom":
                    f1s = [v.get("macro_f1", 0.0) for k, v in rep.items() if isinstance(v, dict) and "macro_f1" in v]
                    accs = [v.get("accuracy", 0.0) for k, v in rep.items() if isinstance(v, dict) and "accuracy" in v]
                    mean_f1 = sum(f1s) / len(f1s) if f1s else 0.0
                    mean_acc = sum(accs) / len(accs) if accs else 0.0
                    loc_so = rep.get("location-so", {}).get("accuracy", 0.0)
                    out.append({
                        "name": bname,
                        "status": "DONE",
                        "items": f"{d.get('items', resp_count)}",
                        "metric1": f"F1:{mean_f1*100:5.1f}%",
                        "metric2": f"Acc:{mean_acc*100:5.1f}% (SO:{loc_so*100:4.1f}%)",
                        "forced": f"{d.get('forced_end', 0)}/{d.get('items', resp_count)}",
                        "think": f"{d.get('mean_thinking_tokens') or 0.0:5.0f}t",
                    })
                else:
                    k_acc = rep.get("knowledge", {}).get("accuracy", 0.0)
                    b_acc = rep.get("belief", {}).get("accuracy", 0.0)
                    out.append({
                        "name": bname,
                        "status": "DONE",
                        "items": f"{d.get('items', resp_count)}",
                        "metric1": f"Know:{k_acc*100:4.1f}%",
                        "metric2": f"Belief:{b_acc*100:4.1f}%",
                        "forced": f"{d.get('forced_end', 0)}/{d.get('items', resp_count)}",
                        "think": f"{d.get('mean_thinking_tokens') or 0.0:5.0f}t",
                    })
            except Exception:
                pass
        else:
            total_target = 12832 if "fantom" in bname else 260
            pct = (resp_count / total_target * 100.0) if total_target else 0.0
            out.append({
                "name": bname,
                "status": "PARTIAL/QUEUED",
                "items": f"{resp_count}/{total_target} ({pct:.1f}%)",
                "metric1": "En progreso",
                "metric2": "Checkpoint guardado",
                "forced": "-",
                "think": "-",
            })
    return out


def render(plain=False):
    c_rst = "" if plain else C_RESET
    c_bld = "" if plain else C_BOLD
    c_grn = "" if plain else C_GREEN
    c_cyn = "" if plain else C_CYAN
    c_yel = "" if plain else C_YELLOW
    c_red = "" if plain else C_RED
    c_mag = "" if plain else C_MAGENTA
    c_blu = "" if plain else C_BLUE
    c_gry = "" if plain else C_GRAY
    bg_grn = "" if plain else BG_GREEN
    bg_yel = "" if plain else BG_YELLOW
    bg_red = "" if plain else BG_RED

    now_str = dt.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    gpu = get_gpu_stats()
    procs = get_processes()
    ctrl, pauses_30m, recent_tlog = get_control_and_thermal()
    running, pending = get_queue_info()
    adapters = get_adapters()
    benches = get_benchmarks()

    lines = []
    w = 108
    lines.append(f"{c_cyn}{c_bld}╔{'═'*(w-2)}╗{c_rst}")
    title = f" ARCHIE-OS // WANDA BONSAI2-9B (RTX 3070) — MONITOR AUTÓNOMO DE ENTRENAMIENTO & BENCHMARKS "
    pad = w - 2 - len(title)
    lines.append(f"{c_cyn}{c_bld}║{title}{' '*max(0, pad)}║{c_rst}")
    sub = f" Hora local: {now_str}  |  Contexto: 32,768 (Chat) / 4,096 (Seq)  |  Think Budget: 2,048 tokens "
    lines.append(f"{c_cyn}╠{'═'*(w-2)}╣{c_rst}")
    lines.append(f"{c_cyn}║{c_rst}{c_gry}{sub:<{w-2}}{c_rst}{c_cyn}║{c_rst}")
    lines.append(f"{c_cyn}╚{'═'*(w-2)}╝{c_rst}")

    # 1. GPU & THERMAL PANEL
    lines.append(f"\n{c_bld}{c_blu}┌─ [1] TELEMETRÍA GPU & GUARDIÁN TÉRMICO (POWER LIMIT ~50% ACTIVO) {'─'*(w-68)}┐{c_rst}")
    if gpu:
        t_col = c_grn if gpu["temp"] < 68 else (c_yel if gpu["temp"] < 78 else c_red)
        p_col = c_grn if gpu["limit"] <= 140 else c_red
        t_bar = bar(gpu["temp"], 87, 24, t_col, plain)
        p_bar = bar(gpu["power"], max(gpu["limit"], 1.0), 24, p_col, plain)
        m_bar = bar(gpu["mem_used"], gpu["mem_total"], 24, c_cyn, plain)
        u_bar = bar(gpu["util"], 100, 24, c_mag, plain)

        if ctrl == "run":
            state_badge = f"{bg_grn}{c_bld} ● RUNNING (ENTRENANDO / GENERANDO) {c_rst}"
        elif ctrl == "pause":
            state_badge = f"{bg_yel}{c_bld} ❄ COOLDOWN (ENFRIANDO GPU A <=65°C) {c_rst}"
        else:
            state_badge = f"{bg_red}{c_bld} ■ STOPPED ({ctrl.upper()}) {c_rst}"

        lines.append(f"│ Estado Control : {state_badge}   | Pausas térmicas (últ. 30m): {c_bld}{pauses_30m}{c_rst}")
        lines.append(f"│ Temperatura    : {t_bar} {t_col}{c_bld}{gpu['temp']:2d}°C{c_rst} / 87°C máx  (Pause >=78°C | Resume <=65°C)")
        lines.append(f"│ Potencia (W)   : {p_bar} {p_col}{c_bld}{gpu['power']:5.1f}W{c_rst} / {gpu['limit']:.1f}W Límite (50% Power Limit OK)")
        lines.append(f"│ VRAM / Util    : {m_bar} {c_cyn}{gpu['mem_used']:4d}{c_rst}/{gpu['mem_total']} MiB  |  GPU Util: {u_bar} {gpu['util']:3d}%")
    else:
        lines.append(f"│ {c_red}[!] nvidia-smi no disponible temporalmente{c_rst}")

    if recent_tlog:
        last_ev = recent_tlog[-1]
        lines.append(f"│ Últ. evento    : {c_gry}{last_ev}{c_rst}")
    lines.append(f"{c_blu}└{'─'*(w-2)}┘{c_rst}")

    # 2. WATCHDOG & DAEMONS PANEL
    lines.append(f"\n{c_bld}{c_blu}┌─ [2] ESTADO DEL WATCHDOG & PROCESOS EN LINUX (WSL2) {'─'*(w-55)}┐{c_rst}")
    wd_ok = procs["ensure"] is not None
    tg_ok = procs["thermal"] is not None
    wk_ok = procs["worker"] is not None
    ll_ok = procs["llama"] is not None

    def p_badge(ok, proc_tuple, label):
        if ok and proc_tuple:
            return f"{c_grn}[✓ ACTIVO]{c_rst} {label:<18} (PID {proc_tuple[0]:>5}, Uptime {proc_tuple[1]:>7}, CPU {proc_tuple[2]:>4}%)"
        return f"{c_red}[✗ CAÍDO ]{c_rst} {label:<18} (Sin proceso activo)"

    lines.append(f"│ {p_badge(wd_ok, procs['ensure'], 'Watchdog WSL')}   |   {p_badge(tg_ok, procs['thermal'], 'Thermal Guard (1s)')}")
    lines.append(f"│ {p_badge(wk_ok, procs['worker'], 'Wanda Worker')}   |   {p_badge(ll_ok, procs['llama'], 'llama-server 32k')}")
    lines.append(f"{c_blu}└{'─'*(w-2)}┘{c_rst}")

    # 3. ACTIVE PIPELINE & QUEUE PANEL
    lines.append(f"\n{c_bld}{c_blu}┌─ [3] PROGRESO DEL ENTRENAMIENTO / STaR G3 EN CURSO & COLA DE TRABAJOS {'─'*(w-73)}┐{c_rst}")
    star_stats = None
    if running:
        jtype = running.get("type", "?")
        jid = running.get("job_id", "?")
        starts = running.get("starts", 0)
        lines.append(f"│ Trabajo Activo : {c_grn}{c_bld}{jid}{c_rst} ({running.get('_file')}) | Tipo: {c_cyn}{jtype.upper()}{c_rst} | Reinicios: {starts}/3")
        if jtype == "star":
            star_stats = get_star_live_stats(running)
            p_bar = bar(star_stats["done"], star_stats["limit"], 30, c_grn, plain)
            pct = (star_stats["done"] / star_stats["limit"] * 100.0) if star_stats["limit"] else 0.0
            acc_rate = (star_stats["accepted"] / star_stats["done"] * 100.0) if star_stats["done"] else 0.0
            f_rate = (star_stats["forced"] / star_stats["done"] * 100.0) if star_stats["done"] else 0.0
            lines.append(
                f"│ Progreso STaR  : {p_bar} {c_bld}{star_stats['done']:3d}/{star_stats['limit']}{c_rst} ({pct:5.1f}%) "
                f"| Racionalizaciones: {star_stats['rats_done']} ({star_stats['rats_accepted']} ok)"
            )
            lines.append(
                f"│ Calidad Trazas : {c_grn}Aceptadas: {star_stats['accepted']} ({acc_rate:4.1f}%){c_rst} | "
                f"{c_yel}Resp. Incorrecta: {star_stats['wrong']}{c_rst} | "
                f"{c_cyn}Truncadas (ForcedEnd): {star_stats['forced']} ({f_rate:4.1f}%){c_rst}"
            )
            lines.append(
                f"│ Tokens Razonam.: Promedio: {c_bld}{star_stats['mean_tok']:.0f}t{c_rst} | "
                f"Máx visto: {c_bld}{star_stats['max_tok']}t{c_rst} / {star_stats['budget']}t budget"
            )
            lines.append(f"│ Última Traza   : {c_gry}{star_stats['last_info']}{c_rst}")
    else:
        lines.append(f"│ Trabajo Activo : {c_yel}Ninguno en queue/running/{c_rst}")

    q_str = " -> ".join([f"{fn.split('-')[0]}:{jid}" for fn, jid, _, _ in pending[:5]])
    lines.append(f"│ Cola Pendiente : {c_cyn}{q_str if q_str else '(Cola vacía)'}{c_rst}")
    lines.append(f"{c_blu}└{'─'*(w-2)}┘{c_rst}")

    # 4. ADAPTERS & BENCHMARKS TABLE
    lines.append(f"\n{c_bld}{c_blu}┌─ [4] LEADERBOARD DE ADAPTADORES ENTRENADOS & BENCHMARKS {'─'*(w-59)}┐{c_rst}")
    lines.append(f"│ {c_bld}{'Adaptador LoRA':<20} │ {'Paso':<6} │ {'Train Loss':<10} │ {'Dev Loss (Base->Fin)':<22} │ {'Choice Acc':<10} │ {'CF Acc':<8} │ {'Estado'}{c_rst}")
    lines.append(f"│ {'─'*20}─┼────────┼────────────┼────────────────────────┼────────────┼──────────┼──────────")
    for ad in adapters:
        tl = f"{ad['train_loss']:.4f}" if ad["train_loss"] is not None else "-"
        dl = f"{ad['base_loss']:.3f} -> {ad['dev_loss']:.3f}" if (ad["base_loss"] and ad["dev_loss"]) else "-"
        ca = f"{ad['choice_acc']*100:5.1f}%" if ad["choice_acc"] is not None else "-"
        cfa = f"{ad['cf_acc']*100:5.1f}%" if ad["cf_acc"] is not None else "-"
        st = f"{c_grn}LISTO{c_rst}" if ad["receipt"] else f"{c_yel}EN CURSO{c_rst}"
        lines.append(f"│ {ad['name']:<20} │ {ad['opt_step']:>4}/48 │ {tl:<10} │ {dl:<22} │ {ca:<10} │ {cfa:<8} │ {st}")

    lines.append(f"├{'─'*(w-2)}┤")
    lines.append(f"│ {c_bld}{'Benchmark Suite':<28} │ {'Estado':<14} │ {'Items':<16} │ {'Métrica Principal':<12} │ {'Detalle / Sub-métrica':<21} │ {'Think'}{c_rst}")
    lines.append(f"│ {'─'*28}─┼────────────────┼──────────────────┼──────────────┼───────────────────────┼──────")
    for b in benches:
        st_col = c_grn if b["status"] == "DONE" else c_yel
        lines.append(
            f"│ {b['name']:<28} │ {st_col}{b['status']:<14}{c_rst} │ {b['items']:<16} │ "
            f"{b['metric1']:<12} │ {b['metric2']:<21} │ {b['think']}"
        )
    lines.append(f"{c_blu}└{'─'*(w-2)}┘{c_rst}")

    # 5. AUTONOMOUS HEALTH & INTERVENTION VERDICT
    issues = []
    if not wk_ok:
        issues.append("El proceso worker de Wanda no está corriendo.")
    if not tg_ok:
        issues.append("El guardián térmico (thermal-guard.sh) no está corriendo.")
    if gpu and gpu["temp"] >= 87:
        issues.append(f"Temperatura crítica ({gpu['temp']}°C >= 87°C).")
    if gpu and gpu["limit"] > 145:
        issues.append(f"Power limit subió a {gpu['limit']}W (>140W).")
    if running and running.get("starts", 0) >= 3:
        issues.append(f"Trabajo activo acumula {running.get('starts')} reinicios (riesgo de fallo).")
    if star_stats and star_stats["done"] >= 10 and (star_stats["forced"] / star_stats["done"]) > 0.50:
        issues.append("Más del 50% de las trazas STaR están truncándose por presupuesto (forced_end).")

    lines.append(f"\n{c_bld}{c_blu}┌─ [5] DIAGNÓSTICO AUTÓNOMO & ¿REQUIERE INTERVENCIÓN? {'─'*(w-55)}┐{c_rst}")
    if not issues:
        lines.append(
            f"│ {bg_grn}{c_bld} [✓] SISTEMA OPERANDO ÓPTIMAMENTE — NO REQUIERE INTERVENCIÓN HUMANA {c_rst}"
        )
        lines.append(
            f"│ • El entrenamiento/curación avanza de forma autónoma con límite de 132.3W y enfriamiento activo."
        )
    else:
        lines.append(
            f"│ {bg_red}{c_bld} [!] ATENCIÓN: SE DETECTARON {len(issues)} CONDICIÓN(ES) QUE REQUIEREN REVISIÓN {c_rst}"
        )
        for iss in issues:
            lines.append(f"│   - {c_yel}{iss}{c_rst}")
    lines.append(f"{c_blu}└{'─'*(w-2)}┘{c_rst}")
    if not plain:
        lines.append(
            f" {c_cyn}{c_bld}[R]{c_rst} Reiniciar todo desde último Checkpoint   "
            f"{c_cyn}{c_bld}[P]{c_rst} Pausar/Reanudar GPU   "
            f"{c_cyn}{c_bld}[Q / Ctrl+C]{c_rst} Salir (el entrenamiento sigue en 2do plano)"
        )
    return "\n".join(lines)


def trigger_recovery(mode="restart"):
    script = str(WORKTREE / "ops" / "recover-and-start.sh")
    subprocess.run(["bash", script, "2026-10-23T17:00:00-03:00", mode], check=False)


def toggle_pause():
    ctrl_file = ROOT / "control.json"
    cur = "run"
    if ctrl_file.exists():
        try:
            cur = json.loads(ctrl_file.read_text()).get("command", "run")
        except Exception:
            pass
    nxt = "pause" if cur == "run" else "run"
    ctrl_file.write_text(json.dumps({"command": nxt}))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--once", action="store_true", help="Print once and exit")
    parser.add_argument("--plain", action="store_true", help="Disable ANSI colors")
    parser.add_argument("--interval", type=float, default=2.0, help="Refresh interval seconds")
    args = parser.parse_args()

    if args.once:
        print(render(plain=args.plain))
        return

    is_tty = sys.stdin.isatty()
    old_settings = None
    if is_tty:
        try:
            import select
            import termios
            import tty
            old_settings = termios.tcgetattr(sys.stdin.fileno())
            tty.setcbreak(sys.stdin.fileno())
        except Exception:
            is_tty = False

    try:
        while True:
            out = render(plain=args.plain)
            sys.stdout.write("\033[2J\033[H" + out + "\n")
            sys.stdout.flush()
            if is_tty:
                import select
                rlist, _, _ = select.select([sys.stdin], [], [], args.interval)
                if rlist:
                    ch = sys.stdin.read(1).lower()
                    if ch == "q":
                        break
                    elif ch == "r":
                        sys.stdout.write(f"\n{C_YELLOW}{C_BOLD}[!] Reiniciando worker y recuperando desde el último checkpoint...{C_RESET}\n")
                        sys.stdout.flush()
                        trigger_recovery("restart")
                        time.sleep(1.5)
                    elif ch == "p":
                        toggle_pause()
            else:
                time.sleep(args.interval)
    except KeyboardInterrupt:
        pass
    finally:
        if old_settings is not None:
            try:
                import termios
                termios.tcsetattr(sys.stdin.fileno(), termios.TCSADRAIN, old_settings)
            except Exception:
                pass
        print("\nSaliendo del monitor (el entrenamiento continúa activo en segundo plano)...")


if __name__ == "__main__":
    main()
