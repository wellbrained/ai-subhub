#!/bin/bash
# Native Mac test build. Ad-hoc signing is local only, not Developer ID/notarization.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/.*<Version>\(.*\)<\/Version>.*/\1/p' Directory.Build.props | tr -d '\r')
rid="osx-$(uname -m)"
if [ "$rid" = "osx-aarch64" ]; then rid=osx-arm64; fi
if [ "$rid" = "osx-x86_64" ]; then rid=osx-x64; fi
dotnet build AI.SubHub.slnx -c Release
dotnet test AI.SubHub.slnx -c Release --no-build
root="artifacts/AI-SubHub-$version-$rid-local-test-$(date +%Y%m%d%H%M%S)"
app="$root/AI SubHub.app"
mkdir -p "$app/Contents/Resources"
dotnet publish src/Hub.App/Hub.App.csproj -c Release -r "$rid" --self-contained true -p:DebugType=None -o "$app/Contents/MacOS"
cp src/Hub.App/Assets/hub.icns "$app/Contents/Resources/hub.icns"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>AI SubHub</string>
<key>CFBundleDisplayName</key><string>AI SubHub</string>
<key>CFBundleIdentifier</key><string>org.aisubhub.app</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleExecutable</key><string>AI.SubHub</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>hub.icns</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
<key>NSAppleEventsUsageDescription</key><string>AI SubHub opens Terminal to run the CLI profile and project you select.</string>
</dict></plist>
EOF
cp README.md LICENSE "$app/Contents/MacOS/"
chmod +x "$app/Contents/MacOS/AI.SubHub"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
ditto -c -k --keepParent "$app" "$root.zip"
echo "Local test ZIP: $root.zip"
