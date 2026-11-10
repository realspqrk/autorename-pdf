# Python Behavior Contract (v3.2.0) — parity target for the Rust rewrite

Source of truth: `autorename-pdf.py`, `_config_loader.py`, `_profiles.py`, `_ai_processing.py`,
`_pdf_utils.py`, `_paddleocr_bridge.py`, `_document_processing.py`, `_utils.py` at commit `2ff1907`.
`autorename_pdf_runner.py` is only an import shim for tests (loads the hyphenated module as `autorename_pdf`).
Line refs are `file:line`. "Quirk" = behavior a clean-room port would likely get wrong.

Deliberate 4.0 divergences (beta.2, the rest of this file describes 3.x): `company.name` → `company.names` (list; prompt `{company}` = `"A", "B"`); no `Unknown` fallback for empty fields (the field and its separator are left out; an empty basename is still `Unknown.pdf`); a field value repeated at the start of the next field is left out; amounts are re-written with `output.decimal_separator` (default `.`) and no grouping; company names lose their legal form deterministically.

---

## 1. CLI

### 1.1 Global flags (`build_parser`, autorename-pdf.py:461)

| Flag | Type | Default | Notes |
|---|---|---|---|
| `--version` | action | — | prints `autorename-pdf 3.2.0` to stdout, exit 0 |
| `--output/-o` | `text`\|`json` | `None` → auto | accepted before OR after subcommand (sub-parsers re-declare with `default=SUPPRESS`) |
| `--quiet/-q` | bool | false | text: suppress all progress + summary; json: suppress stderr `Processing` lines (JSON still printed) |
| `--verbose/-v` | bool | false | console log handler DEBUG instead of WARNING |
| `--config PATH` | str | None | sets config path AND base dir = `dirname(abspath(PATH))` |
| `--undo` | bool | false | hidden, deprecated → runs undo handler (see 1.6) |
| `-h/--help` | — | — | argparse help, exit 0 |

Output format resolution (`resolve_output_format`, :162): explicit `-o` > frozen EXE ⇒ `text` > stdout not a TTY ⇒ `json` > `text`.

### 1.2 Subcommands

| Subcommand | Args / flags | Defaults |
|---|---|---|
| `rename` (default) | `paths*`, `--dry-run`, `--recursive/-r`, `--provider STR`, `--model STR`, `--vision`, `--text-only`, `--ocr`, `--profile ID` | paths=[] |
| `undo` | `directory?`, `--list` (dest list_batches), `--batch ID`, `--all` (dest undo_all) | directory=base dir |
| `config show` / `config validate` / `config inspect` | — | `config` w/o action ⇒ usage_error exit 2 |

`--provider` is NOT validated by argparse (any string; validated later by `_validate_config`). `--profile` overrides `profile:` for this run only (config not written).

### 1.3 Implicit `rename` injection (`_preprocess_argv`, :612)
Scan argv; skip `--output`,`-o`,`--config` plus their next token; skip any other token starting with `-`.
First positional: if in `{rename, undo, config}` → unchanged; else insert `rename` before it. No positional → unchanged.
- `file.pdf` → `rename file.pdf`; `-o json a.pdf b.pdf` → `-o json rename a.pdf b.pdf`; `--undo` → unchanged; `[]` → `[]`.
- Quirks: `--config=x` (equals form) is treated as a boolean flag (fine). `--provider openai x.pdf` (rename-only flag before positional) → `--provider rename openai x.pdf` → argparse error exit 2. A file literally named `undo`/`config`/`rename` is taken as subcommand.
- No subcommand after parsing (e.g. only `-v`, or empty argv) → `print_help()` to stdout, exit 2.
- argparse errors (bad `-o` choice, unknown flag) → usage on stderr, exit 2.

