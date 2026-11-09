# macOS release (build, sign, notarize)

Runs on a Mac; Linux/Windows hosts cannot sign or notarize. Not validated end to end yet: the first run must confirm every step and update this file (and `known.md` for traps).

## TOC
- [1. Prerequisites](#1-prerequisites)
- [2. Build](#2-build)
- [3. Sign and notarize](#3-sign-and-notarize)
- [4. Verify](#4-verify)
- [5. Publish](#5-publish)

## 1. Prerequisites
- Apple Developer Program membership; a **Developer ID Application** certificate in the login keychain (`security find-identity -v -p codesigning` lists it).
- Xcode Command Line Tools (`xcode-select --install`): `codesign`, `xcrun notarytool`, `xcrun stapler`.
- Rust via rustup (`rustup target add aarch64-apple-darwin x86_64-apple-darwin`), repo checkout at the release tag.
- Notarization credentials, one of: App Store Connect API key (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`) or Apple ID (`APPLE_ID`, `APPLE_PASSWORD` = app-specific password, `APPLE_TEAM_ID`). Never commit them.

## 2. Build
```bash
cargo xtask fetch-natives
cargo xtask dist                                   # Apple silicon: .app + .dmg in target/dist/aarch64-apple-darwin/
cargo xtask dist --target x86_64-apple-darwin      # Intel: OCR off (no ORT build for this target)
```
Smoke test before signing: `target/dist/<triple>/stage/bin/autorename-pdf --version` and `inspect examples/demo/acme-invoice.pdf` (proves PDFium loads).

## 3. Sign and notarize
cargo-packager 0.11.8 signs and notarizes the `.app`/`.dmg` when configured:
```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: <Name> (<TEAMID>)"
# plus the notarization variables from step 1
cargo xtask dist        # re-run: packager signs with hardened runtime, then submits to notarytool and staples
```
If the installed packager version ignores the env var, add `signing-identity` under `[macos]` in a local, uncommitted copy of `Packager.toml`, or sign manually:
```bash
APP="target/dist/aarch64-apple-darwin/AutoRename-PDF.app"
find "$APP/Contents" -name '*.dylib' -exec codesign --force --timestamp --options runtime -s "$APPLE_SIGNING_IDENTITY" {} \;
codesign --force --timestamp --options runtime -s "$APPLE_SIGNING_IDENTITY" "$APP"
ditto -c -k --keepParent "$APP" app.zip
xcrun notarytool submit app.zip --keychain-profile <profile> --wait     # or --apple-id/--team-id/--password
xcrun stapler staple "$APP"
```
Sign nested dylibs (PDFium) before the bundle; never use `--deep` for signing.

## 4. Verify
```bash
codesign --verify --strict --verbose=2 "$APP"
spctl -a -vv "$APP"                    # "accepted", source=Notarized Developer ID
xcrun stapler validate "$APP"
open "$APP"                            # opens without a Gatekeeper warning; Settings > About shows the version
"$APP/Contents/MacOS/autorename-pdf" integrate install && "$APP/Contents/MacOS/autorename-pdf" integrate status
```
Finder Quick Actions ("Rename now", "Preview") appear under right-click > Quick Actions; then `integrate remove`.

## 5. Publish
Add the `.dmg` and its `SHA256SUMS-<triple>` to the GitHub release ([release.md](release.md) step 5), then update README's install table (macOS row) and `followup.md` (macOS signing entry).
