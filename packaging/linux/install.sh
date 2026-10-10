#!/usr/bin/env sh
# Installs texcopy for the current user (no root needed): the program, a menu entry and the icon.
#   ./install.sh            install under ~/.local
#   ./install.sh --remove   take it out again
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin"
apps="$prefix/share/applications"
icons="$prefix/share/icons/hicolor/256x256/apps"

if [ "${1:-}" = "--remove" ]; then
  rm -f "$bin/texcopy" "$bin/texcopy-gui" "$apps/texcopy.desktop" "$icons/texcopy.png"
  echo "removed texcopy from $prefix"
  exit 0
fi

mkdir -p "$bin" "$apps" "$icons"
install -m 755 "$here/texcopy" "$bin/texcopy"
install -m 644 "$here/texcopy.png" "$icons/texcopy.png"
# The menu entry names the program by full path, so it works even if ~/.local/bin is not on PATH.
sed "s|^Exec=texcopy|Exec=$bin/texcopy|" "$here/texcopy.desktop" > "$apps/texcopy.desktop"
chmod 644 "$apps/texcopy.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" >/dev/null 2>&1 || true

echo "installed texcopy to $bin/texcopy"
case ":$PATH:" in *":$bin:"*) ;; *) echo "note: $bin is not on your PATH; add it to run 'texcopy' from a terminal" ;; esac
echo "it is in your applications menu as 'texcopy'"