### 1.4 Rename flow (`_handle_rename`, :1014) — ORDER MATTERS
1. base_dir = `dirname(abspath(--config))` | dir of `sys.executable` (frozen) | dir of script. config_path = `--config` verbatim or `base_dir/config.yaml`. aliases = `base_dir/harmonized-company-names.yaml`.
2. Load config. v1 schema ⇒ prints migration text (1.8) to **stdout**, exit **1** (even with `-o json`; no JSON). `None` (missing/empty/YAML error/dup key/any load exception) ⇒ `config_error` "Could not load config.yaml. See config.yaml.example for setup." suggestion "Copy config.yaml.example to config.yaml and add your API key." exit 3.
3. Apply overrides in order: `--provider`→`ai.provider`; `--model`→`ai.model`; `--vision`→`pdf.vision=True`; `--text-only`→`pdf.ocr=False, pdf.vision=False`; `--ocr`→`pdf.ocr=True`. (So `--text-only --ocr` ⇒ ocr on, vision off; `--vision --text-only` ⇒ vision off.)
4. `_validate_config(config, path, --profile)`; invalid ⇒ `config_error`, message = first `error`-level issue message, no suggestion, exit 3. **Happens before path checks** (bad config + no paths ⇒ 3, not 2).
5. `select_profile` + `build_metadata_model`; ValueError ⇒ `config_error` exit 3.
6. No paths ⇒ `usage_error` "No files or folders specified." suggestion "Usage: autorename-pdf [options] <files_or_folders...>" exit 2.
7. Collect PDFs (1.5). None ⇒ `no_files` "No PDF files found in the specified paths." exit 4.
8. Non-dry-run: undo log = `base_dir/.autorename-log.json` (NOT the PDF's dir), batch_id = `YYYYMMDDTHHMMSS-<6 hex>` (UTC, `os.urandom(3)`). Dry-run: both null.
9. Per file: progress line, `process_pdf`, tally.
10. If `renamed==0 && skipped>0 && !dry_run` ⇒ append empty batch to undo log (even if some failed).
11. Emit result; exit: `0<failed<total` ⇒ 5; `failed==total` ⇒ 1; else 0.

### 1.5 PDF collection (`collect_pdf_files`, :280)
Each input: `strip('"')`, then `rstrip('\\')`, then `rstrip('/')`, then NFC. File ⇒ included iff `lower().endswith(".pdf")` (else log warning). Dir ⇒ non-recursive `os.listdir` (files only, `.pdf` case-insensitive), recursive `os.walk` (all depths). Missing path ⇒ log error, skipped. Order = input order, then OS listing order (unsorted!). Duplicates not removed. Paths are kept as given (relative stays relative).

### 1.6 Undo flow (`_handle_undo`, :943)
dir = positional `directory` or base_dir (from `--config`). log = `dir/.autorename-log.json`. Missing ⇒ `no_files` "No undo log found in {dir}" suggestion "Run a rename operation first, then undo." exit 4.
`--list` ⇒ batches JSON / text lines `  {batch_id}  {timestamp}  {n} file(s)  active|undone`, exit 0 (text with none: "No undo batches found.").
Else `undo_renames(batch=--batch, all=--all)`; exit 0 if failed==0 else 5. Text: `Undo complete: {s} restored, {f} failed`.
Legacy `--undo`: text ⇒ prints "Warning: --undo is deprecated. Use autorename-pdf undo instead." first; json ⇒ stderr `{"warning": "--undo is deprecated. Use 'autorename-pdf undo' instead."}` (compact JSON). Then same undo handler.

### 1.7 stdout / stderr contract
- All JSON on stdout: `json.dumps(indent=2, ensure_ascii=True)` ⇒ **non-ASCII escaped as `\uXXXX`** (e.g. `—` → `\u2014`, `ä` → `\u00e4`). Single JSON document, trailing newline.
- stdio reconfigured to UTF-8 with `errors="backslashreplace"`.
- JSON mode, not quiet: stderr gets `Processing [{i}/{total}] {basename_nfc}` per file, before processing (1-based).
- Python logging StreamHandler also writes to **stderr** (WARNING+ normally, DEBUG with `-v`), format `%(asctime)s - %(levelname)s - %(message)s` (asctime like `2026-11-07 10:11:12,345`). These interleave with Processing lines. Third-party loggers (httpx, httpcore, openai, anthropic, instructor, pdfminer, pdfplumber, PIL, urllib3, charset_normalizer) clamped to WARNING.
- Rotating log file: Windows `%LOCALAPPDATA%\autorename-pdf\autorename.log`, else `~/.local/share/autorename-pdf/autorename.log`; DEBUG; 5 MiB × 3 backups; utf-8. Failure to create is a warning only.

### 1.8 JSON shapes (verbatim key order)

BatchResult (rename):
```json
{
  "success": true, "total": 1, "renamed": 1, "skipped": 0, "failed": 0,
  "dry_run": false, "batch_id": "20261107T101112-a1b2c3",
  "files": [{
    "file": "C:/in/scan.pdf", "status": "renamed",
    "new_name": "20260906 ACME AP 12,13.pdf", "new_path": "C:/in/20260906 ACME AP 12,13.pdf",
    "error": null, "warnings": [], "company": "ACME", "date": "2026-09-06", "doc_type": "AP",
    "provider": "openai", "model": "gpt-5.6-luna", "profile": "business",
    "fields": {"document_date": "06.09.2026", "company_name": "ACME", "document_type": "AP", "description": "12,13"}
  }]
}
```
FileResult semantics (`process_pdf`, :322):
| key | value |
|---|---|
| file | `abspath(pdf)` with `\`→`/`, NFC |
| status | `renamed` (also in dry-run) \| `skipped` (target == current path) \| `failed` |
| new_name / new_path | only when renamed; new_path = abspath with `/`; includes `_(n)` suffix |
| error | `"No content extracted"` \| `"AI returned no metadata"` \| `str(exception)` |
| warnings | extraction warnings (4.6), set even on later failure |
| fields | **raw** model output (pre-harmonize, pre-sanitize); `{}` if failure before AI result |
| company | if `company_name` ∈ fields: harmonized value iff `harmonize_field=="company_name"` else raw; else null |
| date | `parse_document_date(fields.document_date).isoformat()` (`YYYY-MM-DD`) or null |
| doc_type | raw `document_type` if present (may be `""`), else null |
| provider/model/profile | effective values after CLI overrides |

ErrorResult: `{"success": false, "error_type": "...", "message": "...", "suggestion": null|"..."}`. error_types emitted: `config_error`, `usage_error`, `no_files`. (`general_error` default unused.)
UndoResult: `{"success", "restored", "failed", "files":[{"old_path","new_path","status","error":null}]}` — **no `batch_id` key** even with `--batch` (dataclass has it, `to_json` drops it). In files, `old_path` = the renamed path (current), `new_path` = original restored path, `/` separators. `error` always null. No batches/targets ⇒ `{"success":true,"restored":0,"failed":0,"files":[]}` exit 0.
UndoBatchListResult: `{"batches":[{"batch_id","timestamp","file_count","undone"}]}`.
Validate: `{"valid","issues":[{"field","level","message"}],"profile","resolved_profile"}`.
Inspect: `{"config_path","valid","issues","profile","resolved_profile","config"}` (see 2.6).

### 1.9 Exit codes (`_utils.ExitCode`)
| Code | Name | Emitted when |
|---|---|---|
| 0 | SUCCESS | all ok / help via `--help` / `--version` / undo with nothing to undo |
| 1 | GENERAL_ERROR | every file failed; **v1 config schema** on rename/show/validate |
| 2 | USAGE_ERROR | no paths; `config` w/o action; `config inspect` without `-o json`; no subcommand (help); argparse errors |
| 3 | CONFIG_ERROR | config missing/unloadable; validation error; profile error; validate/inspect invalid |
| 4 | NO_FILES | no PDFs collected; no undo log |
| 5 | PARTIAL_FAILURE | some (not all) files failed; undo with ≥1 failed restore |
| 10 | PROVIDER_ERROR | **never emitted** (defined, tested only as constant) |
| 11 | AUTH_ERROR | **never emitted** — auth/network errors become per-file `failed` ⇒ exit 1/5 |

### 1.10 Text mode (rich markup; cosmetic, keep similar)
Dry-run header `Dry run no files will be renamed`. Per file `[i/n] filename`, then steps `  ✓ Text extracted quality 0.92`, `  ✓ PaddleOCR`, `  ✓ Vision page images`, `  ⚠ <warning>`, `  ✓ AI <provider> / <model>`, `  → <new_name>` (`~` in dry-run), `  · Already named correctly`, `  ✗ <error>`. Summary `Done: 2 renamed, 1 skipped, 1 failed` (only non-zero parts) or `Done: All N file(s) already correctly named` when renamed==failed==0. Errors: message red + suggestion dim.

### 1.11 GUI sidecar usage (`gui/src/lib/sidecar.ts`)
- Binary `autorename-pdf-cli`, env `PYTHONUTF8=1 PYTHONIOENCODING=utf-8`; always appends `--config <resourceDir>/config.yaml` at the END of argv (after subcommand args).
- rename: `rename <paths…> --output json [--dry-run] [--recursive] [--provider X] [--model Y] [--vision] [--text-only] [--ocr] --config P`. Never `--profile`.
- undo: `undo --output json [--batch ID] --config P` (undo dir = resourceDir). inspect: `config inspect --output json --config P`. GUI is read-only on config.
- Parses stdout as JSON whenever non-empty (regardless of exit code). If exit≠0 AND stdout empty: code 3 or stderr∋"config" ⇒ config_error; code 11 or stderr∋"api key"/"auth" ⇒ auth_error. Every stderr chunk is forwarded as progress text (expects `Processing [i/n] name`; log lines also show).
- inspect must ALWAYS print JSON to stdout (GUI throws on empty stdout).

---

## 2. Config v2

### 2.1 Discovery & load (`load_yaml_config`, _config_loader.py:305)
- Path: `--config` or `<exe/script dir>/config.yaml`. No CWD/XDG/home search.
- Missing ⇒ None. `.env` loaded from `dirname(abspath(config))/.env` via `python-dotenv` `load_dotenv(path)` (override=False ⇒ **real OS env wins over .env**); only when config exists.
- YAML: SafeLoader single document; **duplicate mapping keys at any depth ⇒ YAMLError ⇒ None** (logged). Empty/falsy document ⇒ None. YAML 1.1 semantics (`yes/no/on/off` are bools!).
- v1 detection: top-level `openai` mapping containing `api_key` ⇒ ConfigSchemaError.
- Then migrations (2.3) on raw, deep-merge over DEFAULTS, env interpolation over merged tree, venv_path autodetect.
- Any other exception (e.g. `paddleocr: null` or `paddleocr: "x"` → TypeError/AttributeError) ⇒ None ("unloadable").

### 2.2 Keys, defaults, validation
| Key | Type | Default | Validation (`_validate_config`, autorename-pdf.py:685) |
|---|---|---|---|
| config_version | int | 2 | none |
| ai.provider | str | `openai` | error if falsy "AI provider not set"; error if ∉ {openai,anthropic,gemini,xai,ollama}: `Unknown AI provider 'x'. Expected one of: openai, anthropic, gemini, xai, ollama` |
| ai.model | str | `gpt-5.6-luna` | warning "No model specified, will use provider default" if falsy (only reachable by explicit `""`; NOT actually replaced by a default at call time) |
| ai.api_key | str | `""` | if provider≠ollama and falsy: error "API key required for non-ollama providers"; elif provider truthy ≠ollama and str contains `${`: error "API key still contains an unexpanded ${VAR} reference"; independently, `strip().lower()` ∈ {`your-api-key-here`,`your-api-key`}: warning "Placeholder API key; replace before calling a cloud provider" |
| ai.base_url | str | `""` | none; used by openai/gemini/xai/ollama; **ignored by anthropic** |
| ai.temperature | float | 0.0 | none; sent only for gemini/xai/ollama |
| ai.max_retries | int | 2 | none |
| pdf.max_pages | int | 3 | error unless int (not bool) ≥1: "pdf.max_pages must be an integer >= 1" |
| pdf.ocr / pdf.vision | bool\|"auto" | false | error unless bool or `"auto"`: `pdf.ocr must be true, false, or "auto"` |
| pdf.text_quality_threshold | float | 0.3 (4.0: 0.8, per-page readability) | none |
| pdf.incoming_invoice | str | `AP` | none (prompt placeholder) |
| pdf.outgoing_invoice | str | `AR` | none |
| paddleocr.venv_path | str | `""` → autodetect | none |
| paddleocr.language | str | `en` | none |
| paddleocr.device | str | `auto` | none (auto/cpu/gpu) |
| paddleocr.detection_model | str | `""` | none |
| paddleocr.det_limit_side_len | int | 736 | none |
| paddleocr.cpu_threads | int | 4 | none |
| company.name | str | `""` | warning if falsy or == `Your Company Name`: "Company name not configured. AI cannot distinguish incoming vs outgoing invoices." |
| output.language | str | `English` | none |
| output.date_format | strftime | `%Y%m%d` | none |
| prompt_extension | str | `""` | none |
| profile | str | `business` | via select_profile (field `profile`) |
| profiles | map | `{}` | via resolve_profiles (field `profile`) |
| filename (top-level) | — | — | presence ⇒ error "unsupported top-level filename; use profiles.<id>.template" |
- Sections `ai`/`company`/`pdf` present but not mappings (or null after merge — null treated as `{}`) ⇒ error field=`ai` "ai must be a mapping, not str"; when any such section error exists, profile resolution is skipped (profile/resolved_profile null).
- Unknown keys anywhere else: silently kept. Issue order = check order above. `valid` = no error-level issue.
- Merge (`_deep_merge`): dict+dict recurse; anything else replaces (null replaces a default dict).
- venv autodetect: if `paddleocr.venv_path` falsy ⇒ Windows `%LOCALAPPDATA%\autorename-pdf\paddleocr-venv` (skipped if LOCALAPPDATA unset), else `~/.local/share/autorename-pdf/paddleocr-venv`. **Written into config ⇒ visible in show/inspect.**

### 2.3 Legacy migrations (pre-merge, each logs a deprecation warning)
| Old | New |
|---|---|
| top-level `openai.api_key` (v1) | hard error (1.4 step 2 / inspect JSON exit 3) |
| `pdf.extraction_mode: vision_only` | setdefault ocr=false, vision=true |
| `pdf.extraction_mode: text_only` | ocr=false, vision=false |
| mode auto/unset + `pdf.ocr_fallback: paddleocr` | ocr="auto", vision=false |
| mode auto/unset + `ocr_fallback: vision` | ocr=false, vision="auto" |
| mode auto/unset + fallback none/unset | ocr=false, vision=false |
| `paddleocr.use_gpu: true/false` | setdefault device = `gpu` / `auto` |
| `paddleocr.languages: [a, …]` | setdefault language = first or `en` |
`setdefault` ⇒ explicit new keys win. Old keys are popped. Migration-instructions text (printed on v1) is verbatim at _config_loader.py:182.

### 2.4 `${VAR}` expansion
Regex `\$\{([^}]+)\}`; applied to every **string inside nested dicts** (not inside lists) after merge (so profile descriptions too). Unset var ⇒ left literal + warning `Environment variable ${X} is not set`. Quirk: an unexpanded `${X}` inside a profile description then fails profile placeholder parsing (`{X}` unknown) ⇒ config_error.

### 2.5 `config show`
Load (v1 ⇒ stdout text exit 1); None ⇒ `config_error` "Config file not found: {path}" suggestion "Copy config.yaml.example to config.yaml" exit 3 (also for YAML errors). Redact; select_profile (ValueError ⇒ config_error exit 3). Adds `profile` (selected id), `resolved_profile` (full resolved dict: intro, fields{id:{description}}, template, truncate_field, harmonize_field). JSON adds `config_path` (abs) last. Text: `yaml.dump(..., default_flow_style=False, allow_unicode=True)` (keys **sorted**) through rich. Exit 0. No validation.
Redaction (`_redact_config`): only `ai.api_key`, only if truthy: str len>8 ⇒ `key[:4]+"..."+key[-4:]`; else `***`. Applied to the env-expanded value.

### 2.6 `config validate` / `config inspect`
- validate: load (v1 ⇒ text exit 1); None ⇒ issue `{"field":"config_file","level":"error","message":"Config file not found: {path}"}` (also for parse errors). Prints validate JSON (`profile` id + `resolved_profile`); text "Configuration is valid." / "Configuration has errors:" then `  {level}: {field} — {message}`. Exit 0/3.
- inspect: requires json else usage_error exit 2. v1 ⇒ payload with config_file issue message `Config uses v1 schema; migrate to v2 (see config.yaml.example).`, exit 3. None ⇒ message `Config file unreadable: {path}` if file exists else `Config file not found: {path}`, exit 3. Error payload: `config_path` abs, `valid:false`, `profile:null`, `resolved_profile:null`, `config:null`. Success: `config` = redacted merged config (keeps user `profile` key, no `config_path`/`resolved_profile` inside), exit 0 valid / 3 invalid. Profile issue appears once (field `profile`).

---

## 3. Profiles (`_profiles.py`)

### 3.1 Built-ins (verbatim text at _profiles.py:16-108)
| id | fields (ordered) | template | truncate_field | harmonize_field |
|---|---|---|---|---|
| business | document_date, company_name, document_type, description | `{document_date} {company_name} {document_type} {description}` | description | company_name |
| academic | document_date, first_author_surname, journal_name, title | `{document_date} {first_author_surname} {journal_name} {title}` | title | null |
business.company_name description switches on `company.name` truthy (two variants, :16). business intro / academic intro at :33 / :79.

### 3.2 Declarations (`resolve_profiles`, :373)
Profile keys allowed: `intro, fields, template, truncate_field, harmonize_field, extends`. Field spec keys: `description` only (non-empty after strip, str). `fields.<id>: null` = delete.
1. Every declared id must be non-empty str with no surrounding whitespace ("invalid profile id").
2. Pass 1: overlays on built-ins (`business`, `academic`) — `extends` forbidden. Applied regardless of declaration order.
3. Pass 2: custom ids **in declaration order**; `extends` must name an already-resolved id (built-in, or custom declared earlier) — forward refs, self, cycles ⇒ "unknown parent". Without `extends`: must have both `template` and `truncate_field` ("standalone profile … is missing settings"); missing intro ⇒ `""`, harmonize ⇒ null; null field in standalone ⇒ error.
4. Overlay: scalars replace; existing field ids replaced in place (keep order); new ids appended; null deletes (error if missing; `document_date` cannot be deleted). Children inherit the parent's **post-overlay** state.
5. Validate each resolved: has `document_date`; template non-empty, no `/` or `\`, must not end with `.pdf` (case-insens.); placeholders ⊂ fields; truncate_field ∈ fields, ≠document_date, appears in template; harmonize_field null or ∈ fields and ≠document_date; intro str.
6. Interpolate intro + every description once (3.3). Unused profiles are still validated ⇒ any broken profile breaks every run.
Field-id rules: Python identifier, not keyword, not `_…`, not `model_…`, not an attribute of pydantic BaseModel (e.g. `schema`, `copy`, `json`, `dict`, `construct`, `validate`, `parse_obj`; `fields`, `title`, `date` are allowed).
Selection: `--profile` or `config.profile` (default business); must be non-empty, unstripped-equal str, existing (case-sensitive) else "unknown profile 'x'".

### 3.3 Placeholder syntax (prompt and template share the parser, :135)
`{{`→`{`, `}}`→`}`; `{name}` where name `isidentifier()`; anything else (`{}`, `{a.b}`, `{a!r}`, `{a:>3}`, lone `{`/`}`) ⇒ ValueError. Prompt placeholders allowed: `company` (company.name), `language` (output.language), `incoming_invoice`, `outgoing_invoice`; substituted values are literal (no re-expansion; braces in company name stay).

### 3.4 Dynamic schema (`build_metadata_model`, :421)
Pydantic model `DocumentMetadata`, `extra=forbid`, `strict=True`, every field required `str` with `description`. JSON schema emitted (business):
`{"additionalProperties":false,"properties":{"document_date":{"description":"…","title":"Document Date","type":"string"},…},"required":[all fields in order],"title":"DocumentMetadata","type":"object"}` — title = field id `_`→space, title-cased.

### 3.5 Filename rendering (`render_filename`, _document_processing.py:372)
Values (`_prepare_values`): for each profile field (missing/non-str ⇒ `""`):
- `document_date`: if non-blank and `parse_document_date` ok ⇒ `strftime(output.date_format)`; else `00000000` (date_format ignored).
- other: `sanitize_filename(NFC(raw))`; if blank ⇒ `Unknown` **only when profile_id == "business" literally** and field ∈ {company_name, document_type}; else `""`. (Custom `extends: business` gets no fallback; description never falls back.)
Token join (`_join_tokens`):
- Tokens = literals and fields. Empty field values are dropped.
- "Separator run" = non-empty literal of only whitespace (Unicode, incl. NBSP) and `-`/`_`.
- Before the first emitted piece, separator-run literals are discarded; a non-separator literal is emitted verbatim and starts output.
- After start, separator runs accumulate in `pending`; emitted only before the next non-empty field or non-separator literal; trailing pending dropped. Collapse of a pure run: contains `-` and any whitespace ⇒ ` - `; `-` only ⇒ `-`; contains `_` ⇒ `_`; else ` `.
- Then `sanitize_filename(NFC(base))`, then `" ".join(base.split())` (collapse all whitespace).
`sanitize_filename` (_utils.py:41): drop `[\x00-\x1f\x7f]`, drop `<>:"/\|?*`, then `strip('. ')` (ends only). Field values are sanitized individually AND whole base again.
Truncation: `_MAX_BASE = 244` **code points** (Python `len`). While `len(base)>244` and truncate value non-empty: drop last char of that (sanitized) value, re-join (every occurrence of the field shares the shortened value). Then if still >244: `base[:244].rstrip(". ")`. Empty ⇒ `Unknown`. Return NFC(`base + ".pdf"`). Only one truncation target.
Date parsing (`parse_document_date`): `dateparser.parse(s, settings={'DATE_ORDER':'DMY'})` with language autodetect — **big parity risk**. Observed: `06.09.2026`→2026-09-06; `2026-09-06`→2026-09-06; `09/06/2026`→2026-06-09; `15.03.24`→2024-03-15; `6 Sept. 2026`→2026-09-06; `15 March 2024` ok; `31.02.2026`→None; `not a date`/`""`/`1.234,56`→None; **`2026`→today's month/day in 2026; `March 2024`→2024-03-<today's day>; `today`→today** (relative words parse).
Examples (golden, tests/test_profile_filenames.py): `20260906 ACME AP 12,13.pdf`; business all empty ⇒ `00000000 Unknown Unknown.pdf`; academic all empty ⇒ `00000000.pdf`; company `<>:`, type `?*` ⇒ `20260906 Unknown Unknown.pdf`; title `Full title: subtitle` ⇒ `Full title subtitle`; `{{Paper}} {title}` ⇒ `{Paper} A Study.pdf`; `{a}_ -_{b}__{c}` with b empty ⇒ `Smith - A Study.pdf`; `_ -{title}- _` empty ⇒ `Unknown.pdf`.

---

## 4. Extraction pipeline (`extract_content`, _pdf_utils.py:353)

### 4.1 Text (tier 1)
- File opened as binary stream (non-ASCII path safety). `pdfplumber.open(stream, unicode_norm="NFC", raise_unicode_errors=False, repair=False)`; pages `0..min(max_pages, n)`; `page.extract_text()` defaults (x_tolerance=3, y_tolerance=3, non-layout). Per-page exception ⇒ warning, skip page. Non-blank pages emitted as `Page {i+1}:\n{text}`, joined `\n\n` (page numbers are original indices; blank pages skipped).
- Whole-open failure ⇒ retry with `repair=True` (pdfplumber uses Ghostscript for repair); second failure ⇒ `("", 0.0)`. Encrypted PDFs ⇒ `""`, 0.0.
- Mojibake fix (`_maybe_fix_mojibake`): markers `Ã`, `Â`, `â‚`, `â€œ`, `â€`, `â€“`, `â€”` (count = sum of `str.count` per marker; overlapping markers double-count). If >0: try `text.encode("cp1252").decode("utf-8")`, then `latin-1`; first that succeeds and has strictly fewer markers wins. Quirk: Python cp1252 is strict for undefined bytes 0x81/0x8D/0x8F/0x90/0x9D (encode fails ⇒ try latin-1); `encoding_rs` maps them, so emulate the strict table.
- Quality computed on the final text **including the `Page N:` prefixes**.
- **4.0 (not parity):** NFC + mojibake repair run per page, and the score is per page (4.2). The `Page N:` text sent to the AI is unchanged for clean PDFs (fixture golden text stays byte-identical).

### 4.2 Quality score (Python 3.x record, :113; **replaced in 4.0**)

**4.0 per-page readability score (intentional break, no Python parity):** per page, after NFC/mojibake repair and before `Page N:`: `N` = non-whitespace chars; blank or no letter/digit ⇒ `Q = 0`. `B` = U+FFFD, private-use and control chars except tab/CR/LF (counted before whitespace filtering); `M` = chars in residual multi-char mojibake sequences (`Â`/`Ã` + continuation, `â€`/`â‚…`, `ï»¿`, 4-byte `ð…`; a lone `Ã`/`Â` is legitimate). Each char counts once. `Q = clamp(1 − 10·(B+M)/N, 0, 1)`; short clean text = 1.0. This is the readability `R`. A page with a significant picture or drawing (image, curved/complex filled path, shading; tables and rectangles excluded; rebuild spec §Pipeline 1) scores at most **0.75** (`Qtext = min(R, 0.75)`), because content printed only as an image is invisible to `R`. After OCR the page scores its OCR text alone. The document summary (`quality` in plans, GUI) is the **minimum** page result. Alphabetic gibberish (wrong ToUnicode maps) is not detected. `pdf.text_quality_threshold` default **0.8** (3.x: 0.3), so pages with pictures are read again by default. 4.0 per-page readability `R` of the fixtures stays in `crates/core/tests/golden/quality.json`; with the cap, `mixed_invoice_initech` (logo) scores 0.75 and `image_invoice_springfield` 0. The 3.x formula below is kept as a record only:
```
if not text or not text.strip(): return 0.0
char_score  = min(len(text.strip()) / 500, 1.0) * 0.4
alnum_ratio = count(c.isalnum() for c in text) / count(not c.isspace()) (0 if denominator 0)
alnum_score = alnum_ratio * 0.3
words = text.split(); avg = sum(len(w))/len(words)
word_score = 0.3 if 3<=avg<=8 else 0.15 if 2<=avg<=12 else 0.05   (0.0 if no words)
return min(char_score + alnum_score + word_score, 1.0)
```
Unicode-aware `isalnum`/`isspace`/`split` (code points). 3.x fixture scores (max_pages 3): empty 0.0, image_invoice_springfield 0.0, minimal_text 0.562 (len 15), mixed_invoice_initech 0.814, multipage_invoice_stark 0.9682, text_invoice_acme 0.9167, text_letter_globex 0.886, text_outgoing_invoice_wayne 0.8237, text_rechnung_mustermann 0.7537. 4.0 scores: every text fixture 1.0 (minimal_text too), empty and image_invoice_springfield 0.0 (`crates/core/tests/golden/quality.json`).

### 4.3 OCR/vision decision (`_should_run_step`)
`True`/`"true"` ⇒ run; `False`/`"false"`/falsy ⇒ skip; anything else (i.e. `"auto"`) ⇒ `quality < text_quality_threshold` (strict). OCR and vision decided independently with the same quality.
**4.0:** same rule, **per page**: `auto` OCRs only pages with `Qtext < threshold`; each page's full OCR text is scored alone (never concatenated) and becomes the page result (no `max` with the text score, no cap); vision `auto` sends only pages whose result is still below the threshold (`on` = all selected pages). A failed or unavailable OCR keeps `Qtext` plus a warning. OCR'd pages whose text layer reads below the threshold add every OCR line to the evidence, others only the lines mostly missing from the text layer. Page numbers are original indices even when a render fails.

### 4.4 Rendering
If ocr or vision: pypdfium2 `page.render(scale=s)` → PIL; `s = 1.5` (108 DPI) if OCR only, else `2.0` (144 DPI). Pages `min(max_pages, n)`. Per-page render errors skipped. No images ⇒ warning `Could not render page images`, both disabled. Images are PIL RGB (pdfium default render, no alpha), encoded as **PNG** for both OCR and vision; no resizing/compression.

### 4.5 PaddleOCR bridge (tier 2a)
- Python: `<venv>/Scripts/python.exe` (win) or `<venv>/bin/python`; must exist else "not installed".
- Bridge script `_paddleocr_bridge.py` next to the app (frozen: copied from `_MEIPASS` to temp dir; PATH scrubbed of `_MEIPASS`; `_MEIPASS2`, `PYTHONPATH`, `PYTHONHOME` removed; `SetDllDirectoryW(NULL)`; `CREATE_NO_WINDOW`).
- argv: `python bridge <language> --device <device> --det-limit <n> --cpu-threads <n> [--det-model <m>]` (det-model only if non-empty).
- Protocol: write each image to `tmp/autorename_ocr_*/page_{i}.png`; send path + `\n` on stdin; read one stdout line JSON: `{"status":"ok","text":"<rec_texts joined \n>"}` or `{"status":"error","message":…}`. Empty line ⇒ skip page; JSON/pipe error ⇒ stop. No per-page timeout; after stdin close wait 30 s then kill. stderr lines containing downloading/fetching/download complete ⇒ WARNING logs, else INFO.
- OCR text: `Page {i+1}:\n{text}` per ok page (even when text empty!) joined `\n\n`. Quirk: an ok page with no text still makes `ocr_text` non-blank ⇒ `ocr` in sources, no warning.
- Bridge model mapping: det default `PP-OCRv6_small_det`; rec by lang: `PP-OCRv5_server_rec` {ch, chinese_cht, japan}; `PP-OCRv6_small_rec` {en + latin list at _paddleocr_bridge.py:53-60}; korean/th/el/te/ta/eslav(ru,be,uk)/arabic/cyrillic/devanagari `*_PP-OCRv5_mobile_rec`; unknown ⇒ v6 small rec + stderr warning. PaddleOCR kwargs: no doc orientation/unwarping/textline orientation, `enable_mkldnn=False`, `text_det_limit_side_len`, `text_det_limit_type="max"`, `cpu_threads`, `device` only if cpu/gpu; gpu init failure ⇒ retry without device. Env flags set at :15-28.
- The Rust port can keep this Python bridge unchanged (same protocol).

### 4.6 Warnings (verbatim, in FileResult.warnings)
`Could not render page images` · `PaddleOCR failed: {e}` · `PaddleOCR returned no text` (only if no warning yet) · `PaddleOCR not installed — run setup.ps1 to install` (em dash).
Sources: `["text"]` + `"ocr"` if ocr_text non-blank + `"vision"` if vision ran. Images kept only if vision ran.

### 4.7 Content gate
If `text.strip()`, `ocr_text.strip()` both empty and no images ⇒ failed "No content extracted" (AI not called).
Combined text (`_build_combined_text`): parts = [text if non-blank], then if ocr non-blank: (`"\n--- OCR Text ---\n"` if text part present), ocr_text; joined with `"\n"` ⇒ `text + "\n\n--- OCR Text ---\n\n" + ocr`.
Mode: text+images ⇒ multimodal; images only ⇒ vision; text only ⇒ text.

---

## 5. AI (`_ai_processing.py`)

### 5.1 Prompt (`build_system_prompt`, :48)
```
parts = ["Extract the requested fields from the document content. Due to OCR text detection, the text may be noisy and contain spelling and detection errors. Handle those as well as possible."]
+ [intro] if intro
+ ["\n\n".join(f"{name}: {description}" for fields in order)]
+ ["Return every requested field as a string. If a value is not found, return an empty string. Do not invent missing values or return undeclared fields."]
prompt = "\n\n".join(parts); if prompt_extension: prompt += "\n\n" + prompt_extension   (verbatim, not interpolated)
```
User text: text mode `Extract the information from this text:\n\n{text}`; vision `Extract document metadata from these page images:`; multimodal `Extract document metadata from this text and images:\n\n{text}`. Text block first, then images in page order. No truncation of text length.

### 5.2 Per-provider requests
| Provider | Call | Messages | Params | Schema | Retries |
|---|---|---|---|---|---|
| openai | `client.responses.parse` (Responses API) | `input=[{role:system,content:[{type:input_text,text:prompt}]},{role:user,content:[input_text…, {type:input_image,image_url:"data:image/png;base64,…"}…]}]` | `model`, `store=False`, `reasoning={"effort":"none"}` iff model lower contains `luna` or startswith `gpt-5.6`; no temperature, no max tokens, no `detail` | `text_format` = model ⇒ `text.format={type:json_schema,name:"DocumentMetadata",strict:true,schema}`; result `output_parsed` | SDK `max_retries=ai.max_retries`; base_url = ai.base_url or default |
| anthropic | `client.messages.parse` | `system=prompt` (string), `messages=[{role:user,content: str (text mode) | [{type:text},{type:image,source:{type:base64,media_type:image/png,data}}…]}]` | `model`, `max_tokens=1024`; no temperature | `output_format` = model (structured outputs `output_config.format` json_schema); result `parsed_output` | SDK `max_retries=ai.max_retries`; **base_url ignored** |
| gemini | instructor TOOLS over OpenAI chat.completions, base `https://generativelanguage.googleapis.com/v1beta/openai/` | `[{role:system,content:prompt},{role:user,content: str | [{type:text,text},{type:image_url,image_url:{url:data-uri}}…]}]` | `model`, `temperature=ai.temperature` | tool/function = model schema | instructor `max_retries=ai.max_retries` (re-asks on validation error); OpenAI client default 2 retries (not configurable) |
| xai | same, base `https://api.x.ai/v1` | same | same | TOOLS | same |
| ollama | same, base `http://localhost:11434/v1`, api_key `"ollama"` if empty | same | same | instructor **JSON mode** | same |
Custom `ai.base_url` overrides the provider base for openai/gemini/xai/ollama. Missing api key at call time ⇒ ValueError `API key required for provider 'x'. Set ai.api_key in config.yaml.` (normally pre-empted by validation).
Model default: only DEFAULTS `gpt-5.6-luna` (regardless of provider!). A config with `provider: anthropic` and no `model` key sends `gpt-5.6-luna` to Anthropic. Quirk.
Errors: `None` parsed output (refusal / no parse) ⇒ "AI returned no metadata"; any exception (auth 401, rate limit after retries, network, pydantic validation e.g. extra/missing field or non-string) ⇒ file failed with `str(e)` as error. No error classification, no exit 10/11. Strict model: a missing field or extra field fails validation.

---

## 6. Harmonization (`harmonize_company_name`, _document_processing.py:236)
- Only when profile `harmonize_field` set (business: `company_name`); input = raw value of that field (`""` if absent). Aliases file re-read **per file**: `base_dir/harmonized-company-names.yaml`.
- Format: YAML mapping `Canonical: [synonym, …]` (see `.example`). Missing/empty/invalid YAML ⇒ return `NFC(strip(input))`.
- Algorithm: `name = NFC(input.strip())`; for each canonical (file order): score = max over synonyms of `rapidfuzz.distance.JaroWinkler.similarity(name.lower(), syn.lower())` (prefix_weight 0.1, Unicode code points; Python `str.lower`). Best = first maximum. If score **> 0.85** (strict) ⇒ `NFC(canonical)`; else name.
- Canonical key itself is not a synonym unless listed. Synonyms are not NFC-normalized (only input is). Quirks: empty synonym list ⇒ `max()` ValueError ⇒ file fails; a non-string synonym (null/number) ⇒ AttributeError ⇒ file fails; a string instead of list iterates characters.
- Harmonized value is then sanitized/rendered; `FileResult.company` gets the harmonized unsanitized value; `fields` keeps raw.

---

## 7. Rename & undo

### 7.1 `rename_document` (:392)
- `pdf_path`, `new_name` NFC; append `.pdf` if missing (case-insens.). `new_path = join(dirname(pdf_path), new_name)` (relative stays relative).
- If `pdf_path == new_path` (exact string) ⇒ skipped (None). Quirk: case-only difference on case-insensitive FS ⇒ exists ⇒ `_(1)`.
- Collision: while `exists(new_path)`: `"{base}_({n}).pdf"`, n=1,2,…. No reservation across files in one batch (dry-run may report the same target twice; real run sees earlier renames).
- Dry-run: return path, no FS change, no log.
- Real: `os.rename` with retry on PermissionError: 3 attempts, 1.0 s sleep; final error message `Cannot rename: file is in use after 3 attempts. Close the file and try again: {src}`. TOCTOU: on POSIX `os.rename` overwrites silently; on Windows raises if dst exists.
- After success, append entry to undo log (if log path).

### 7.2 Undo log `.autorename-log.json` (base_dir; GUI: resourceDir)
```json
{"version": 2, "batches": [{"batch_id": "20261107T101112-a1b2c3", "timestamp": "2026-11-07T10:11:12.345678+00:00",
  "source": "cli", "undone": false,
  "files": [{"old_path": "C:\\in\\scan.pdf", "new_path": "C:\\in\\20260906 ACME AP 12,13.pdf", "timestamp": "2026-11-07T11:11:12.400000"}]}]}
```
- Written `indent=2, ensure_ascii=False`, utf-8, whole-file rewrite per renamed file (non-atomic). Key order as above (`batches` before `version` if loaded file had that order; new files get `{"version":2,"batches":…}` via read default; on write `version` is re-set).
- batch.timestamp = UTC aware ISO (`+00:00`, microseconds); entry.timestamp = **local naive** ISO. Paths stored as given (native separators, possibly relative, NFC).
- Batch created lazily on first renamed file; skipped/failed files not logged. Empty batch (`files: []`) written when renamed==0 and skipped>0.
- Pruning on every write: keep batch if timestamp empty or `timestamp >= (utcnow-30d).isoformat()` — **lexicographic string compare**.
- Read: missing/corrupt JSON ⇒ empty v2; v1 (bare list of entries) ⇒ one batch `migrated-v1`, timestamp of first entry, source cli, undone false.
- Legacy (no batch_id) call creates one batch per entry.

### 7.3 `undo_renames` (:557)
Targets: `--all` ⇒ all non-undone; `--batch ID` ⇒ that batch if non-undone (else nothing, exit 0); default ⇒ last non-undone in file order (may be an empty batch ⇒ no-op that consumes it). Per target, entries in **reverse** order: if `new_path` exists ⇒ rename back (same retry) ⇒ restored; else / on exception ⇒ failed. Batch marked `undone: true` regardless of failures; log never deleted; rewritten (with pruning). `--all` and `--batch` together ⇒ `--all` wins.

---

## 8. Quirks checklist (easy to miss)
1. JSON uses ASCII escapes (`ensure_ascii=True`); undo log does not.
2. Log lines go to stderr alongside `Processing [i/n]` lines.
3. v1 config ⇒ plain-text migration notice on stdout + exit 1 (except `config inspect` ⇒ JSON, exit 3).
4. Validation runs before "no paths" (exit 3 beats 2); CLI overrides applied before validation (`--provider bogus` ⇒ exit 3).
5. Exit 10/11 never produced; provider/auth failures ⇒ per-file failed ⇒ exit 1 (all) or 5.
6. 3.x: quality score includes `Page N:` headers; minimal 15-char PDF scores 0.562 (> 0.3 so `auto` does NOT trigger). 4.0: per-page readability, minimal_text = 1.0, default threshold 0.8.
7. OCR "ok" page with empty text still counts as OCR content.
8. Scale 1.5 for OCR-only, 2.0 otherwise; images always PNG.
9. `Unknown` fallback only for literal profile id `business`; date fallback `00000000` for all profiles; empty basename ⇒ `Unknown.pdf`.
10. 244-code-point base limit; collision suffix may push past (252 chars observed).
11. `dateparser` fills missing day/month from **today** and accepts relative words — port must replicate or explicitly diverge.
12. Harmonization threshold strictly `> 0.85`, lowercase compare, first max wins, YAML order.
13. Undo output swaps meaning: `old_path` = renamed file, `new_path` = restored original; no `batch_id` key.
14. Undo default dir = config dir, not PDF dir; GUI relies on resourceDir.
15. `ai.model` default is `gpt-5.6-luna` for every provider.
16. Anthropic ignores `ai.base_url`; temperature only for compat providers; OpenAI reasoning `none` only for luna / gpt-5.6*.
17. All profiles (even unused) are validated each run; custom profiles resolve in declaration order.
18. `${VAR}` only expanded in dict strings, not list items; `.env` never overrides OS env.
19. `config show` text is sorted YAML; JSON show/inspect include autodetected `paddleocr.venv_path`.
20. File collection order is OS listing order (unsorted); paths kept relative ⇒ undo log may hold relative paths (undo then depends on CWD).
21. `--text-only` then `--ocr` ⇒ OCR on (override order).
22. Collision check is not batch-aware in dry-run.
23. YAML 1.1 + duplicate-key rejection (dup ⇒ treated as "not found"/"unreadable").

---

## 9. Golden tests to port (file:test → behavior)
| Test | Behavior |
|---|---|
| test_profile_filenames.py:test_spec_filename_table (16 cases) | full render matrix incl. Unknown/00000000/academic/hyphen date format |
| test_profile_filenames.py:test_literal_and_value_safety (11) | separator collapse rules, `{{}}`, NBSP, sanitization, NFC |
| test_profile_filenames.py:test_truncates_only_explicit_title… / repeated_truncate / exhausted_fallback / hard_cut…strips_trailing_dot / no_second_truncation_target | 244 truncation algorithm |
| test_profile_filenames.py:test_date_behavior_is_reserved… | invalid date ⇒ 00000000 ignoring format; `:` stripped |
| test_profile_filenames.py:test_invoice_id_overlay… / test_profile_names_keep_collision… | overlay template; `_(1)` suffix, dry-run no log, skip, batch undo, name len 252 |
| test_profiles.py:test_builtin_ids_fields_and_templates | built-in shapes |
| test_profiles.py:test_overlay_before_inheritance… / test_standalone_nine_fields… / test_deleting_company… | overlay/extends/standalone semantics, ordering |
| test_profiles.py:test_invalid_profile_definitions (parametrized) / invalid_pydantic_field_ids / invalid_selected_ids / invalid_declared_ids / reject_*_placeholder_syntax | every rejection rule |
| test_profiles.py:test_interpolation_is_single_pass… / inherited_interpolation_happens_once… | placeholder interpolation |
| test_profile_pipeline.py (all 6) | FileResult fields/company/date/doc_type mapping, harmonize_field variants, failure/skip shapes |
| test_profile_ai.py:test_prompt_assembly_uses_resolved_schema_order… / generated_model_* | prompt text, strict schema |
| test_profile_cli.py (5) | exit 3 before any content/provider call; `--profile` override; show redaction + resolved_profile |
| test_profile_config.py | duplicate keys rejected; env expansion before profile interpolation |
| test_ai_processing.py:TestBuildProviderCreateKwargs / TestNativeStructuredExtract / TestNativeClientMaxRetries / TestGetInstructorClient / TestBuildCombinedText | request params, message shapes, base URLs, JSON vs TOOLS mode, OCR separator |
| test_cli.py:TestPreprocessArgv / TestResolveOutputFormat / TestRedactConfig / TestValidateConfig / TestHandleConfig{Show,Validate,Inspect} / TestHandleRename / TestHandleUndo / TestMain / TestGetBaseDirectory / TestErrorResult (ascii escapes) | CLI contract, JSON shapes, exit codes |
| test_config_loader.py (all) | merge, v1 detect, migrations (extraction_mode/ocr_fallback/use_gpu/languages), env vars, venv autodetect, company names loader |
| test_document_processing.py:TestHarmonizeCompanyName / TestParseDocumentDate / TestRenameInvoice / TestUndoRenames / TestUndoLogV2Schema / TestUndoLogMigration / TestAutoExpiry / TestRenameWithRetry / TestWriteEmptyBatch / TestFilenameLength | harmonize, dates, rename, undo log format, pruning, retry, empty batch |
| test_pdf_utils.py:TestShouldRunStep / TestAssessTextQuality / TestMojibakeRepair / TestExtractContent* / TestOCRConfigPassthrough / TestPaddleOcrV6Defaults / TestEncryptedPdfDetection | extraction decisions, scales, bridge argv, model mapping |
| test_extraction_fixtures.py (all) | real-fixture extraction (pages, max_pages, render counts, unicode filename) |
| test_end_to_end.py (all non-live) | mocked-AI full pipeline, harmonization, undo restore, base-dir log, mixed success exit 5, trailing-backslash paths |
| test_example_configs.py (all) | every examples/*.yaml + config.yaml.example validates and documented filenames render |
| test_utils.py | sanitize/valid filename constants, NFC |
Live-only (skip by default, `--run-live`): test_live_integration.py, test_live_profiles.py.

Fixture PDFs (`tests/fixtures/`, generated by `tests/generate_test_pdfs.py` via fpdf2): `text_invoice_acme.pdf` (EN incoming invoice), `text_rechnung_mustermann.pdf` (DE invoice), `text_outgoing_invoice_wayne.pdf` (outgoing/AR), `text_letter_globex.pdf` (non-invoice letter), `image_invoice_springfield.pdf` (scanned image, no text layer), `mixed_invoice_initech.pdf` (text + logo image), `multipage_invoice_stark.pdf` (3 pages), `minimal_text.pdf` (tiny text), `empty.pdf` (blank page). Sample config fixture: `tests/conftest.py:sample_config`.
