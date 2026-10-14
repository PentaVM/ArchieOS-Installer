#!/bin/bash
# Escritorio Archie Mac con la esfera en el fondo, sobre WSLg y la GPU (WSL la expone por D3D12).
# En una sesión Wayland nativa de Arch basta con arrancar el visor y `qml Main.qml`.
DIR="$(cd "$(dirname "$0")" && pwd)"
bash "$DIR/arrancar.sh"
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}" XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/mnt/wslg/runtime-dir}" QT_QPA_PLATFORM=wayland
export QML_XHR_ALLOW_FILE_READ=1 QTWEBENGINE_CHROMIUM_FLAGS='--no-sandbox --ignore-gpu-blocklist'
if [ -d /usr/lib/wsl/lib ]; then export LD_LIBRARY_PATH=/usr/lib/wsl/lib GALLIUM_DRIVER=d3d12 MESA_D3D12_DEFAULT_ADAPTER_NAME=NVIDIA; fi
cd "$DIR/../shell/prototype" && exec /usr/lib/qt6/bin/qml Main.qml
