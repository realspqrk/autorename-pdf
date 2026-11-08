# Live POC (cloud + offline)

## TOC
- [Prereqs](#prereqs)
- [Cloud providers](#cloud-providers)
- [Offline (Ollama)](#offline-ollama)
- [Pass criteria](#pass-criteria)

## Prereqs
- `PATH=$HOME/.local/share/mise/shims:$PATH`; `cargo xtask fetch-natives` (PDFium).
- Keys in repo `.env` (gitignored): `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` (optional `GEMINI_API_KEY`). A 401 with `sk-your-…` = placeholder.
- Build CLI without GUI if gui/ is mid-change: `cargo build -p autorename-app --no-default-features --features ocr --release`.

## Cloud providers
```
AUTORENAME_LIVE=1 cargo test -p autorename-core --test providers_live -- --ignored --nocapture
```

## Offline (Ollama)
1. `curl -s localhost:11434/api/version || (nohup ollama serve >/dev/null 2>&1 &)`; `ollama pull gemma4:e2b` (CPU hosts) or `gemma4` (≥16 GB / GPU).
2. OCR models: `autorename-pdf models fetch latin` (must run WITHOUT offline config; offline blocks it with exit 3).
3. Scratch dir with copies of `tests/fixtures/{text_invoice_acme,image_invoice_springfield}.pdf`, `sha256sum in/*.pdf > before.sha`, config:
   ```yaml
   config_version: 4
   privacy: offline
   ai: {provider: ollama, model: gemma4:e2b, base_url: http://localhost:11434}
   ```
4. `autorename-pdf --config cfg/config.yaml -o text rename --jobs 1 in/` → `undo --all` → `sha256sum -c before.sha`.

## Pass criteria
- Both fixtures renamed (scan via auto OCR, no flags), `enriched · new company`, exit 0.
- `models fetch` under offline → exit 3. Undo restores byte-identical files.
- First Ollama call is slow (model load, ~90 s CPU); later calls ~10 s on CPU.
