# macOS release (build, sign, notarize)

Runs on a Mac (build + DMG + tests are Mac-bound; `rcodesign` could sign/notarize from Linux but is not wired up). Steps 1–4 validated on the Mac mini 2026-11-09 (4.0.0-beta.1: build, `cargo test --workspace`, signing, notarization, Gatekeeper on a quarantined copy).

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
One command (validated 2026-11-09, both submissions Accepted in ~1–3 min each):
```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Gerhard Petermeir (HCJ7D67RFZ)"
export APPLE_KEYCHAIN_PROFILE=spqrk-notarize    # notarytool profile on the Mac mini (team HCJ7D67RFZ)
cargo xtask dist --sign
```
xtask fails before building if either variable is missing. It passes the identity to cargo-packager as `[macos] signing-identity` (packager ignores `APPLE_SIGNING_IDENTITY` itself); packager signs every Mach-O (incl. `Contents/Resources/libpdfium.dylib`) with hardened runtime + timestamp, notarizes and staples the `.app`, then builds and signs the `.dmg`; xtask notarizes + staples the `.dmg` and writes `SHA256SUMS` last. Rejected submission: `xcrun notarytool log <id> --keychain-profile spqrk-notarize`. The DMG has no license click-through (xtask drops `license-file` for macOS; LICENSE ships in `Contents/Resources/licenses/`).

## 4. Verify
Test what a user downloads: a quarantined copy of the DMG and the app copied out of it.
```bash
D=target/dist/aarch64-apple-darwin; DMG=$(ls $D/*.dmg); T=/tmp/arp-verify; rm -rf $T; mkdir -p $T/mnt
(cd $D && shasum -a 256 -c SHA256SUMS)
xcrun stapler validate "$DMG"; spctl -a -t open --context context:primary-signature -vv "$DMG"   # accepted, Notarized Developer ID
cp "$DMG" $T/d.dmg; xattr -w com.apple.quarantine "0081;$(printf %x $(date +%s));Safari;" $T/d.dmg
hdiutil attach -nobrowse -readonly -mountpoint $T/mnt $T/d.dmg && ditto $T/mnt/AutoRename-PDF.app $T/AutoRename-PDF.app && hdiutil detach $T/mnt
xattr -w com.apple.quarantine "0081;$(printf %x $(date +%s));Safari;" $T/AutoRename-PDF.app; APP=$T/AutoRename-PDF.app
codesign --verify --strict --deep --verbose=2 "$APP"; codesign -dvv "$APP" 2>&1 | grep -E "flags|Authority=Developer|Timestamp"   # flags=0x10000(runtime)
spctl -a -vv "$APP"; xcrun stapler validate "$APP"; syspolicy_check distribution "$APP"
B="$APP/Contents/MacOS/autorename-pdf"; export AUTORENAME_PDF_DATA_DIR=$T/data
$B --version && $B models fetch latin && $B inspect examples/demo/springfield-scan.pdf -o json | grep '"ocr_quality": 1'   # ORT under hardened runtime
open --env AUTORENAME_GUI_DEMO=1 --env AUTORENAME_PDF_DATA_DIR=$T/gui "$APP"   # window appears; capture it with screencapture -o -l <CGWindowNumber> (never full screen: private desktop)
```
Finder Quick Actions: `$B integrate install`, right-click a PDF > Quick Actions > "Rename now"/"Preview", then `$B integrate remove` (not yet validated: needs someone at the Mac).

## 5. Publish
Pull and attach to the draft release that CI created ([release.md](release.md) step 5): `mac.sh pull Code/autorename-pdf/target/dist/aarch64-apple-darwin/ <local>`, rename `SHA256SUMS` to `SHA256SUMS-aarch64-apple-darwin`, `gh release upload vX.Y.Z <dmg> SHA256SUMS-aarch64-apple-darwin`.
