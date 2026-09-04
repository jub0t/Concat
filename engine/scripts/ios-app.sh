#!/usr/bin/env bash
# Wraps the iOS build of the window into an app bundle.
#
#   scripts/ios-app.sh aarch64-apple-ios     [release|debug]
#   scripts/ios-app.sh aarch64-apple-ios-sim [release|debug]
#
# Build first (`cargo build -p concat --target aarch64-apple-ios --release`
# under `eval "$(scripts/mobile-env.sh aarch64-apple-ios)"`), then this
# lays out target/<target>/<profile>/Concat.app: the binary, an Info.plist,
# the icon, and the sherpa-onnx framework the binary loads, with the load
# path rewritten to the bundle. The bundle is ad-hoc signed, which is what
# a simulator accepts; a phone needs a developer identity and provisioning
# profile on top, with `codesign --force --sign "<identity>"`.
set -euo pipefail

target=${1:?target triple: aarch64-apple-ios or aarch64-apple-ios-sim}
profile=${2:-release}
engine=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$engine/.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$engine/Cargo.toml" | head -1)
ios_min=${IPHONEOS_DEPLOYMENT_TARGET:-15.0}

binary=$engine/target/$target/$profile/concat
[ -f "$binary" ] || { echo "no binary at $binary: build first" >&2; exit 1; }
framework=$(find "$engine/vendor/sherpa-onnx/ios" -maxdepth 2 -name '*.xcframework' | head -1)
[ -n "$framework" ] || { echo "no sherpa-onnx xcframework: run scripts/sherpa-mobile.sh $target" >&2; exit 1; }
slice=ios-arm64
[ "$target" = aarch64-apple-ios-sim ] && slice=ios-arm64_x86_64-simulator

app=$engine/target/$target/$profile/Concat.app
rm -rf "$app"
mkdir -p "$app/Frameworks"
cp "$binary" "$app/Concat"
cp -R "$framework/$slice/SherpaOnnxC.framework" "$app/Frameworks/"
cp "$root/assets/icons/concat_logo_512.png" "$app/AppIcon.png" 2>/dev/null || true

# The binary was linked against the dylib by the name the linker saw in
# vendor/; in the bundle it lives in Frameworks/ under the framework's
# own name.
old=$(otool -L "$app/Concat" | awk '/sherpa-onnx-c-api|SherpaOnnxC/ {print $1; exit}')
if [ -n "$old" ]; then
  install_name_tool -change "$old" "@rpath/SherpaOnnxC.framework/SherpaOnnxC" "$app/Concat"
fi
install_name_tool -add_rpath "@executable_path/Frameworks" "$app/Concat" 2>/dev/null || true

platform=iPhoneOS
[ "$target" = aarch64-apple-ios-sim ] && platform=iPhoneSimulator
cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Concat</string>
  <key>CFBundleDisplayName</key><string>Concat</string>
  <key>CFBundleIdentifier</key><string>app.concat.editor</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleExecutable</key><string>Concat</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleSupportedPlatforms</key><array><string>${platform}</string></array>
  <key>MinimumOSVersion</key><string>${ios_min}</string>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>UILaunchScreen</key><dict/>
  <key>UISupportedInterfaceOrientations</key>
  <array>
    <string>UIInterfaceOrientationPortrait</string>
    <string>UIInterfaceOrientationLandscapeLeft</string>
    <string>UIInterfaceOrientationLandscapeRight</string>
  </array>
  <key>UIRequiresFullScreen</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
  <key>NSPhotoLibraryUsageDescription</key><string>Concat imports the clips you pick.</string>
  <key>NSMicrophoneUsageDescription</key><string>Concat records voice-over.</string>
  <key>NSHumanReadableCopyright</key><string>AGPL-3.0-or-later. See THIRD_PARTY_NOTICES.md.</string>
</dict>
</plist>
PLIST

codesign --force --sign - "$app/Frameworks/SherpaOnnxC.framework"
codesign --force --sign - "$app"
echo "Bundled $app"
