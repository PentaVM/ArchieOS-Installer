#!/usr/bin/env bash
# shellcheck disable=SC2034

iso_name="archieos"
iso_label="ARCHIEOS_01"
iso_publisher="PentaVM <https://github.com/PentaVM/ArchieOS-Nvidia>"
iso_application="ARCHIE Artificial Reasoning, Control and Hierarchical Intelligence Engine"
iso_version="0.1.0"
install_dir="archieos"
buildmodes=('iso')
bootmodes=('bios.syslinux'
           'uefi.systemd-boot')
pacman_conf="pacman.conf"
airootfs_image_type="squashfs"
airootfs_image_tool_options=('-comp' 'xz' '-Xbcj' 'x86,arm64' '-b' '1M' '-Xdict-size' '1M'
                            '-processors' '2' '-mem' '256M')
bootstrap_tarball_compression=('zstd' '-c' '-T0' '--auto-threads=logical' '--long' '-19')
file_permissions=(
  ["/etc/shadow"]="0:0:400"
  ["/etc/archie/archie.toml"]="0:0:640"
  ["/usr/bin/archie"]="0:0:755"
  ["/usr/bin/archied"]="0:0:755"
  ["/usr/bin/archie-db"]="0:0:755"
  ["/usr/bin/archie-displayd"]="0:0:755"
  ["/usr/bin/archie-worldd"]="0:0:755"
  ["/usr/bin/archie-cortexd"]="0:0:755"
  ["/usr/bin/archie-display"]="0:0:755"
)
