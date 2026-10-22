#!/usr/bin/env sh
# installs texcopy for this user, ./install.sh to install and --remove to take it out (--remove --purge also deletes the downloaded models).
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin"
apps="$prefix/share/applications"
icons="$prefix/share/icons/hicolor/256x256/apps"

remove=0
purge=0
for arg in "$@"; do
  case "$arg" in
    --remove) remove=1 ;;
    --purge) purge=1 ;;
    *) echo "usage: ./install.sh [--remove [--purge]]" >&2; exit 2 ;;
  esac
done
if [ "$purge" = 1 ] && [ "$remove" = 0 ]; then
  echo "--purge only goes with --remove" >&2
  exit 2
fi

if [ "$remove" = 1 ]; then
  # the program has to still be there to clean up after itself.
  if [ "$purge" = 1 ] && [ -x "$bin/texcopy" ]; then
    "$bin/texcopy" --uninstall --yes || echo "couldn't remove the downloads, run 'texcopy --uninstall' yourself" >&2
  fi
  rm -f "$bin/texcopy" "$bin/texcopy-gui" "$apps/texcopy.desktop" "$icons/texcopy.png"
  echo "removed texcopy from $prefix"
  exit 0
fi

mkdir -p "$bin" "$apps" "$icons"
install -m 755 "$here/texcopy" "$bin/texcopy"
install -m 644 "$here/texcopy.png" "$icons/texcopy.png"
# the menu entry uses the full path so it works when ~/.local/bin isn't on PATH.
sed "s|^Exec=texcopy|Exec=$bin/texcopy|" "$here/texcopy.desktop" > "$apps/texcopy.desktop"
chmod 644 "$apps/texcopy.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" >/dev/null 2>&1 || true

echo "installed texcopy to $bin/texcopy"
case ":$PATH:" in *":$bin:"*) ;; *) echo "note: $bin is not on your PATH; add it to run 'texcopy' from a terminal" ;; esac
echo "it is in your applications menu as 'texcopy'"
