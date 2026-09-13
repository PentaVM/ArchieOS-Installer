# Build the ArchieOS WSL distribution

Build inside an Arch Linux WSL distro with `arch-install-scripts`, Rust 1.88 or newer,
Git, gzip and zstd installed. The build uses official Arch repositories and records the
resolved package versions in the rootfs.

```bash
cd /path/to/ArchieOS-Nvidia
sudo ./scripts/wsl/build-rootfs.sh ./dist
```

The command creates `ArchieOS.wsl`, `ArchieOS-rootfs.tar.zst`, `SHA256SUMS`, the
PowerShell fallback installer, and build metadata. The modern artifact is a
gzip-compressed rootfs tar with its archive root at `/`; it contains no kernel or
initramfs because WSL supplies the kernel.

Install on WSL 2.4.4 or newer:

```powershell
wsl --install --from-file .\ArchieOS.wsl
```

Fallback:

```powershell
.\install-archie-wsl.ps1
```

The fallback needs `zstd.exe` in `PATH`, expands the `.tar.zst` into a temporary tar,
and calls `wsl --import`. After installation run `archie setup`, then `archie doctor`.

