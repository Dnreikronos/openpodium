#!/bin/bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
    echo "macOS is required to build the application bundle." >&2
    exit 1
fi
profile=release
build_profile=release
if [[ "${1:-}" == --debug && $# == 1 ]]; then
    profile=debug
    build_profile=dev
elif [[ $# != 0 ]]; then
    echo "Usage: bash scripts/package-macos.sh [--debug]" >&2
    exit 1
fi

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
cargo build --locked --bin openpodium --profile "$build_profile"
mkdir -p "$repo_root/target/macos"
package_dir=$(mktemp -d "$repo_root/target/macos/package.XXXXXX")
mv "$package_dir" "$package_dir.noindex"
package_dir="$package_dir.noindex"
bundle="$package_dir/OpenPodium.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "${CARGO_TARGET_DIR:-$repo_root/target}/$profile/openpodium" "$bundle/Contents/MacOS/openpodium"
cp packaging/macos/openpodium-launcher "$bundle/Contents/MacOS/openpodium-launcher"
chmod 755 "$bundle/Contents/MacOS/openpodium-launcher"
cp packaging/macos/Info.plist "$bundle/Contents/Info.plist"
version=$(awk -F '"' '/^version = / {print $2; exit}' Cargo.toml)
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$bundle/Contents/Info.plist"
swift packaging/macos/generate-icon.swift "$package_dir/OpenPodium.iconset"
iconutil --convert icns "$package_dir/OpenPodium.iconset" --output "$bundle/Contents/Resources/OpenPodium.icns"
plutil -lint "$bundle/Contents/Info.plist"
codesign --force --sign - "$bundle/Contents/MacOS/openpodium"
codesign --force --sign - "$bundle"
codesign --verify --deep --strict "$bundle"
printf '\nBundle ready: %s\n' "$bundle"
