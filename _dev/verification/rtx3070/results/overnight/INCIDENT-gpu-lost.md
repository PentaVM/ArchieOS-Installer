# Incident: RTX 3070 lost during overnight run (2026-10-16 04:30 -03:00)

## What happened

- 04:26: last successful native prefix extraction (tranche-002, example 38/80).
  Tranche-001 was fully prepared (80/80 admitted, no rejections, max 520 tokens).
- 04:30:58 to 04:31:29: `NVIDIA Overlay.exe` (libcef.dll, 0xc0000005) and `dwm.exe`
  (0xe0464645) crashed repeatedly. DWM's exit names the NVIDIA GeForce RTX 3070 as the
  primary display device.
- After that, `Get-PnpDevice -Class Display` reports the RTX 3070 as `Unknown /
  CM_PROB_PHANTOM` (device not present). Windows `nvidia-smi` fails, WSL NVML reports
  "GPU access blocked by the operating system" and `torch.cuda.is_available()` is False.
- No `nvlddmkm`, WHEA or Kernel-Power events were logged in that window, so the root
  cause (driver, power or thermal) is **not established**. When it happened, GPU load from
  the trainer was light: a 9B native forward of ~500 tokens every ~16 s. Rocket League and
  the NVIDIA Overlay were also running and are no longer running.

## State preserved

- The worker (`/home/archie/wanda/run`) moved to `waiting_resources`. Resource errors
  never fail a job, so the queue is intact: prepare-t2, then four training runs. No
  training step had started, so no training work was lost.
- The watchdog (`ops/watchdog.ps1`) keeps running until 13:30. It only stops idle sleep
  and keeps WSL open.

## Recovery (owner)

1. Reboot Windows so the GPU re-enumerates. Then confirm with `nvidia-smi` in PowerShell.
2. Start the watchdog again with a new cutoff. It starts the worker, and the worker
   resumes the queue: already extracted examples come from cache and resumed training
   runs restart from `last.pt`.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File "$env:USERPROFILE\.codex\worktrees\wanda-bonsai9b\ArchieOS\training\wanda_bonsai2\ops\watchdog.ps1" -Cutoff 2026-10-16T23:00:00-03:00
```

3. To stop the run, write `{"command":"stop"}` to `/home/archie/wanda/run/control.json`.

If the GPU drops again under this light load, check the hardware (PCIe seating, PSU
connectors, temperatures) and reinstall the driver before trusting long runs.
