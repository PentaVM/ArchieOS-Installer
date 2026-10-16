# Wanda on the exact Bonsai2 9B base

Implementation is incremental. Current maintained capabilities: bounded read-only GGUF3
inspection, F32/PTQ1_0 row decoding, frozen chunked Torch linear forward/input backward,
and conservative resource preflight. This is **not yet a complete trainer** or canonical
NF4 QLoRA. No added moral classifier or Core authority lives in this worker.

Run the CPU suite from this directory:

```sh
PYTHONPATH=src python -m unittest tests.test_identity tests.test_ptq tests.test_resources -v
```

Torch tests require the existing guest environment. They are skipped explicitly on a
host without Torch. Opt-in real CUDA/checkpoint verification:

```sh
PYTHONPATH=src ARCHIE_WANDA_HARDWARE=1 /opt/archie/iris-venv/bin/python -m unittest tests.test_hardware_projection -v
```

The hardware test hashes the real2GB base, checks actual Prism CPU unpacking, compares
the maintained CUDA operation and its input gradient against a dense reference, and
writes a new timestamped receipt under `_dev/verification/rtx3070/results/`. Its small
memory peak is for one projection; it does not establish full-model training capacity.
No weights are downloaded or modified. Do not install another multi-GB environment
before checking physical host disk headroom.

Implementation plan and source/benchmark boundaries are in
`_dev/verification/rtx3070/2026-10-16-wanda-trainer-plan.md` at the repository root.
