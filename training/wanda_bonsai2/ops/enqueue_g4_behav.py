#!/usr/bin/env python3
"""Generate the g4 behavioral psychology + PackStateWorld tranche and enqueue jobs 0420..0426."""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "src"))

from archie_wanda.chatdata import write_tranche
from archie_wanda.tomgen import generate_v3


def main():
    run_root = Path("/home/archie/wanda/run")
    tranche_path = run_root / "tranches" / "tom-gen-005-behav.json"
    problems = generate_v3(640, seed=20261019)
    write_tranche(
        problems,
        tranche_path,
        {
            "kind": "owned-procedural",
            "generator": "tomgen.generate_v3",
            "seed": 20261019,
            "count": 640,
        },
    )

    qdir = run_root / "queue"
    jobs = {
        "0420-bench-longchat-pack2-v2.json": {
            "job_id": "bench-longchat-pack2-v2",
            "type": "bench",
            "bench_id": "longchat-24k-pack2-v2",
            "benchmark": "longchat",
            "form": "pack2",
            "worlds": 20,
            "target_chars": 84000,
            "context": 32768,
            "max_tokens": 48,
            "starts": 0,
        },
        "0421-prepare-chat-tom-gen-005-behav.json": {
            "job_id": "prepare-chat-tom-gen-005-behav",
            "type": "prepare_chat",
            "tranche": str(tranche_path),
            "starts": 0,
        },
        "0422-star-g4-behav.json": {
            "job_id": "star-g4-behav-640",
            "type": "star",
            "star_id": "g4-behav-640",
            "tranche": str(tranche_path),
            "checkpoint": "/home/archie/wanda/run/runs/g3-star-qkvo-s17/best.pt",
            "limit": 400,
            "context": 32768,
            "think_budget": 2048,
            "max_thinking_tokens": 2048,
            "rationalize": True,
            "rationale_budget": 2048,
            "starts": 0,
        },
        "0423-prepare-chat-g4-behav.json": {
            "job_id": "prepare-chat-g4-behav",
            "type": "prepare_chat",
            "tranche": "/home/archie/wanda/run/tranches/g4-behav-640.json",
            "starts": 0,
        },
        "0424-train-g4-behav-star-s17.json": {
            "job_id": "g4-behav-star-qkvo-s17",
            "type": "train",
            "run_id": "g4-behav-star-qkvo-s17",
            "datasets": [
                "/home/archie/wanda/run/datasets/g4-behav-640.json",
                "/home/archie/wanda/run/datasets/tom-gen-005-behav.json",
                "/home/archie/wanda/run/datasets/g3-base-640.json",
                "/home/archie/wanda/run/datasets/tom-gen-003.json",
                "/home/archie/wanda/run/datasets/tranche-001.json",
                "/home/archie/wanda/run/datasets/tranche-002.json",
            ],
            "targets": ["q", "k", "v", "o"],
            "lr": 0.0002,
            "epochs": 2,
            "accumulation": 8,
            "eval_every": 40,
            "seed": 17,
            "max_minutes": 180,
            "max_optimizer_steps": 600,
            "answer_weight": 1.0,
            "keep_pairs": True,
            "starts": 0,
        },
        "0425-bench-opentom-g4-think512-pilot.json": {
            "job_id": "bench-opentom-g4-think512-pilot",
            "type": "bench",
            "bench_id": "opentom-g4-think512-pilot",
            "benchmark": "opentom",
            "checkpoint": "/home/archie/wanda/run/runs/g4-behav-star-qkvo-s17/best.pt",
            "batches": [1],
            "max_keys": 20,
            "mode": "think",
            "context": 32768,
            "think_budget": 2048,
            "answer_tokens": 32,
            "starts": 0,
        },
        "0426-bench-longchat-pack2-g4.json": {
            "job_id": "bench-longchat-pack2-g4",
            "type": "bench",
            "bench_id": "longchat-24k-pack2-g4",
            "benchmark": "longchat",
            "checkpoint": "/home/archie/wanda/run/runs/g4-behav-star-qkvo-s17/best.pt",
            "form": "pack2",
            "worlds": 20,
            "target_chars": 84000,
            "context": 32768,
            "mode": "think",
            "think_budget": 2048,
            "answer_tokens": 32,
            "starts": 0,
        },
    }
    for fname, payload in jobs.items():
        (qdir / fname).write_text(json.dumps(payload), encoding="utf-8")
    print(f"Wrote {tranche_path} ({len(problems)} problems) and {len(jobs)} queued jobs")


if __name__ == "__main__":
    main()
