# macOS release (build, sign, notarize)

Runs on a Mac (build + DMG + tests are Mac-bound; `rcodesign` could sign/notarize from Linux but is not wired up). Steps 1–2 validated on the Mac mini 2026-11-09 (unsigned; `cargo test --workspace` green). Steps 3–5 not yet run: the first run must confirm them and update this file (and `known.md` for traps).

## TOC
- [1. Prerequisites](#1-prerequisites)
- [2. Build](#2-build)
- [3. Sign and notarize](#3-sign-and-notarize)
- [4. Verify](#4-verify)
- [5. Publish](#5-publish)

## 1. Prerequisites
- From Omarchy: the LAN Mac mini is driven by the global `mac-remote` skill (`~/.claude/skills/mac-remote/`): `mac.sh sync ~/Work/autorename-pdf`, then every step below as `mac.sh run 'cd ~/Code/autorename-pdf && …'` (GUI-session runner, keychain unlocked). Toolchain below is already installed there (2026-11-09).
- Apple Developer Program membership; a **Developer ID Application** certificate in the login keychain (`security find-identity -v -p codesigning` lists it).
- Xcode Command Line Tools (`xcode-select --install`): `codesign`, `xcrun notarytool`, `xcrun stapler`.
- Rust via rustup (`aarch64-apple-darwin`; Apple silicon only, no Intel build), repo checkout at the release tag.
- Notarization credentials (cargo-packager reads env), one of: `APPLE_KEYCHAIN_PROFILE=<profile>` (a `xcrun notarytool store-credentials` profile; checked first), App Store Connect API key (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`) or Apple ID (`APPLE_ID`, `APPLE_PASSWORD` = app-specific password, `APPLE_TEAM_ID`). Never commit them.

## 2. Build
```bash
cargo xtask fetch-natives
cargo xtask dist        # .app + .dmg in target/dist/aarch64-apple-darwin/
```
Smoke test before signing: `APP=target/dist/aarch64-apple-darwin/AutoRename-PDF.app; B="$APP/Contents/MacOS/autorename-pdf"; export AUTORENAME_PDF_DATA_DIR=/tmp/arp-smoke`, then `$B --version`; `$B inspect examples/demo/acme-invoice.pdf -o json` (PDFium loads from `Contents/Resources`); `$B models fetch latin`, then `inspect examples/demo/springfield-scan.pdf` → `ocr_quality` 1.0 (ORT works). GUI: `open -n --env AUTORENAME_GUI_DEMO=1 --env AUTORENAME_PDF_DATA_DIR=/tmp/arp-smoke/gui "$APP"` from the runner, then `screencapture -x -o -l <CGWindowNumber> out.png` (window capture works from the runner; full-screen VNC does not).

## 3. Sign and notarize
cargo-packager 0.11.8 signs only when `[macos] signing-identity` is set in the effective config; it ignores `APPLE_SIGNING_IDENTITY` and `xtask` does not forward it. It then signs + notarizes + staples the `.app`, but the `.dmg` is **only signed** (no notarize, no staple). Set the identity in a local, uncommitted `Packager.toml` copy, then:
```bash
# notarization vars from step 1 must be exported, else packager fails with MissingNotarizeAuthVars
CI=true cargo xtask dist   # CI=true → create-dmg --skip-jenkins (no Finder AppleScript; needed when unattended)
DMG=target/dist/aarch64-apple-darwin/<name>.dmg
xcrun notarytool submit "$DMG" --keychain-profile <profile> --wait   # or --key/--key-id/--issuer
xcrun stapler staple "$DMG"
```
Regenerate `SHA256SUMS-<triple>` after stapling (the DMG bytes changed). Manual fallback (sign without packager):
```bash
APP="target/dist/aarch64-apple-darwin/AutoRename-PDF.app"
find "$APP/Contents" -name '*.dylib' -exec codesign --force --timestamp --options runtime -s "$APPLE_SIGNING_IDENTITY" {} \;
codesign --force --timestamp --options runtime -s "$APPLE_SIGNING_IDENTITY" "$APP"
ditto -c -k --keepParent "$APP" app.zip
xcrun notarytool submit app.zip --keychain-profile <profile> --wait
xcrun stapler staple "$APP"
```
The fallback leaves the previously built DMG untouched: rebuild the DMG from the stapled `.app`, then sign, notarize and staple it as above. Sign nested dylibs (PDFium) before the bundle; never use `--deep` for signing. Over SSH the login keychain is locked (`errSecInternalComponent`): unlock it first (`security unlock-keychain`).

## 4. Verify
```bash
codesign --verify --strict --verbose=2 "$APP"
spctl -a -vv "$APP"                    # "accepted", source=Notarized Developer ID
xcrun stapler validate "$DMG"
syspolicy_check distribution "$APP"    # macOS 14+: stricter than spctl
xcrun stapler validate "$APP"
open "$APP"                            # opens without a Gatekeeper warning; Settings > About shows the version
"$APP/Contents/MacOS/autorename-pdf" integrate install && "$APP/Contents/MacOS/autorename-pdf" integrate status
```
Finder Quick Actions ("Rename now", "Preview") appear under right-click > Quick Actions; then `integrate remove`.

## 5. Publish
Add the `.dmg` and its `SHA256SUMS-<triple>` to the GitHub release ([release.md](release.md) step 5), then update README's install table (macOS row) and `followup.md` (macOS signing entry).
