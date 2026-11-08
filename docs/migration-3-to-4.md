# Migrating from AutoRename-PDF 3.x to 4.0

4.0 reads none of the 3.x files. This page lists every breaking change, one line each.

## TOC
- [Install](#install)
- [Config](#config)
- [Harmonization file](#harmonization-file)
- [CLI](#cli)
- [Undo](#undo)

## Install

- The portable ZIP, `setup.ps1`, the PaddleOCR venv and the Python CLI are gone. 4.0 ships a Windows installer (per user, no admin), a Linux AppImage/deb/pacman package, and one binary `autorename-pdf` (CLI + GUI).
- Before deleting a 3.x folder, run its `setup.ps1` once more and remove the context-menu entries; 4.0 does not touch 3.x registry entries. The 4.0 menu is "AutoRename-PDF > Rename now / Preview".
- In that uninstall, answer **No** to "Remove PaddleOCR installation?" once 4.0 is installed: it deletes all of `%LOCALAPPDATA%\autorename-pdf`, which 4.0 uses for its undo journal and OCR models. Delete only its `paddleocr-venv` and `python-embed` subfolders.
- OCR is built in: fetch its models once with `autorename-pdf models fetch` (or Settings > Processing > Reading in the GUI).

## Config

- Location: `--config` > `AUTORENAME_PDF_CONFIG` > OS config dir (`%APPDATA%\autorename-pdf\config.yaml`, `~/.config/autorename-pdf/config.yaml`, `~/Library/Application Support/autorename-pdf/config.yaml`). A `config.yaml` next to the executable is no longer picked up.
- `.env` is still read from the directory of the config file and still never overrides real environment variables.
- `config_version` is now `4`; any other value is an error. (You can also leave it out.)
- Unknown keys are errors. Before 4.0 they were silently ignored.
- YAML 1.2: `yes`/`no`/`on`/`off` are strings, not booleans. Write `true`/`false`.
- A `null` value (e.g. `model:` with nothing after it) means "use the default".
- `paddleocr:` section removed → `ocr:` with `language` (default `auto`: every installed pack; the latin pack reads English, German and 46 more at once) and `threads` (was `cpu_threads`). `det_limit_side_len`, `venv_path`, `device`, `use_gpu`, `languages` and `detection_model` are gone (OCR is built in and runs on the CPU).
- `pdf.extraction_mode` / `pdf.ocr_fallback` removed → use `pdf.ocr` and `pdf.vision` (`true | false | "auto"`).
- `pdf.ocr` and `pdf.vision` now default to `auto` (3.x: off). The route is free first: text layer → local OCR (when the text is poor) → page images to the model (only if text + OCR are still poor). With `ocr: auto` and no OCR models installed you get an `ocr_models_missing` warning and vision is tried instead of failing; `ocr: true` keeps the hard error. Under `privacy: offline` vision uses the local Ollama model. Set `vision: false` to never send page images.
- v1 keys (`openai.api_key`, …) are rejected as unknown keys.
- Top-level `filename:` is rejected as an unknown key → use `profiles.<id>.template`.
- `ai.model` default depends on the provider (3.x sent the OpenAI default to every provider).
- Types are checked strictly: `ai.temperature` must be a number, `ai.max_retries` an integer ≥ 0, `ocr.*` integers ≥ 1.
- New keys: `privacy: cloud | offline`, `enrich.enabled` (default `true`), `harmonization.auto_learn` (default `true`), `harmonization.decider: ai | openai-decisions | jev | off` (default `ai`), `harmonization.decider_api_key` (optional key for `openai-decisions`/`jev`, e.g. `"${JEV_API_KEY}"`; empty = `ai.api_key` when the provider is OpenAI, else `OPENAI_API_KEY` / `JEV_API_KEY`).
- New command `autorename-pdf setup` replaces hand-editing for first-time setup (idempotent; keys go to `.env`).
- Profile field ids are ASCII: a letter first, then letters, digits or `_` (Python keywords and pydantic names like `schema` are now allowed).
- Dates: missing day/month are no longer filled from today, and words such as `today` no longer parse. Incomplete dates render as `00000000`.

Before (3.x):

```yaml
config_version: 2
ai: {provider: openai, model: gpt-5.6-luna, api_key: "${OPENAI_API_KEY}"}
pdf: {extraction_mode: auto, ocr_fallback: paddleocr, max_pages: 3}
paddleocr: {venv_path: "", language: de, use_gpu: false, cpu_threads: 4}
company: {name: Example GmbH}
```

After (4.0):

```yaml
config_version: 4
ai: {provider: openai, api_key: "${OPENAI_API_KEY}"}   # model defaults per provider
pdf: {ocr: auto, vision: auto, max_pages: 3}   # the defaults; shown for clarity
ocr: {language: de, threads: 4}
company: {name: Example GmbH}
```

## Harmonization file (name lists)

- The company file moved: `harmonized-company-names.yaml` is no longer read. Move it to `name-lists/company_name.yaml` next to the config file (`autorename-pdf config path` prints the `name_lists` folder) and convert it to the structured format below; the flat `Name: [aliases]` list form is an error. Each harmonized field has its own list there (`name-lists/<field>.yaml`).
- `autorename-pdf companies ...` is now `autorename-pdf names ...` (same verbs: `list`, `review`, `merge`, `rename`, `add`; `--field <id>` picks a list other than the active profile's).
- Each entry is a mapping: `aliases` (hand-written), `learned` (added automatically), `source: user | auto`, `added`, `review`. Unknown entry keys and duplicate names are errors.
- The app rewrites the file when it learns names, which drops your comments. It writes a header comment.
- Matching: names are normalized (case, punctuation, legal forms such as GmbH/Inc/Ltd) and the canonical name itself also matches. 3.x compared only against the listed aliases.

Before (3.x):

```yaml
ACME:
  - ACME Corporation
  - Acme Corp
```

After (4.0):

```yaml
"ACME":
  aliases: ["ACME Corporation", "Acme Corp"]
  source: user
```

## CLI

- `autorename-pdf-cli.exe` is now `autorename-pdf` (`autorename-pdf.exe`). The subcommand is required: `autorename-pdf file.pdf` no longer means `rename`.
- `--text-only` is gone: use `--ocr=off --vision=off`. `config inspect` is gone: use `config show` and `config validate`.
- Exit codes 10 (provider) and 11 (auth) are now emitted when every file failed for that reason.
- `--ocr` / `--vision` take an optional value: `--ocr` = on, `--ocr=auto|on|off`.
- Running `rename` now also enriches the PDF metadata and learns company names by default; turn these off with `--no-enrich` / `--no-learn`.
- v1 config files no longer print a migration text; they fail validation with exit 3 like any other invalid config.

## Undo

- The undo journal moved from `.autorename-log.json` next to the config to `journal.json` in the OS data dir (`%LOCALAPPDATA%\autorename-pdf`, `~/.local/share/autorename-pdf`, `~/Library/Application Support/autorename-pdf`). Old logs are not read, so 3.x renames cannot be undone with 4.0.
- One batch now also reverts its metadata enrichment and the company names it learned.
- Undo output: `old_path` is the original name (restored), `new_path` the renamed path. In 3.x these were swapped.
- Undo never overwrites: if the original path is taken again, that entry fails.
- A corrupt journal is reported as an error instead of being silently reset.
