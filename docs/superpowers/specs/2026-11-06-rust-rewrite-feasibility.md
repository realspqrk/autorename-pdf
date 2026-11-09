# Rust Rewrite + Scan-to-PDF — Feasibility (2026-11-06, rev. 2)

Codex (Astra, high) and Grok 4.7 (high) reviewed rev. 1. Both returned **go-with-changes**. Their corrections are merged into this revision.

## TOC
- [Verdict](#verdict)
- [Current baseline](#current-baseline)
- [Component mapping](#component-mapping)
- [Scan-to-PDF feature](#scan-to-pdf-feature)
- [Architecture: one app, GUI + CLI](#architecture-one-app-gui--cli)
- [Effort](#effort)
- [Risks](#risks)
- [Recommended plan](#recommended-plan)
- [Open decisions](#open-decisions)
- [Sources](#sources)
- [Appendix: console relay pattern (option B)](#appendix-console-relay-pattern-option-b)

## Verdict

**Technically feasible, nothing is blocked. It is still a 4–6 month project, and one risk dominates: text extraction parity with pdfplumber.**

| Question | Answer |
|---|---|
| Can all of it be done in Rust? | Yes. Every dependency has a permissively licensed Rust path. The one exception is mupdf, which is AGPL, so we avoid it. |
| GUI + CLI combined? | Yes as one codebase and one installer, with all logic in `core`. It's still two EXEs on Windows: a PE binary is either a GUI or a console app, so a thin console EXE is needed next to the GUI. See [the relay pattern](#appendix-console-relay-pattern-option-b) for the alternative design. |
| Scanner like Genius Scan? | Photos and scans only. **Gate: on par with or better than Genius Scan, Adobe Scan and Lens**, measured on a shared corpus. Achievable with DocAligner + classic fallback + linear-light cleanup, but not guaranteed. The gate decides. |
| Effort | **95–135 dev-days** for full port + scanner incl. gate. Scanner alone, in front of the current Python core: **33–50 dev-days**. |
| Biggest risk | PDFium text output differs from pdfplumber/pdfminer in reading order, spacing and repair. That changes the LLM input and the OCR/vision routing (quality score). |

**Recommendation:** start with a **5–8 day spike**. Its results decide whether to port the core at all. The scanner can proceed either way.

## Current baseline

| Item | Now |
|---|---|
| Core | ~3.9k LOC Python (+6.5k LOC tests). CLI floor is Python 3.11. PyInstaller EXE. |
| GUI | Tauri 2 + TypeScript (~3k LOC). Runs the CLI as a sidecar (`gui/src/lib/sidecar.ts`). Contract: JSON on stdout, `Processing [i/n]` progress on stderr, exit codes, `config inspect`. |
| Text | **pdfplumber/pdfminer** (`extract_text`, NFC, `repair=True`, cp1252 mojibake round-trip). pypdfium2 is used **only for rendering**. |
| OCR | PaddleOCR 3.7 venv (~500 MB, optional). PP-OCRv6_small for Latin scripts, **v5 script-specific recognizers** for CJK, Cyrillic, Arabic and others. GPU/device and custom detection-model settings. Text only; the bridge discards geometry. |
| AI | OpenAI and Anthropic native parse. instructor TOOLS mode for Gemini and xAI, JSON mode for Ollama. `reasoning.effort=none`, `store=false`. |
| Release | 65 MB portable ZIP + `setup.ps1` (context menu, optional OCR). |
| Config | YAML v2, PyYAML (**YAML 1.1**), legacy-key migration, dotenv precedence. The GUI only reads it. |

## Component mapping

| Python | Rust replacement | Confidence | Caveat |
|---|---|---|---|
| pdfplumber (text) | **pdfium-render 0.9.x** (or pdf_oxide, which is pure Rust but has one maintainer) | **Medium. Spike gate.** | Different engine. Reading order, spacing, repair and the quality score all shift. Word/line grouping has to be rebuilt on character boxes. The mojibake fix stays a cp1252 round-trip; this crate's `has_unicode_map_error()` does not replace it. |
| pypdfium2 (render) | pdfium-render | High | Same engine. Ships `pdfium.dll` (~5–6 MB). |
| PaddleOCR venv | **oar-ocr 0.10 on ort 2.0-rc** (Apache): PP-OCRv6 + v5 script models, PP-LCNet orientation, UVDoc, word boxes | Med-High | Inventory every script, device and det-model setting. Pin model hashes and dictionaries. GPU via ort execution providers, or document it as a dropped setting. Windows.Media.Ocr fallback is **disputed**: Codex says it needs package identity, while `winocr` works unpackaged. The spike settles it. |
| openai + anthropic + instructor | **async-openai 0.43** (`create_byot`, Responses) for OpenAI-compatible providers, plus a thin reqwest client for Anthropic `output_config.format` | Medium | Keep TOOLS vs JSON modes per provider, validation retries, refusal/incomplete handling and the error-to-exit-code mapping. 8–10 days, not 4–5. |
| pydantic dynamic models | Runtime JSON-schema builder + `jsonschema` validation | High | Per-provider schema rules: OpenAI strict means all fields required; Anthropic allows no min/max. |
| dateparser (DMY) | Own parser: ISO, `d.m.yyyy`, `d. Monat yyyy`, `15 March 2024`, EN/DE/FR/IT/ES months. Strftime-subset formatter for `output.date_format`. | Medium | A deliberate narrowing. Document it as a compatibility change. |
| rapidfuzz JaroWinkler | **rapidfuzz (Rust)** | High | Same metric as today, which keeps the `> 0.85` threshold stable. strsim scores differ on short strings. Replay the harmonization fixtures. |
| PyYAML | serde-saphyr 2.x | Medium | YAML **1.2**: `yes`/`no`/`on`/`off` are no longer booleans. Fail hard with a migration hint, and parse every example config with both parsers. |
| python-dotenv, argparse | dotenvy, clap 4 | High | Keep subcommands, JSON shapes, stderr progress and exit codes 0–11. |
| mupdf | **Do not use** | — | AGPL. |

## Scan-to-PDF feature

Scope (confirmed by Gerhard, 2026-11-06): **photos and scans only, no live camera.** Still images become a clean, **searchable** PDF, which then goes through the existing rename pipeline.

**Hard gate (Gerhard): output must be on par with or better than Genius Scan, Adobe Scan and Microsoft Lens. Otherwise the feature does not ship.**

Gate protocol (fair comparison, because every one of those apps can import photos from the gallery):
1. **Corpus:** about 40 photos in 8 categories (low-contrast desk, cluttered background, shadow or hand, glare, crumpled or curled, receipt, multi-column letter, colored paper or stamps), plus about 10 bad flatbed scans.
2. **Process each image** with Genius Scan, Adobe Scan, Lens (gallery import, default auto settings) and our pipeline (auto only, no manual corner adjust).
3. **Metrics:**
   - Corner IoU against hand-labelled quads.
   - OCR character error rate on each output. Run the same OCR engine over all of them.
   - File size per page.
   - Failed detections.
4. **Blind A/B review:** shuffled side-by-side pages, judged by a human and by Codex visual review.
5. **Pass:** we match or beat the best competitor on CER and IoU averaged over the corpus. No category is worse than the median competitor. The blind review prefers ours or calls it a tie on ≥ 50%.

If the gate fails, the fallback is to iterate: fine-tune the corner model on failing categories, or adopt a better cleanup model. Ship only after it passes.

| Stage | Technique / crate | Days |
|---|---|---|
| Load, EXIF rotate, downscale | `image`, `kamadak-exif` | 0.5 |
| Corner detection | **DocAligner** ONNX (Apache-2.0) via ort | 2–3 |
| Corner fallback | imageproc 0.27: Canny, contours, Douglas-Peucker, quad scoring, Hough | 3 |
| Perspective warp | `imageproc::Projection::from_control_points` at full resolution | 1 |
| Orientation + OCR (+ dewarp v2) | One **oar-ocr** session: PP-LCNet, PP-OCRv6, UVDoc | 2 |
| Shadow removal / white balance | Background division done **in linear light** (not sRGB), percentile stretch | 2–3 |
| Color modes | Color / gray / B&W (Sauvola) | 2 |
| PDF writer | pdf-writer: JPEG (color/gray), **Flate for bilevel**, one embedded font with ToUnicode, OCR boxes mapped through the homography, invisible text mode 3 | 4–6 |
| Corner-adjust UI | Small JPEG preview over IPC, drag handles + loupe, batch review grid; the warp runs in Rust | 5 |
| Benchmark set | Corner IoU, OCR CER, memory and latency on categories of difficult photos | 3 |
| **MVP total** | | **25–35** |

Parked for v2: CCITT G4 (no Rust encoder exists), PDF/A (krilla), UVDoc curl dewarp, live camera.

Must-haves: keep the original files, make processing adjustable, and always offer manual corner adjust.

Avoid: opencv-rust (Windows build pain) and the DocTr/DewarpNet weights (non-commercial or unclear licenses).

## Architecture: one app, GUI + CLI

**The Windows constraint.** The PE subsystem is fixed at link time. A `windows` subsystem EXE gets no console, so stdout/stderr are lost when it runs from a terminal. A `console` subsystem EXE flashes a console window when launched from Explorer. `AttachConsole(ATTACH_PARENT_PROCESS)` tricks give you broken prompts and no proper exit codes for shell scripts. So "combined" means **one codebase, one installer, all logic in one place**, plus a tiny console front end. Desktop apps that feel like one combined GUI + CLI ship two binaries this way. The mature form is a console relay, described in the [appendix](#appendix-console-relay-pattern-option-b).

Two options for AutoRename-PDF:

| Option | How | Pro | Con |
|---|---|---|---|
| **A. CLI links `core` directly** (recommended) | `autorename-pdf-cli.exe` statically links `crates/core` | Stateless batch tool, so no resident app is needed. Fast context-menu calls. No pipe protocol. | Context-menu runs show console output, not GUI progress |
| B. Console relay ([appendix](#appendix-console-relay-pattern-option-b)) | A thin Rust console EXE forwards argv/stdio/exit code over a named pipe. The GUI runs a CLI server on that pipe. | One running instance. Context-menu runs could show progress in the GUI. | The app must launch hidden per call (cold start). More moving parts. Only worth it if the GUI becomes resident (tray). |

Both options share `crates/core`. Moving from A to B later is cheap.

```
crates/core      -- config, profiles, providers, extraction, rename/undo, scanner
crates/cli       -- autorename-pdf-cli.exe  (console subsystem; also the GUI's worker)
gui/src-tauri    -- autorename-pdf.exe      (windows subsystem; Tauri commands)
```

- **Native PDF and OCR work stays in a worker process.** The GUI spawns the Rust CLI the way it spawns the sidecar today. That keeps the crash boundary, timeouts, cancellation and restart, and it preserves the existing sidecar contract (PDFium calls are serialized behind a mutex anyway). Cheap operations like config, undo listing and scanner previews can be direct Tauri commands.
- One NSIS installer (per user) with **both EXEs and the DLLs signed**. It registers today's per-user classic context menu. An updater needs a plugin and a hosted manifest, so that is a separate decision.
- **User-data migration:** config and log paths from the resource dir, portable mode, retention across uninstall, keeping secrets redacted in `config inspect`.

## Effort

| Workstream | Dev-days |
|---|---|
| Spike / feasibility gates | 5–8 |
| Config, profiles, schema builder, YAML 1.1 migration | 6–8 |
| Provider adapters (5 providers, modes, retries, error mapping) | 8–10 |
| Text extraction parity + quality score + mojibake | 6–10 |
| OCR (oar-ocr, model inventory, download/cache, device) | 5–7 |
| Rename, undo v2, harmonization, dates | 4–5 |
| CLI contract + GUI rewire | 5–7 |
| Tests: replayed provider responses, golden extraction text, filename rendering with mocked metadata | 8–12 |
| Installer, signing, context menu, data migration, release playbook | 6–8 |
| **Port subtotal** | **55–75** |
| Scanner MVP | 25–35 |
| Scanner par-or-better gate (corpus labelling, comparison tooling, tuning loop) | 8–15 (open-ended if the corner model needs fine-tuning) |
| Hardening + buffer | 5–10 |
| **Total** | **95–135** ≈ 5–7 months full time |

## Risks

| Risk | Sev. | Mitigation |
|---|---|---|
| pdfium/pdf_oxide text ≠ pdfplumber, so naming and routing drift | **High** | Spike gate: diff text on fixtures plus about 50 real invoices (tables, columns, broken fonts, rotation). No pass, no port. |
| Scanner falls short of commercial quality | High | Define the MVP as still images only. Benchmark, use adjustable presets, always do manual adjust. |
| Provider behavior drift once instructor is gone | Med | Replay tests for recorded failure responses; per-provider live smoke tests |
| OCR parity (scripts, GPU, custom models) | Med | Config inventory; explicitly drop or migrate unsupported settings |
| YAML 1.1 → 1.2, dateparser narrowing | Med | Hard error + migration hint; document the compatibility change |
| ort still RC; oar-ocr, hayro and pdf_oxide each mostly one maintainer | Med | Pin versions and model hashes; vendor if needed |
| Unsigned DLLs trigger SmartScreen / Smart App Control | Low-Med | Sign every binary |

Testing note: live LLM output varies, so a byte-diff against the Python JSON can never pass. Gate on golden extraction text, deterministic filename rendering and replayed provider responses instead. Live calls stay as statistical smoke tests.

## Recommended plan

0. **Spike, 5–8 days. This is the go/no-go.**
   - Text from pdfium-render vs pdfplumber on fixtures plus real invoices, scored with the existing quality function.
   - oar-ocr vs PaddleOCR on scanned fixtures.
   - DocAligner and a linear-light cleanup on the gate corpus, compared against Genius Scan, Adobe Scan and Lens (see the scanner gate protocol).
   - Windows.Media.Ocr from an unpackaged EXE.
   - A minimal searchable PDF that pdfplumber can read back.
1. **Spike fails on text parity:** build only the scanner in Rust, in front of the Python sidecar (33–50 days incl. gate), and stop the rewrite. **The scanner fails its par-or-better gate:** the scanner doesn't ship, and the port decision is made separately.
2. **Spike passes:** port in strangler order — deterministic logic (config, profiles, dates, harmonization, undo), then providers, then extraction and OCR last. Build the scanner on the same oar-ocr/ort stack in parallel. Python stays the reference until the gates pass.
3. **Cutover = v4.0:** NSIS installer, Python and PyInstaller deleted, the `/rename-pdfs` dev skill switches to `cargo run -p cli`.

## Open decisions

- Port the whole core, or only the scanner (running in front of the Python core)? Decide after the spike.
- CLI option A (links core directly) or B (console relay)? B only makes sense if the GUI becomes a resident tray app.
- OCR default: download oar-ocr models on first use (~30–60 MB including models) or keep them optional.
- Drop GPU OCR and the non-Latin scripts, or carry them over?
- Updater: Tauri updater + hosted manifest, or keep manual ZIP/installer releases?

## Sources

Crate data comes from the crates.io API on 2026-11-06.
- https://github.com/ajrcarey/pdfium-render
- https://github.com/yfedoseev/pdf_oxide
- https://github.com/GreatV/oar-ocr
- https://github.com/pykeio/ort
- https://huggingface.co/collections/PaddlePaddle/pp-ocrv6
- https://github.com/DocsaidLab/DocAligner
- https://github.com/tanguymagne/UVDoc
- https://docs.rs/imageproc/latest/imageproc/all.html
- https://github.com/marquaye/scanic
- https://docs.rs/pdf-writer
- https://docs.rs/krilla
- https://github.com/64bit/async-openai
- https://platform.claude.com/docs/en/build-with-claude/structured-outputs
- https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr
- https://v2.tauri.app/distribute/sign/windows/
- https://zng-ui.github.io/doc/zng_env/windows_subsystem/index.html
- https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/index.html
- https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights

## Appendix: console relay pattern (option B)

This appendix is self-contained: everything needed to build option B is here. It is a generic design, and the code is a sketch written for AutoRename-PDF, not taken from another project. Only build it if the GUI becomes resident (tray). Option A needs none of this.

### Shape

```
terminal / Explorer verb / agent
        │ argv, stdin, cwd, tty flags
        ▼
autorename-pdf-cli.exe  (console subsystem, ~300 KB, no logic)
        │ NDJSON frames over \\.\pipe\autorename-pdf-<random>
        ▼
autorename-pdf.exe      (windows subsystem, Tauri, owns crates/core)
        │ runs the command, streams out/err, asks for stdin, sends exit code
        ▼
crates/core  (same code option A links directly)
```

The relay holds no business logic, so it never drifts from the app. Help, validation and errors all come from the server. The relay only answers `--version`, `--no-launch`, and offline help when the app can't be reached.

### Discovery files

Both files sit in `%APPDATA%\autorename-pdf\cli\`. Write them atomically (temp file + rename).

| File | Written | Content | Purpose |
|---|---|---|---|
| `state.json` | On every app start and settings change | `{"enabled": true, "version": "4.0.0", "launch": {"exe": "...", "args": ["--background"]}}` | Tells the relay whether CLI access is on and how to start the app. It survives app exit. |
| `endpoint.json` | After the pipe listens; deleted on clean exit | `{"pipe": "\\\\.\\pipe\\autorename-pdf-<128-bit random>", "token": "<64 hex>"}` | Where to connect, and the session secret. |

The pipe name and token are new on every app start. A stale `endpoint.json` after a crash is harmless: the dial fails, and the relay treats that as "not running".

### Wire protocol (v1)

Messages are newline-delimited JSON, UTF-8, one object per line. Read lines with no length limit.

| Direction | Frame | Meaning |
|---|---|---|
| relay → app | `{"t":"hello","v":1,"token":"…","argv":[…],"cwd":"…","stdinTty":b,"stdoutTty":b,"stderrTty":b,"noColor":b}` | Always the first frame. The app rejects a wrong token or an unknown `v`. |
| app → relay | `{"t":"out","d":"…"}` / `{"t":"err","d":"…"}` | Write to stdout / stderr. This carries the existing JSON-on-stdout and `Processing [i/n]`-on-stderr contract unchanged. |
| app → relay | `{"t":"stdin"}` | The command wants stdin. The relay reads stdin only now, never eagerly. |
| relay → app | `{"t":"in","d":"…"}` … `{"t":"in-end","truncated":b}` | Stdin chunks (48 KB, split on UTF-8 boundaries), then the end marker. Cap at 12 MB. |
| app → relay | `{"t":"ping"}` | Keepalive during long OCR/LLM runs. It only resets the relay's watchdog. |
| app → relay | `{"t":"exit","code":n}` | Last frame. The relay exits with `n`, using the same exit codes 0–11. |

Unknown frame types are ignored, so the protocol can grow without breaking old relays. Bump `v` only for breaking changes.

### Relay flow

1. Parse relay-only flags (`--no-launch`, `--version`). Forward everything else untouched, including anything after `--`.
2. Resolve `%APPDATA%`. If it's missing, fail with exit 3. Never fall back to the cwd.
3. Read `state.json`. If `enabled` is false, fail with a hint to enable it in Settings.
4. Read `endpoint.json` and dial the pipe. If the pipe is busy (`ERROR_PIPE_BUSY` = 231), retry with backoff from 50 ms to 500 ms for up to 5 s. Once it has seen busy, **never launch**: the app is running, it just has no free instance.
5. **Permission denied** on a file or the pipe (an agent sandbox, for example) → fail with a clear "not allowed to reach the app" error. **Never launch** in that case either.
6. Not running → launch `state.launch` (or the EXE next to the relay) detached and hidden. Poll both files every 200 ms for up to 20 s. `--no-launch` skips this and fails instead.
7. Send `hello`, then loop on frames. A watchdog resets on every frame and kills the session after 120 s of silence (override: `AUTORENAME_CLI_TIMEOUT`). Synchronous Windows pipe handles don't support read deadlines, so the watchdog runs on a separate thread and closes the handle.

### Relay sketch (`crates/relay/src/main.rs`)

```rust
// Console subsystem (the Rust default). Dependencies: serde, serde_json.
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ERROR_PIPE_BUSY: i32 = 231;

#[derive(Deserialize)]
struct Endpoint { pipe: String, token: String }

#[derive(Deserialize)]
struct Launch { exe: String, args: Vec<String> }

#[derive(Deserialize)]
struct State { enabled: bool, launch: Option<Launch> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Hello<'a> {
    t: &'a str, v: u32, token: &'a str, argv: &'a [String], cwd: String,
    stdin_tty: bool, stdout_tty: bool, stderr_tty: bool, no_color: bool,
}

#[derive(Deserialize)]
struct Frame { t: String, d: Option<String>, code: Option<i32> }

enum Dial { Ok(fs::File, String), Busy, Denied, Absent }

fn cli_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("autorename-pdf").join("cli"))
}

fn dial(dir: &PathBuf) -> Dial {
    let raw = match fs::read(dir.join("endpoint.json")) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Dial::Denied,
        Err(_) => return Dial::Absent,
    };
    let Ok(ep) = serde_json::from_slice::<Endpoint>(&raw) else { return Dial::Absent };
    // A named pipe client opens like a file; std gives a synchronous handle.
    match OpenOptions::new().read(true).write(true).open(&ep.pipe) {
        Ok(f) => Dial::Ok(f, ep.token),
        Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => Dial::Busy,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Dial::Denied,
        Err(_) => Dial::Absent,
    }
}

fn connect(dir: &PathBuf, no_launch: bool) -> Result<(fs::File, String), (u8, &'static str)> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut backoff = Duration::from_millis(50);
    loop {
        match dial(dir) {
            Dial::Ok(f, t) => return Ok((f, t)),
            Dial::Denied => return Err((3, "Not allowed to reach AutoRename-PDF (sandbox?).")),
            Dial::Busy if Instant::now() < deadline => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_millis(500));
            }
            Dial::Busy => return Err((3, "AutoRename-PDF is busy; retry in a moment.")),
            Dial::Absent => break,
        }
    }
    if no_launch { return Err((3, "AutoRename-PDF is not running.")); }
    launch_app(dir)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
        if let Dial::Ok(f, t) = dial(dir) { return Ok((f, t)); }
    }
    Err((3, "AutoRename-PDF did not start within 20 seconds."))
}

fn launch_app(dir: &PathBuf) -> Result<(), (u8, &'static str)> {
    let state: Option<State> = fs::read(dir.join("state.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    if matches!(&state, Some(s) if !s.enabled) {
        return Err((3, "Command line access is off in AutoRename-PDF Settings."));
    }
    let (exe, args) = match state.and_then(|s| s.launch) {
        Some(l) => (PathBuf::from(l.exe), l.args),
        None => (std::env::current_exe().unwrap().with_file_name("autorename-pdf.exe"),
                 vec!["--background".into()]),
    };
    Command::new(exe).args(args)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn().map(|_| ()).map_err(|_| (3, "Could not start AutoRename-PDF."))
}

fn send(w: &mut impl Write, v: &serde_json::Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(v)?;
    line.push(b'\n');
    w.write_all(&line)
}

fn send_stdin(pipe: &mut fs::File) -> io::Result<()> {
    let mut buf = vec![0u8; 48 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    let mut total = 0usize;
    let mut stdin = io::stdin().lock();
    loop {
        let n = stdin.read(&mut buf)?; // a read error aborts: the app gets no in-end
        carry.extend_from_slice(&buf[..n]);
        // Keep an incomplete trailing UTF-8 sequence for the next chunk.
        let cut = if n == 0 { carry.len() } else {
            match std::str::from_utf8(&carry) {
                Ok(_) => carry.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => carry.len(), // genuinely invalid: lossy below
            }
        };
        let chunk: Vec<u8> = carry.drain(..cut).collect();
        if !chunk.is_empty() {
            total += chunk.len();
            if total > 12 * 1024 * 1024 {
                return send(pipe, &serde_json::json!({"t": "in-end", "truncated": true}));
            }
            let d = String::from_utf8_lossy(&chunk);
            send(pipe, &serde_json::json!({"t": "in", "d": d}))?;
        }
        if n == 0 { return send(pipe, &serde_json::json!({"t": "in-end"})); }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let no_launch = args.iter().any(|a| a == "--no-launch");
    let argv: Vec<String> = args.into_iter().filter(|a| a != "--no-launch").collect();
    let Some(dir) = cli_dir() else {
        eprintln!("APPDATA is not set.");
        return ExitCode::from(3);
    };
    let (mut pipe, token) = match connect(&dir, no_launch) {
        Ok(c) => c,
        Err((code, msg)) => { eprintln!("{msg}"); return ExitCode::from(code); }
    };
    let hello = Hello {
        t: "hello", v: 1, token: &token, argv: &argv,
        cwd: std::env::current_dir().unwrap_or_default().display().to_string(),
        stdin_tty: io::stdin().is_terminal(), stdout_tty: io::stdout().is_terminal(),
        stderr_tty: io::stderr().is_terminal(), no_color: std::env::var_os("NO_COLOR").is_some(),
    };
    if send(&mut pipe, &serde_json::to_value(&hello).unwrap()).is_err() { return ExitCode::from(1); }

    // Watchdog omitted for brevity: a thread that waits on an mpsc channel with
    // recv_timeout(idle) and, on timeout, closes the handle (CloseHandle via a
    // duplicated raw handle) so the blocking read below returns an error.
    let mut reader = BufReader::with_capacity(64 * 1024, pipe.try_clone().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => { eprintln!("Connection to AutoRename-PDF lost."); return ExitCode::from(1); }
            Ok(_) => {}
        }
        if line.trim().is_empty() { continue; }
        let Ok(f) = serde_json::from_str::<Frame>(&line) else { return ExitCode::from(1) };
        match f.t.as_str() {
            "out" => { let _ = io::stdout().write_all(f.d.unwrap_or_default().as_bytes()); }
            "err" => { let _ = io::stderr().write_all(f.d.unwrap_or_default().as_bytes()); }
            "stdin" => if send_stdin(&mut pipe).is_err() { return ExitCode::from(3) },
            "exit" => {
                let _ = io::stdout().flush();
                return ExitCode::from(f.code.unwrap_or(1).clamp(0, 255) as u8);
            }
            _ => {} // "ping" and unknown frames only reset the watchdog
        }
    }
}
```

### App-side server sketch (`gui/src-tauri/src/cli_server.rs`)

```rust
// Dependencies: tokio (net, io-util, rt), serde_json, rand, hex, constant_time_eq.
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

pub async fn serve(cli_dir: std::path::PathBuf) -> std::io::Result<()> {
    let pipe = format!(r"\\.\pipe\autorename-pdf-{:032x}", rand::random::<u128>());
    let token = hex::encode(rand::random::<[u8; 32]>());
    // first_pipe_instance(true) fails if anyone squatted the name.
    // Restrict the DACL to the current user SID (set SECURITY_ATTRIBUTES via
    // create_with_security_attributes_raw) so other local users can't connect.
    let mut server = ServerOptions::new().first_pipe_instance(true)
        .max_instances(4).create(&pipe)?;
    write_atomic(&cli_dir.join("endpoint.json"),
        &serde_json::json!({"pipe": pipe, "token": token}))?;
    loop {
        server.connect().await?;
        let conn = server;
        server = ServerOptions::new().max_instances(4).create(&pipe)?; // next listener
        let token = token.clone();
        tokio::spawn(async move { let _ = session(conn, &token).await; });
    }
}

async fn session(conn: NamedPipeServer, token: &str) -> std::io::Result<()> {
    let (rd, mut wr) = tokio::io::split(conn);
    let mut lines = BufReader::new(rd).lines();
    let hello: serde_json::Value = match lines.next_line().await? {
        Some(l) => serde_json::from_str(&l).unwrap_or_default(),
        None => return Ok(()),
    };
    let ok = hello["v"] == 1 && constant_time_eq::constant_time_eq(
        hello["token"].as_str().unwrap_or("").as_bytes(), token.as_bytes());
    if !ok {
        return frame(&mut wr, serde_json::json!({"t": "exit", "code": 11})).await; // auth
    }
    // Run the command through crates/core with writers that emit out/err frames.
    // A ping task sends {"t":"ping"} every 15 s while the command runs. A
    // stdin-reading command sends {"t":"stdin"} and collects in/in-end frames.
    // The exit code mapping is the one the CLI already uses (0–11).
    let code = run_cli_command(&hello, &mut wr, &mut lines).await;
    frame(&mut wr, serde_json::json!({"t": "exit", "code": code})).await
}

async fn frame(w: &mut (impl AsyncWriteExt + Unpin), v: serde_json::Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(&v)?;
    line.push(b'\n');
    w.write_all(&line).await
}
```

`write_atomic` writes a temp file and renames it over the target. `run_cli_command` is the same clap entry point `crates/cli` uses, with the stdout/stderr writers swapped out for frame writers.

### Security and robustness rules

- **The DACL limits the pipe to the current user.** The random name, `first_pipe_instance(true)` and the token are layered on top. Check the token in constant time.
- **Never launch on busy or permission errors.** A launch storm or a hidden second instance is worse than a clear error.
- **The relay never reads stdin unless asked.** Otherwise `cmd | autorename-pdf-cli rename x.pdf` would block, or swallow input meant for another process.
- **Keep the existing stdout/stderr split.** The GUI sidecar contract and the `--output json` consumers must behave the same whether they talk to the relay or option A's direct CLI.
- **Version handshake:** the relay prints its own version with `--version`, plus the app version from `state.json`. A mismatched `v` returns a clear "update the app" error, not a hang.
- **Tests:** inject the filesystem, dialer, launcher and clock behind small traits, so busy/denied/absent/launch-timeout paths run as unit tests without real pipes. Add one integration test against a real named pipe on Windows CI.

### Effort if chosen

About 6–9 dev-days on top of option A: relay ~2, server + frame writers ~2–3, discovery/launch/settings toggle ~1–2, tests ~1–2. The `crates/core` API is the same either way, which is why A → B later stays cheap.
