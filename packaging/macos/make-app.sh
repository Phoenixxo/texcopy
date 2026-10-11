#!/usr/bin/env bash
# builds texcopy.app and a zip of it from a built binary. usage: make-app.sh [binary] [output-dir] [version]
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
bin="${1:-$root/target/release/texcopy}"
out="${2:-$root/dist}"
version="${3:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)}"
arch="$(uname -m)"
app="$out/texcopy.app"

[ -x "$bin" ] || { echo "no binary at $bin (run: cargo build --release)" >&2; exit 1; }
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/texcopy"

# icon from the 1024 master scaled to the sizes .icns needs.
iconset="$(mktemp -d)/texcopy.iconset"
mkdir -p "$iconset"
master="$root/packaging/icon/icon-1024.png"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$master" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$master" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/texcopy.icns"

# licence notices go with the program.
cp "$root/LICENSE" "$app/Contents/Resources/LICENSE"
python3 "$root/tools/third_party_licenses.py" "$app/Contents/Resources/THIRD_PARTY_LICENSES.md"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>texcopy</string>
  <key>CFBundleDisplayName</key><string>texcopy</string>
  <key>CFBundleIdentifier</key><string>dev.texcopy.texcopy</string>
  <key>CFBundleExecutable</key><string>texcopy</string>
  <key>CFBundleIconFile</key><string>texcopy</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# ad hoc signature, apple silicon needs one to run.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

zip="$out/texcopy-$version-macos-$arch.zip"
rm -f "$zip"
ditto -c -k --norsrc --keepParent "$app" "$zip" # no ._* files in the zip.
echo "built $app"
echo "built $zip ($(du -h "$zip" | cut -f1))"
