#!/bin/bash
set -euo pipefail

: "${RELEASE_VERSION:?}"
: "${APPLE_SIGNING_IDENTITY:?}"
: "${APPLE_API_KEY_PATH:?}"
: "${APPLE_API_KEY:?}"
: "${APPLE_API_ISSUER:?}"
: "${RUNNER_TEMP:?}"

bundle="src-tauri/target/aarch64-apple-darwin/release/bundle"
app="$bundle/macos/Travel Gallery.app"
shopt -s nullglob
images=("$bundle"/dmg/*.dmg)
[[ ${#images[@]} -eq 1 ]] || { echo "Expected exactly one release DMG" >&2; exit 1; }
dmg="${images[0]}"
node --input-type=module -e '
    import { readFileSync } from "node:fs";
    import { assertFormalMacBundle, assertMacNativeTargets } from "./scripts/formal-macos-bundle.mjs";
    const app = process.argv[1];
    const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
    assertFormalMacBundle(app);
    assertMacNativeTargets(app, "arm64", config.bundle.macOS.minimumSystemVersion);
' "$app"
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")" == "com.travelgallery.desktop" ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")" == "$RELEASE_VERSION" ]]

verify_identity() {
    codesign --verify --strict --verbose=2 "$1"
    codesign --display --verbose=4 "$1" 2>&1 | grep -Fqx -- "Authority=$APPLE_SIGNING_IDENTITY"
}

codesign --verify --deep --strict --verbose=2 "$app"
verify_identity "$app"
for name in travel-gallery travel-gallery-ai travel-gallery-quality ffmpeg ffprobe; do
    binary="$app/Contents/MacOS/$name"
    [[ "$(lipo -archs "$binary")" == "arm64" ]]
    verify_identity "$binary"
done
xcrun stapler validate "$app"
spctl --assess --type execute --verbose=4 "$app"

# Tauri notarizes/staples the App. Submit the signed outer DMG separately too,
# so Gatekeeper can validate both the download and the contained offline App.
verify_identity "$dmg"
hdiutil verify "$dmg"
xcrun notarytool submit "$dmg" \
    --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" \
    --wait --timeout 30m --output-format json > "$RUNNER_TEMP/travel-gallery-dmg-notary.json"
node --input-type=module -e '
    import { readFileSync } from "node:fs";
    const result = JSON.parse(readFileSync(process.env.RUNNER_TEMP + "/travel-gallery-dmg-notary.json", "utf8"));
    console.log(`DMG notarization: ${result.status} (${result.id})`);
    if (result.status !== "Accepted") throw new Error("Apple did not accept the DMG");
'
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
verify_identity "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg"
