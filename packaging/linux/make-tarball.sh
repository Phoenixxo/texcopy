#!/usr/bin/env bash
# Packs an already-built texcopy into a .tar.gz with an installer, a menu entry and the icon.
#   packaging/linux/make-tarball.sh [binary] [output-dir] [version]
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
bin="${1:-$root/target/release/texcopy}"
out="${2:-$root/dist}"
version="${3:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)}"
name="texcopy-$version-linux-$(uname -m)"

[ -x "$bin" ] || { echo "no binary at $bin (run: cargo build --release)" >&2; exit 1; }
stage="$out/$name"
rm -rf "$stage"
mkdir -p "$stage"
install -m 755 "$bin" "$stage/texcopy"
install -m 644 "$root/packaging/linux/texcopy.desktop" "$stage/texcopy.desktop"
install -m 644 "$root/packaging/icon/icon-256.png" "$stage/texcopy.png"
install -m 755 "$root/packaging/linux/install.sh" "$stage/install.sh"
install -m 644 "$root/README.md" "$root/LICENSE" "$stage/"
python3 "$root/tools/third_party_licenses.py" "$stage/THIRD_PARTY_LICENSES.md"
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "$stage"
echo "built $out/$name.tar.gz ($(du -h "$out/$name.tar.gz" | cut -f1))"
