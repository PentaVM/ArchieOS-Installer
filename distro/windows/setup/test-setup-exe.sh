#!/usr/bin/env bash
# Checks ArchieOS-Setup.exe's own logic on Linux, under .NET 8, with a stand-in for Windows
# PowerShell that logs how it is called and does to the launcher's folder what
# Install-ArchieOS-Launcher.ps1 does. Checked: the files it carries are the repository's;
# --install hands the installer script those files and the exe; an installed build opens the
# launcher with its arguments; a different build, or a missing file, installs again; a failed
# install exits 1 with the reason in the log. Not checked: the window (and so the first run's
# opening on Install), and anything Windows itself does (the script, the shortcuts, Settings → Apps).
#
#   distro/windows/setup/test-setup-exe.sh [EXE]      (default: dist/ArchieOS-Setup.exe)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
exe="$(realpath "${1:-$repo/dist/ArchieOS-Setup.exe}")"
t="$(mktemp -d)"
trap 'rm -rf "$t"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# .NET 8 runs the .NET Framework exe; Windows' folders are made of these environment variables
# (on Linux a backslash is part of a file name, so ArchieOS\Launcher is one folder). The profile
# has a space in its name, as many Windows user names do.
cat > "$t/rc.json" <<'EOF'
{"runtimeOptions":{"tfm":"net8.0","framework":{"name":"Microsoft.NETCore.App","version":"8.0.0"},"rollForward":"LatestMajor"}}
EOF
export XDG_DATA_HOME="$t/App Data" TMPDIR="$t/tmp" WINDIR="$t/win" CALLS="$t/calls"
mkdir -p "$XDG_DATA_HOME" "$TMPDIR" "$WINDIR/System32"
target="$XDG_DATA_HOME/ArchieOS\\Launcher"
run() { dotnet exec --runtimeconfig "$t/rc.json" "$@"; }

ps="$WINDIR/System32/WindowsPowerShell\\v1.0\\powershell.exe"
cat > "$ps" <<'EOF'
#!/usr/bin/env bash
# One line per call, each argument followed by "|": how the quoting came through.
printf '%s|' "$@" >> "$CALLS"; echo >> "$CALLS"
while [ $# -gt 0 ]; do
  case "$1" in
    -File) script="$2"; shift ;;
    -SetupExe) setup="$2"; shift ;;
  esac
  shift
done
case "$script" in
  */Install-ArchieOS-Launcher.ps1)
    [ -z "${FAIL:-}" ] || { echo "Access to the path is denied." >&2; exit 1; }
    target="$XDG_DATA_HOME/ArchieOS\\Launcher"
    mkdir -p "$target"
    for f in "$(dirname "$script")"/*; do
      [ "$(basename "$f")" = Install-ArchieOS-Launcher.ps1 ] || cp "$f" "$target/"
    done
    [ "$(realpath "$setup")" = "$(realpath "$target/ArchieOS.exe" 2>/dev/null || true)" ] || cp "$setup" "$target/ArchieOS.exe"
    echo "ArchieOS launcher 0.1.0 installed in $target" ;;
esac
EOF
chmod +x "$ps"

# 1. The files it carries are byte for byte the repository's.
run "$exe" --extract "$t/x"
while read -r path name; do
  case "$path" in ''|'#'*) continue ;; esac
  cmp -s "$repo/$path" "$t/x/${name:-$(basename "$path")}" || fail "embedded ${name:-$path} differs from $path"
done < "$here/payload.txt"
[ "$(ls "$t/x" | wc -l)" = "$(grep -cv '^\(#\|$\)' "$here/payload.txt")" ] || fail "extra embedded files"

# 2. --install: the script runs from a staged copy of those files with the exe, then the stage goes.
run "$exe" --install || fail "--install exited $?"
grep -q -- "^-NoProfile|-ExecutionPolicy|Bypass|-File|$TMPDIR/ArchieOS-Setup-[0-9]*-[0-9]*/Install-ArchieOS-Launcher.ps1|-SetupExe|$exe|\$" "$CALLS" || fail "install call: $(cat "$CALLS")"
[ -z "$(ls "$TMPDIR" | grep -v '^ArchieOS-Setup.log$')" ] || fail "stage left behind: $(ls "$TMPDIR")"
cmp -s "$exe" "$target/ArchieOS.exe" || fail "ArchieOS.exe is not the setup exe"
grep -q 'installed in' "$TMPDIR/ArchieOS-Setup.log" && grep -q '^exit 0' "$TMPDIR/ArchieOS-Setup.log" || fail "log: $(cat "$TMPDIR/ArchieOS-Setup.log")"

# 3. Installed, this build only opens the launcher, passing its arguments; so does the installed copy.
: > "$CALLS"
run "$exe" -Page modelo
run "$target/ArchieOS.exe" -Autostart
[ "$(wc -l < "$CALLS")" = 2 ] || fail "expected two launcher calls: $(cat "$CALLS")"
launcher="-NoProfile|-ExecutionPolicy|Bypass|-STA|-WindowStyle|Hidden|-File|$target/ArchieOS-Launcher.ps1|"
grep -qxF -- "${launcher}-Page|modelo|" "$CALLS" || fail "launcher call: $(cat "$CALLS")"
grep -qxF -- "${launcher}-Autostart|" "$CALLS" || fail "autostart call: $(cat "$CALLS")"

# 4. Another build, or a missing file, is installed again: the windowed install starts (here it
#    stops at the window, which needs Windows) and the launcher is not opened first.
for case_ in other-build missing-file; do
  rm -rf "$target"; run "$exe" --install
  if [ "$case_" = other-build ]; then printf 'x' >> "$target/ArchieOS.exe"; else rm "$target/local-models.tsv"; fi
  : > "$CALLS"
  if run "$exe" > "$t/out" 2>&1; then fail "$case_: it opened instead of installing"; fi
  grep -q 'System.Windows.Forms' "$t/out" || fail "$case_: $(head -5 "$t/out")"
  [ ! -s "$CALLS" ] || fail "$case_: called $(cat "$CALLS")"
done

# 5. A failed install exits 1 and keeps PowerShell's reason in the log.
rm -rf "$target"
if FAIL=1 run "$exe" --install; then fail "a failed install exited 0"; fi
grep -q 'Access to the path is denied.' "$TMPDIR/ArchieOS-Setup.log" && grep -q '^exit 1' "$TMPDIR/ArchieOS-Setup.log" || fail "log: $(cat "$TMPDIR/ArchieOS-Setup.log")"
[ ! -e "$target/ArchieOS.exe" ] || fail "a failed install left ArchieOS.exe"

echo "ArchieOS-Setup.exe: all checks passed ($(sha256sum "$exe" | cut -c1-16))"
