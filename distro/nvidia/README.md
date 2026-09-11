# Optional NVIDIA host profile

The base image remains GPU-agnostic. For a dedicated NVIDIA image, confirm that the
target GPU is supported by the current `nvidia-open` package and build with:

```bash
sudo ARCHIE_NVIDIA_PROFILE=1 ./distro/archiso/build.sh
```

The build appends `distro/nvidia/packages.x86_64` only for that image. The profile uses
the signed Arch repository packages for the open kernel module, userspace driver, and
NVIDIA container runtime.

NIM containers and model weights are deliberately not embedded in the ISO. They are
large, model-specific, and subject to their own access and license terms. Archie can
connect to a local NIM over loopback, a trusted HTTPS fabric endpoint, or Nebius through
the model-runtime adapter.
