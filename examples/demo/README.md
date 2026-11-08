# Demo: seven sample PDFs

Invoices, a letter, a German *Rechnung*, a 3-page invoice and an image-only scan. The demo company (yours) is **Spqrk GmbH**: bills addressed to it are `AP`, the invoice it sent to Wayne Enterprises is `AR`.

| File | What it shows |
|---|---|
| `acme-invoice.pdf` | plain text invoice |
| `globex-letter.pdf` | letter (no total) |
| `initech-invoice.pdf` | text + logo image (page scores 0.75 → OCR, else vision) |
| `invoice-to-wayne.pdf` | outgoing invoice → `AR` |
| `rechnung-mustermann.pdf` | German invoice, German number format kept |
| `springfield-scan.pdf` | image-only scan → local OCR, else vision |
| `stark-invoice-3-pages.pdf` | multi-page |

All outputs below are real (OpenAI `gpt-6-luna`, recorded 2026-11-07, no OCR models installed). AI answers can vary slightly between runs; `AP`/`AR` and the amount format are derived by the app (from issuer/recipient and `total_amount` + `currency`), not left to the model.

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
  acme-invoice.pdf           →  20240315 ACME Corporation AP EUR 4,258.80.pdf
  globex-letter.pdf          →  20250122 Globex Corporation Letter.pdf
  initech-invoice.pdf        →  20240628 Initech Solutions AP EUR 237.50.pdf
                                ! ocr_models_missing: OCR models for pack 'latin' are not installed in .../models/latin Run `autorename-pdf models fetch latin` (~39 MB), or disable OCR (pdf.ocr: false).
  invoice-to-wayne.pdf       →  20250310 Wayne Enterprises AR EUR 10,700.00.pdf
  rechnung-mustermann.pdf    →  20250208 Mustermann Consulting AP EUR 1.320,00.pdf
  springfield-scan.pdf       →  20241103 Springfield Power Co AP USD 4,300.00.pdf
                                ! ocr_models_missing: OCR models for pack 'latin' are not installed in .../models/latin Run `autorename-pdf models fetch latin` (~39 MB), or disable OCR (pdf.ocr: false).
  stark-invoice-3-pages.pdf  →  20250301 Stark Industries AP EUR 17,100.00.pdf
Dry run: 7 to rename (nothing changed)
```

The scan has no text layer, and the Initech invoice has a logo: a page with a picture scores at most 0.75, below the default threshold 0.8, so both are read again. Without OCR models the app says so and sends those page images to the model instead (`pdf.vision: auto`). With `autorename-pdf models fetch` they are read locally first (`inspect demo/initech-invoice.pdf` shows `p1 0.75 pictures`).

## 4. Plan, apply, inspect, undo

```console
$ autorename-pdf rename --plan plan.json demo      # AI calls, no changes; review plan.json
                                                  # (to choose another name: edit items[].proposed_name)
Plan written to plan.json (run `autorename-pdf apply plan.json`).
...
$ autorename-pdf apply plan.json                   # no AI calls
  acme-invoice.pdf           →  20240315 ACME Corporation AP EUR 4,258.80.pdf  enriched · new company
  globex-letter.pdf          →  20250122 Globex Corporation Letter.pdf  enriched · new company
  initech-invoice.pdf        →  20240628 Initech Solutions AP EUR 237.50.pdf  enriched · new company
                                ! ocr_models_missing: ...
  invoice-to-wayne.pdf       →  20250310 Wayne Enterprises AR EUR 10,700.00.pdf  enriched · new company
  rechnung-mustermann.pdf    →  20250208 Mustermann Consulting AP EUR 1.320,00.pdf  enriched · new company
  springfield-scan.pdf       →  20241103 Springfield Power Co AP USD 4,300.00.pdf  enriched · new company
                                ! ocr_models_missing: ...
  stark-invoice-3-pages.pdf  →  20250301 Stark Industries AP EUR 17,100.00.pdf  enriched · new company
Done: 7 renamed, 7 enriched, 7 names learned  (revert: autorename-pdf undo)

$ autorename-pdf inspect "demo/20240315 ACME Corporation AP EUR 4,258.80.pdf"
.../demo/20240315 ACME Corporation AP EUR 4,258.80.pdf
  size        7774 bytes
  security    not signed, not encrypted
  enrich      eligible
  text        quality 1.00 (threshold 0.80), 653 chars, 1/1 pages
  pages       p1 1.00
  tier        text layer only
  embedded    yes (2026-11-07T15:54:11Z)
  fields      document_date=15.03.2024, company_name=ACME Corporation, issuer=ACME Corporation GmbH, recipient=Spqrk GmbH, document_type=AP, total_amount=4,258.80, currency=EUR, description=EUR 4,258.80
  images      not sent

$ autorename-pdf undo
  20250301 Stark Industries AP EUR 17,100.00.pdf    →  stark-invoice-3-pages.pdf
  20241103 Springfield Power Co AP USD 4,300.00.pdf  →  springfield-scan.pdf
  20250208 Mustermann Consulting AP EUR 1.320,00.pdf  →  rechnung-mustermann.pdf
  20250310 Wayne Enterprises AR EUR 10,700.00.pdf   →  invoice-to-wayne.pdf
  20240628 Initech Solutions AP EUR 237.50.pdf      →  initech-invoice.pdf
  20250122 Globex Corporation Letter.pdf            →  globex-letter.pdf
  20240315 ACME Corporation AP EUR 4,258.80.pdf     →  acme-invoice.pdf
Undo: 7 restored, 0 failed, 7 metadata reverted, 7 names forgotten  (batch 20261107T155411-b3e9a7)
```

After `undo` every file is byte-identical to the original, and the learned company names are gone again.

Agents: add `-o json` to any command for one JSON document (`autorename-pdf schema` has the schemas).

