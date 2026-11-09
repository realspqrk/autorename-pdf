---
name: rename-pdfs
description: Rename PDF files from their content with AutoRename-PDF (dry-run plan, user review, apply). Uses the installed `autorename-pdf` binary, or `cargo run` in this repo. Any OS.
user-invocable: true
argument-hint: "[path_to_pdfs]"
allowed-tools:
  - Bash(autorename-pdf *)
  - Bash(cargo run -q -p autorename-app *)
  - Read
---

# Rename PDFs

## Pick the binary (`BIN`)

- Installed: `autorename-pdf` on PATH (Linux/macOS), or `autorename-pdf` (user PATH) or `"$LOCALAPPDATA/Programs/AutoRename-PDF/autorename-pdf.exe"` on Windows.
- In this repo (dev): `cargo run -q -p autorename-app --release --` (needs `cargo xtask fetch-natives` once).

Check: `BIN config validate -o json`. Exit 3 → help the user with `BIN config init` / `config path` (API key goes into `.env` beside the config as `OPENAI_API_KEY=...`; never print keys).

## Workflow (never skip the review)

1. Plan, no changes: `BIN rename --plan "$TMPDIR/arp-plan.json" -o json $ARGUMENTS` (add `-r` for subfolders).
2. Show the user a table from `items[]`: `source` → `proposed_name`, plus `warnings`, `error`, `company_match`/`learn` (new or review-flagged companies) and `enrich` (skipped PDFs).
3. Only after the user confirms: `BIN apply "$TMPDIR/arp-plan.json" -o json`. No new AI calls; files changed since the plan fail with `source_changed`.
4. Report renamed/failed counts, `enrich_status`, `learn_status`, and the batch id.

## Options (on `rename`)

`--profile business|academic|<id>` · `--provider openai|anthropic|gemini|xai|ollama` · `--model <id>` · `--ocr[=auto|on|off]` · `--vision[=auto|on|off]` (always with `=`) · `--no-enrich` · `--no-learn` · `--offline` (global: local Ollama only) · `-j N`.

## Undo and companies

- `BIN undo -o json` (last batch), `BIN undo --list -o json`, `BIN undo --batch <id>`.
- Wrong canonical company: `BIN names list -o json`, `names merge FROM INTO`, `names add NAME --alias X`, `names review --accept NAME` (add `--field <id>` for non-business profiles).

## Exit codes

0 ok · 2 usage · 3 config · 4 no PDFs · 5 partial · 10 provider · 11 auth (check the key). Details: `BIN --help`.
