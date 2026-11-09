# Windows VM validation (installer, context menu, CLI)

Uses the Omarchy Windows VM. Drive it with the `windows-vm` skill: `vm.sh` is in `~/.claude/skills/windows-vm/scripts/`.
- There is no SSH. Commands run through the guest job runner: `vm.sh run '<powershell>'` returns stdout, stderr and the exit code.
- Job scripts must be ASCII only.
- The agent cannot see the guest screen. Ask the user for screenshots of menus.

## TOC
- [1. Build](#1-build)
- [2. Copy to the guest](#2-copy-to-the-guest)
- [3. Install](#3-install)
- [4. Verify the registry and icons](#4-verify-the-registry-and-icons)
- [5. CLI smoke](#5-cli-smoke)
- [6. Context menu (visual)](#6-context-menu-visual)
- [7. Uninstall](#7-uninstall)

## 1. Build
- Signed build, on a Windows host or in the VM: `cargo xtask dist --sign` ([release.md](release.md), Local signed build in the Windows VM).
- Unsigned test build from Linux (needs `makensis`): `cargo xtask dist --target x86_64-pc-windows-msvc`.
- Without makensis, test the bare exes instead: `cargo xtask dist --target x86_64-pc-windows-msvc --no-package`. Then use `target/dist/x86_64-pc-windows-msvc/stage/{bin,res}` and run `integrate install` by hand (step 3b).

## 2. Copy to the guest
```bash
vm.sh up            # the user approves polkit; keep RDP open (Explorer and the GUI need a desktop)
vm.sh runner        # once per VM session: the user pastes the printed line into an admin PowerShell
vm.sh push target/dist/x86_64-pc-windows-msvc/AutoRename-PDF_4.0.0_x64-setup.exe   # host ~/Windows = guest \\host.lan\Data
vm.sh run 'Copy-Item \\host.lan\Data\AutoRename-PDF_*-setup.exe $env:TEMP\'     # never execute from UNC
```
Each block in steps 3-7 runs as one `vm.sh run '...'` job (PowerShell 5.1).

Probe the runner first. The current `runner.ps1` caches `$p.Handle`, and older runners report an empty exit code (so `vm.sh` returns 1):
```bash
vm.sh run 'exit 7'; echo "got $?"                      # must print: got 7
vm.sh run 'Get-Date -Format o'; echo "got $?"          # must print a date, then: got 0
```
If either check fails, re-copy the runner (`vm.sh runner`) and have the user restart it in the guest.

## 3. Install
a) Installer (silent, per user, no UAC):
```powershell
& "$env:TEMP\AutoRename-PDF_4.0.0_x64-setup.exe" /S ; Start-Sleep 5
Test-Path "$env:LOCALAPPDATA\Programs\AutoRename-PDF\autorename-pdf.exe","$env:LOCALAPPDATA\Programs\AutoRename-PDF\autorename-pdf-gui.exe","$env:LOCALAPPDATA\Programs\AutoRename-PDF\pdfium.dll"
Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu\Programs" -Recurse -Filter "AutoRename-PDF*.lnk"
```
The install dir is `%LOCALAPPDATA%\Programs\AutoRename-PDF` (set in `packaging/nsis/installer.nsi`). App data stays in `%LOCALAPPDATA%\autorename-pdf` and config in `%APPDATA%\autorename-pdf`; uninstall never removes them. An install in the old location (`%LOCALAPPDATA%\AutoRename-PDF`) moves: the post-install `integrate install` deletes the old exes, DLLs and uninstaller there (status item `old-install`) and keeps the data files. Unsigned test installers can be quarantined by Defender ML (`Get-MpThreatDetection`; installer exit 2); signed release builds are the fix, do not weaken Defender. Confirm it with `(Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\AutoRename-PDF).InstallLocation`.

b) Bare exes: copy `stage\bin\*` and `stage\res\*` into one folder, then run `.\autorename-pdf.exe integrate install`.

## 4. Verify the registry and icons
```powershell
$exe = "$env:LOCALAPPDATA\Programs\AutoRename-PDF\autorename-pdf.exe"
& $exe integrate status -o json            # every item "installed"
foreach ($r in 'SystemFileAssociations\.pdf','Directory') {
  $k = "HKCU:\Software\Classes\$r\shell\AutoRenamePDF"
  Get-ItemProperty $k | Select MUIVerb,Icon,SubCommands,MultiSelectModel,AutoRenamePdfIntegration
  Get-ChildItem "$k\shell" | % { $_.PSChildName; (Get-ItemProperty "$($_.PSPath)\command").'(default)' }
}
# Icon resource embedded (both exes):
Add-Type -AssemblyName System.Drawing
[System.Drawing.Icon]::ExtractAssociatedIcon($exe).Size
(Get-Item $exe).VersionInfo | Select ProductName,FileVersion,FileDescription
# User PATH: the install folder is on it exactly once (added by `integrate install`):
$dir = Split-Path $exe
$p = (Get-Item HKCU:\Environment).GetValue('Path', '', 'DoNotExpandEnvironmentNames')
($p -split ';' | ? { $_.TrimEnd('\') -ieq $dir }).Count          # 1
(Get-Item HKCU:\Environment).GetValueKind('Path')                  # ExpandString
Get-ItemProperty HKCU:\Software\AutoRename-PDF\Integration | Select UserPath
```
Expected values:
- `Icon` is `<dir>\autorename-pdf.exe,0`.
- The commands are `"<dir>\autorename-pdf-gui.exe" --rename-now "%1"` and `"<dir>\autorename-pdf-gui.exe" "%1"`.
- Two subkeys: `1-rename-now` and `2-preview`.

## 5. CLI smoke
```powershell
& $exe --version
& $exe config path -o json
& $exe schema plan | Select -First 3
& $exe inspect C:\Windows\Help\*.pdf -o json   # or a PDF copied from \\host.lan\Data; proves pdfium.dll loads
& $exe integrate install ; $LASTEXITCODE        # idempotent: every item "unchanged", exit 0
```

## 6. Context menu (visual)
- Restart Explorer so the HKCU verbs load: `Stop-Process -Name explorer -Force` (it restarts on its own).
- Ask the user to right-click a PDF and a folder (the folder item itself; empty space inside a folder has no entry by design). Windows 11 shows the entries under "Show more options". Expect "AutoRename-PDF" (red icon) > "Rename now" and "Preview in AutoRename-PDF…".
- Multi-select test: select 5 PDFs and choose "Preview…". Exactly one window must open listing all 5, because the launcher coalesces Explorer's one-process-per-file launches.
- Scripted variant of the same test, as one job:
  ```powershell
  $g="$env:LOCALAPPDATA\Programs\AutoRename-PDF\autorename-pdf-gui.exe"
  1..5 | % { Copy-Item C:\arp-test.pdf "C:\arp-test-$_.pdf"; Start-Process $g -ArgumentList "--rename-now","C:\arp-test-$_.pdf" }
  Start-Sleep 8; $p = @(Get-Process autorename-pdf -EA 0); "count " + $p.Count; $p | Stop-Process -Force
  ```
  Expect `count 1`.
- Any job that starts the GUI MUST kill it before it ends. The runner waits for the whole process tree, so a GUI left open blocks every later job until someone closes the window.
- Rename-now with no config: the window opens on onboarding. Nothing is renamed.

## 7. Uninstall
```powershell
& "$env:LOCALAPPDATA\Programs\AutoRename-PDF\uninstall.exe" /S ; Start-Sleep 5
Test-Path "HKCU:\Software\Classes\SystemFileAssociations\.pdf\shell\AutoRenamePDF","HKCU:\Software\Classes\Directory\shell\AutoRenamePDF"   # False False
Test-Path "$env:LOCALAPPDATA\Programs\AutoRename-PDF\autorename-pdf.exe"   # False
$p = (Get-Item HKCU:\Environment).GetValue('Path', '', 'DoNotExpandEnvironmentNames')
$p -split ';' | ? { $_ -like '*AutoRename-PDF*' }                    # nothing
Test-Path HKCU:\Software\AutoRename-PDF                           # False
```
Report the results: pass or fail per step, plus screenshots for step 6.
