# Demo: seven sample PDFs

Invoices, a letter, a German *Rechnung*, a 3-page invoice and an image-only scan. The demo company (yours) is **Spqrk GmbH**: bills addressed to it are `AP`, the invoice it sent to Wayne Enterprises is `AR`.

| File | What it shows |
|---|---|
| `acme-invoice.pdf` | plain text invoice |
| `globex-letter.pdf` | letter (no total) |
| `initech-invoice.pdf` | text + logo image (page scores 0.75 → OCR; without OCR models the text layer) |
| `invoice-to-wayne.pdf` | outgoing invoice → `AR` |
| `rechnung-mustermann.pdf` | German invoice: `1.320,00` is written as `1320.00` (`output.decimal_separator`) |
| `springfield-scan.pdf` | image-only scan → local OCR, else vision |
| `stark-invoice-3-pages.pdf` | multi-page |

All outputs below are real (OpenAI `gpt-6-luna`, recorded 2026-11-10, no OCR models installed). AI answers can vary slightly between runs; `AP`/`AR` and the amount format are derived by the app (from issuer/recipient and `total_amount` + `currency`), not left to the model.

## 1. Set up (once)

```console
$ export OPENAI_API_KEY=sk-...        # or pipe it: printf %s "$KEY" | autorename-pdf setup --api-key-stdin ...
$ autorename-pdf setup --provider openai --api-key-env OPENAI_API_KEY --company Spqrk
# ~/.config/autorename-pdf/config.yaml
  done     config        created from the starter config
  done     provider
  done     company
  done     api_key       stored as OPENAI_API_KEY in .env
  ok       validate
Ready. try: autorename-pdf rename --dry-run <folder>
```

Run it again: nothing changes (JSON `status: unchanged` for every step, shown as `ok` in text). `autorename-pdf setup --check` lists what is still missing and changes nothing; `setup -o json` marks optional steps with `required: false`.

## 2. Work on a copy

```console
$ cp -r examples/demo ./demo
```

These commands need no network and no API key:

```sh offline
autorename-pdf inspect demo/acme-invoice.pdf
autorename-pdf inspect demo/springfield-scan.pdf
autorename-pdf schema plan
autorename-pdf config path
```

```console
$ autorename-pdf inspect demo/acme-invoice.pdf
.../demo/acme-invoice.pdf
  size        1612 bytes
  security    not signed, not encrypted
  enrich      eligible
  text        quality 1.00 (threshold 0.80), 653 chars, 1/1 pages
  pages       p1 1.00
  tier        text layer only
  embedded    no
```

## 3. Preview (changes nothing)

```console
$ autorename-pdf rename --dry-run demo
  acme-invoice.pdf           →  20240315 ACME AP EUR 4258.80.pdf
  globex-letter.pdf          →  20250122 Globex Letter.pdf
  initech-invoice.pdf        →  20240628 Initech Solutions AP EUR 237.50.pdf
                                ! ocr_models_missing: OCR models for pack 'latin' are not installed in .../models/latin. Run `autorename-pdf models fetch latin` (~39 MB) to read scans and logos on this computer; until then scans use AI image reading when it is on.
  invoice-to-wayne.pdf       →  20250310 Wayne Enterprises AR EUR 10700.00.pdf
  rechnung-mustermann.pdf    →  20250208 Mustermann Consulting AP EUR 1320.00.pdf
  springfield-scan.pdf       →  20241103 Springfield Power AP USD 4300.00.pdf
                                page 1 sent to AI as an image
                                ! ocr_models_missing: OCR models for pack 'latin' are not installed in .../models/latin. Run `autorename-pdf models fetch latin` (~39 MB) to read scans and logos on this computer; until then scans use AI image reading when it is on.
  stark-invoice-3-pages.pdf  →  20250301 Stark Industries AP EUR 17100.00.pdf
Dry run: 7 to rename (nothing changed)
```

The scan has no text layer, and the Initech invoice has a logo: a page with a picture scores at most 0.75, below the default threshold 0.8, so both are read again by the local OCR. Without OCR models the app says so; the scan (unreadable) is sent to the model as an image (`pdf.vision: auto`), the Initech page keeps its clean text layer. With `autorename-pdf models fetch` both are read locally (`inspect demo/initech-invoice.pdf` then shows `p1 0.75 pictures ocr 1.00` and `tier OCR`). Company names lose their legal form (`ACME Corporation GmbH` → `ACME`).

## 4. Plan, apply, inspect, undo

```console
$ autorename-pdf rename --plan plan.json demo      # AI calls, no changes; review plan.json
                                                  # (to choose another name: edit items[].proposed_name)
Plan written to plan.json (run `autorename-pdf apply plan.json`).
...
$ autorename-pdf apply plan.json                   # no AI calls
  acme-invoice.pdf           →  20240315 ACME AP EUR 4258.80.pdf  enriched · new entry
  globex-letter.pdf          →  20250122 Globex Letter.pdf  enriched · new entry
  initech-invoice.pdf        →  20240628 Initech Solutions AP EUR 237.50.pdf  enriched · new entry
                                ! ocr_models_missing: ...
  invoice-to-wayne.pdf       →  20250310 Wayne Enterprises AR EUR 10700.00.pdf  enriched · new entry
  rechnung-mustermann.pdf    →  20250208 Mustermann Consulting AP EUR 1320.00.pdf  enriched · new entry
  springfield-scan.pdf       →  20241103 Springfield Power AP USD 4300.00.pdf  enriched · new entry
                                ! ocr_models_missing: ...
  stark-invoice-3-pages.pdf  →  20250301 Stark Industries AP EUR 17100.00.pdf  enriched · new entry
Done: 7 renamed, 7 enriched, 7 names learned  (revert: autorename-pdf undo)

$ autorename-pdf inspect "demo/20240315 ACME AP EUR 4258.80.pdf"
.../demo/20240315 ACME AP EUR 4258.80.pdf
  size        7700 bytes
  security    not signed, not encrypted
  enrich      eligible
  text        quality 1.00 (threshold 0.80), 653 chars, 1/1 pages
  pages       p1 1.00
  tier        text layer only
  embedded    yes (2026-11-10T09:27:05Z)
  fields      document_date=15.03.2024, company_name=ACME Corporation, issuer=ACME Corporation GmbH, recipient=Spqrk GmbH, document_type=AP, total_amount=4,258.80, currency=EUR, description=EUR 4258.80
  images      not sent

$ autorename-pdf undo
  20250301 Stark Industries AP EUR 17100.00.pdf     →  stark-invoice-3-pages.pdf
  20241103 Springfield Power AP USD 4300.00.pdf     →  springfield-scan.pdf
  20250208 Mustermann Consulting AP EUR 1320.00.pdf  →  rechnung-mustermann.pdf
  20250310 Wayne Enterprises AR EUR 10700.00.pdf    →  invoice-to-wayne.pdf
  20240628 Initech Solutions AP EUR 237.50.pdf      →  initech-invoice.pdf
  20250122 Globex Letter.pdf                        →  globex-letter.pdf
  20240315 ACME AP EUR 4258.80.pdf                  →  acme-invoice.pdf
Undo: 7 restored, 0 failed, 7 metadata reverted, 7 names forgotten  (batch 20261110T092705-2bc74b)
```

After `undo` every file is byte-identical to the original, and the learned company names are gone again.

Agents: add `-o json` to any command for one JSON document (`autorename-pdf schema` has the schemas).

