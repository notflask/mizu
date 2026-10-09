#!/usr/bin/env bash
# Assemble mizu.app from a built binary. Used by scripts/install.sh and the
# release workflow, so local and CI bundles are the same.
#
#   packaging/macos/make-app.sh <binary> <version> <out.app>

set -euo pipefail

[ $# -eq 3 ] || { echo "usage: $0 <binary> <version> <out.app>" >&2; exit 2; }
bin="$1"
version="${2#v}"
app="$3"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/mizu"
chmod 755 "$app/Contents/MacOS/mizu"
sed "s/@VERSION@/$version/g" "$here/Info.plist" >"$app/Contents/Info.plist"
cp "$repo/assets/icons/generated/macos/mizu.icns" "$app/Contents/Resources/mizu.icns"
printf 'APPL????' >"$app/Contents/PkgInfo"
