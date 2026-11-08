//! clap definitions and help texts. Short `-h` stays concise; `--help` adds
//! side effects, network use, examples and (top level) agent guidance and
//! exit codes. Help text is pre-wrapped (no terminal-width wrapping) so
//! snapshots are stable.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

const ABOUT: &str = "Rename PDFs from their content (AI + OCR).";

const LONG_ABOUT: &str = "\
Rename PDFs from their content (AI + OCR).

autorename-pdf reads each PDF free-first (text layer, then offline OCR, then
page images for vision models, each only when the previous tier is poor),
asks one AI provider for structured fields (date, company, document type,
total, ...) and renames the file from a profile template, e.g.
`20240315 ACME AP 1.234,56.pdf`. By default it also embeds the extracted data
in the PDF and learns new company-name variants. Every change is journaled
and can be reverted with `undo`.";

const AFTER_HELP: &str = "\
Run `autorename-pdf --help` for the workflow, agent usage and exit codes.";

const AFTER_LONG_HELP: &str = "\
WORKFLOW:
  autorename-pdf setup --api-key-env OPENAI_API_KEY --company Spqrk
                                             # once: config + key (idempotent)
  autorename-pdf rename --dry-run ~/Scans    # preview; changes nothing
  autorename-pdf rename ~/Scans              # rename + enrich + learn
  autorename-pdf undo                        # revert the last run

FOR AGENTS:
  - Setting the app up for a user: `setup --check -o json`, then `setup ...`
    (see `setup --help`; never put API keys on the command line).
  - Always pass `--output json` (also the default when stdout is not a TTY).
  - Two steps: `rename --plan plan.json <paths>` (AI calls, no changes),
    review plan.json, then `apply plan.json` (no AI calls; refuses files that
    changed since the plan was made).
  - Never prompts. stdout carries only the result document; diagnostics and
    progress go to stderr. `--output ndjson` streams progress events on stdout
    (one JSON object per line, the last line is the result document).
  - Errors: {\"schema_version\":4,\"error\":{\"code\",\"message\",\"hint\",\"retryable\"}}
    with stable codes; per-file errors use the same shape in `files[].error`
    (plans: `items[].error`).
  - JSON Schemas of every document: `autorename-pdf schema`.

NETWORK:
  `rename` and `gui` send extracted text (and page images when vision runs)
  to the configured AI provider. With --offline (or `privacy: offline`) only a
  local or LAN Ollama server is allowed; anything else is an error (exit 3);
  proxy variables are ignored and redirects never followed. `models fetch`,
  `setup --ocr-models` (and the GUI's download button) fetch OCR models;
  `setup` also asks a local Ollama which models it has. No other command uses
  the network.

FILES:
  `autorename-pdf config path` prints every location: config.yaml (+ .env
  beside it), the name-lists folder, the undo journal, OCR models.

EXIT CODES:
  0   success
  1   error (or every file failed for mixed reasons)
  2   usage error (bad arguments, unreadable plan file)
  3   config error (missing or invalid config, offline-mode violation)
  4   no PDF files found
  5   partial failure: some files failed
  10  provider error: every file failed at the AI provider
  11  auth error: every file failed authentication (check ai.api_key)";

#[derive(Debug, Parser)]
#[command(
    name = "autorename-pdf",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_help = AFTER_HELP,
    after_long_help = AFTER_LONG_HELP,
    arg_required_else_help = true,
    disable_help_subcommand = true,
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable, aligned, colored on a terminal
    Text,
    /// One JSON document on stdout
    Json,
    /// Progress events, one JSON object per line; the last line is the result
    Ndjson,
}

#[derive(Debug, Args)]
#[command(next_help_heading = "Global options")]
pub struct Global {
    /// Config file [default: $AUTORENAME_PDF_CONFIG, else the OS config dir]
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Output format [default: text on a terminal, json otherwise]
    #[arg(long, short = 'o', global = true, value_enum, value_name = "FORMAT")]
    pub output: Option<OutputFormat>,
    /// No progress or notices on stderr (errors are still reported)
    #[arg(long, short, global = true)]
    pub quiet: bool,
    /// More detail on stderr
    #[arg(long, short, global = true)]
    pub verbose: bool,
    /// Privacy mode: local/LAN Ollama only, any cloud request is an error
    #[arg(long, global = true)]
    pub offline: bool,
    /// Disable colors (also: NO_COLOR, or output that is not a terminal)
    #[arg(long, global = true)]
    pub no_color: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Rename(RenameArgs),
    Apply(ApplyArgs),
    Undo(UndoArgs),
    Inspect(InspectArgs),
    Names(NamesArgs),
    #[command(subcommand)]
    Config(ConfigCmd),
    #[command(subcommand)]
    Models(ModelsCmd),
    Schema(SchemaArgs),
    #[command(subcommand)]
    Integrate(IntegrateCmd),
    Setup(SetupArgs),
    Gui(GuiArgs),
}

// ---------------------------------------------------------------------------
// rename / apply
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Only pages scoring below pdf.text_quality_threshold
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProviderArg {
    Openai,
    Anthropic,
    Gemini,
    Xai,
    Ollama,
}

impl ProviderArg {
    pub fn id(self) -> &'static str {
        match self {
            ProviderArg::Openai => "openai",
            ProviderArg::Anthropic => "anthropic",
            ProviderArg::Gemini => "gemini",
            ProviderArg::Xai => "xai",
            ProviderArg::Ollama => "ollama",
        }
    }
}

const RENAME_LONG: &str = "\
Extract fields from each PDF with AI, then rename it (and enrich and learn).

Per file: text layer -> offline OCR (--ocr) -> page images (--vision) -> one
structured AI call -> name matching -> filename from the profile
template -> `_(n)` suffix on collisions. Existing files are never overwritten.

SIDE EFFECTS (unless --dry-run or --plan):
  - Renames each file in its folder.
  - Enrichment: appends metadata (Title/Subject/Keywords, XMP) and an
    `autorename-pdf.json` attachment as an incremental update; the original
    bytes stay intact. Skipped for signed, certified, encrypted, PDF/A and
    e-invoice files. Off with --no-enrich.
  - Learning: adds new name variants (e.g. companies) to the profile's name
    list, name-lists/<field>.yaml. Off with --no-learn.
  - Records everything as one batch in the undo journal (`undo` reverts it).

With --dry-run or --plan FILE nothing changes: the plan is printed (and
written to FILE); `apply FILE` executes it later without new AI calls.

NETWORK:
  One AI request per file, plus a small one per ambiguous name
  (harmonization.decider: ai). Local Ollama only with --offline.

OUTPUT:
  text: `old.pdf -> new.pdf` per file and a summary.
  json: the result (`schema result`); with --dry-run/--plan the plan
  (`schema plan`). Each file has rename_status, enrich_status, learn_status.";

const RENAME_EXAMPLES: &str = "\
EXAMPLES:
  autorename-pdf rename invoice.pdf
  autorename-pdf rename --dry-run -r ~/Documents/Inbox
  autorename-pdf rename ~/Scans --ocr=auto --vision=auto
  autorename-pdf rename *.pdf --provider anthropic --model claude-haiku-5-5
  autorename-pdf rename ~/Scans --plan plan.json -o json   # then: apply plan.json
  autorename-pdf --offline rename ~/Private                # local Ollama only";

/// Extract, plan and rename PDFs (+ enrich, + learn)
#[derive(Debug, Args)]
#[command(long_about = RENAME_LONG, after_long_help = RENAME_EXAMPLES)]
pub struct RenameArgs {
    /// PDF files and/or folders
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<PathBuf>,
    /// Show the plan; change nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Write the plan to FILE for `apply`; change nothing
    #[arg(long, value_name = "FILE")]
    pub plan: Option<PathBuf>,
    /// Overwrite an existing --plan FILE (never an input PDF)
    #[arg(long, requires = "plan")]
    pub force: bool,
    /// Include subfolders of folder arguments
    #[arg(long, short)]
    pub recursive: bool,
    /// Naming profile for this run (business, academic, or a custom id)
    #[arg(long, value_name = "ID")]
    pub profile: Option<String>,
    /// AI provider for this run
    #[arg(long, value_enum, value_name = "NAME")]
    pub provider: Option<ProviderArg>,
    /// Model id for this run [default: per-provider default from the config]
    #[arg(long, value_name = "ID")]
    pub model: Option<String>,
    /// Offline OCR (`--ocr` = on; default auto = poor text layer); needs `models fetch`
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "on"
    )]
    pub ocr: Option<Mode>,
    /// Page images to the model (`--vision` = on; default auto = text + OCR still poor)
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "on"
    )]
    pub vision: Option<Mode>,
    /// Do not embed metadata into the PDFs
    #[arg(long)]
    pub no_enrich: bool,
    /// Do not add new names to the name list (name-lists/<field>.yaml)
    #[arg(long)]
    pub no_learn: bool,
    /// Files processed in parallel [default: min(4, CPU cores)]
    #[arg(long, short = 'j', value_name = "N", value_parser = clap::value_parser!(u16).range(1..=64))]
    pub jobs: Option<u16>,
}

const APPLY_LONG: &str = "\
Execute a plan written by `rename --plan FILE` (or `rename --dry-run -o json`).

No AI calls. Each source must still have the SHA-256 recorded in the plan,
otherwise that file fails with `source_changed`. Targets that appeared since
fail with `conflict`; nothing is overwritten.

EDITING A PLAN:
  Change `items[].proposed_name` (a file name ending in .pdf), then apply.
  The file is renamed inside its own folder to that name; `target` is only
  informational and ignored. Invalid names (path separators, other
  extensions, reserved names) stop the whole apply before any change (exit 2).
  Edit with any editor; UTF-8 (with or without BOM) and UTF-16 are read
  (PowerShell: `Set-Content -Encoding utf8`). A `rename --dry-run -o json`
  document is the same plan format and can be applied too.

SIDE EFFECTS:
  Same as `rename`: renames, enrichment (unless --no-enrich), learned names
  (unless --no-learn), one undo batch.

NETWORK:
  None.";

const APPLY_EXAMPLES: &str = "\
EXAMPLES:
  autorename-pdf rename ~/Scans --plan plan.json -o json
  autorename-pdf apply plan.json -o json
  autorename-pdf rename ~/Scans --dry-run -o json | autorename-pdf apply -";

/// Execute a saved plan (no AI calls)
#[derive(Debug, Args)]
#[command(long_about = APPLY_LONG, after_long_help = APPLY_EXAMPLES)]
pub struct ApplyArgs {
    /// Plan file (`-` = stdin)
    #[arg(value_name = "PLAN")]
    pub plan: PathBuf,
    /// Do not embed metadata into the PDFs
    #[arg(long)]
    pub no_enrich: bool,
    /// Do not add new names to the name list (name-lists/<field>.yaml)
    #[arg(long)]
    pub no_learn: bool,
}

// ---------------------------------------------------------------------------
// undo / inspect
// ---------------------------------------------------------------------------

const UNDO_LONG: &str = "\
Revert a batch: truncate enrichment back to the original bytes, rename files
back, remove names learned by that batch.

Default: the most recent batch that is not undone yet. Undo never overwrites:
if the original name is taken again, or the file changed after enrichment,
that entry fails (exit 5) and the rest is still reverted.

ORDER (several runs touched the same file):
  Changes are undone newest first. If a newer change to a file cannot be
  undone (e.g. edited since), every older change to that file is skipped as
  `blocked_by_failed_newer` (reported, kept for a retry), never run on the
  wrong bytes. `undo --batch X` is refused up front (exit 1, error code
  `undo_blocked`, nothing changed) while a newer batch still touches the same
  files: undo that one first, or `undo --all`.

SIDE EFFECTS:
  Renames files, truncates enriched PDFs, edits the name lists the batch
  learned into, marks the batch undone in the journal.

NETWORK:
  None.";

const UNDO_EXAMPLES: &str = "\
EXAMPLES:
  autorename-pdf undo
  autorename-pdf undo --list -o json
  autorename-pdf undo --batch 20261107T101112-a1b2c3";

/// Revert renames, enrichment and learned names of a batch
#[derive(Debug, Args)]
#[command(long_about = UNDO_LONG, after_long_help = UNDO_EXAMPLES)]
pub struct UndoArgs {
    /// List batches instead of undoing
    #[arg(long, conflicts_with_all = ["batch", "all"])]
    pub list: bool,
    /// Undo this batch id (see --list)
    #[arg(long, value_name = "ID", conflicts_with = "all")]
    pub batch: Option<String>,
    /// Undo every batch that is not undone yet
    #[arg(long)]
    pub all: bool,
}

const INSPECT_LONG: &str = "\
Show what autorename-pdf sees in a PDF, without AI calls or changes: embedded
AutoRename data (attachment + XMP), signature/encryption status (decides
whether enrichment is possible), text-layer quality and whether OCR/vision
would run with the current settings.

Works without a valid config (defaults are used for the thresholds).

NETWORK:
  None.";

const INSPECT_EXAMPLES: &str = "\
EXAMPLES:
  autorename-pdf inspect invoice.pdf
  autorename-pdf inspect -o json *.pdf  # wildcards work in cmd/PowerShell too";

/// Show embedded data, signature/encryption and text quality
#[derive(Debug, Args)]
#[command(long_about = INSPECT_LONG, after_long_help = INSPECT_EXAMPLES)]
pub struct InspectArgs {
    /// PDF files
    #[arg(required = true, value_name = "PDF")]
    pub paths: Vec<PathBuf>,
}

// ---------------------------------------------------------------------------
// names
// ---------------------------------------------------------------------------

/// Maintain the name lists (Name matching: one spelling per name)
#[derive(Debug, Args)]
#[command(long_about = "\
Maintain the name lists used by Name matching: per harmonized field one file
name-lists/<field>.yaml beside config.yaml (e.g. name-lists/company_name.yaml)
with canonical names, hand-written `aliases` and auto-`learned` variants.

--field picks the list [default: the active profile's harmonize_field, e.g.
company_name for business; academic has none, so pass --field there].
Edits rewrite the file atomically (comments are not preserved). Manual edits
are not journaled. No network use.

LIFECYCLE:
  1. A run learns names automatically (`source: auto`): a close variant of a
     known name becomes a learned alias; a new name a new entry.
  2. An ambiguous name with no decider verdict becomes a new entry flagged
     `review` with its evidence (candidates, scores, source PDF, batch).
  3. You decide: `names merge NEW KNOWN` (it was the same) or
     `names review --accept NEW` (it is new). Either makes the entry yours
     (`source: user`); `names rename OLD NEW` fixes a spelling.
  4. `undo` of a run removes what that run learned, wherever a learned alias
     now lives (also after merge/rename), but never an entry you accepted,
     merged or renamed (reported as `kept`).")]
pub struct NamesArgs {
    /// Name list: a profile field id [default: the profile's harmonize_field]
    #[arg(long, global = true, value_name = "ID")]
    pub field: Option<String>,
    #[command(subcommand)]
    pub cmd: NamesCmd,
}

#[derive(Debug, Subcommand)]
pub enum NamesCmd {
    /// List entries
    List,
    /// List auto-learned entries flagged for review; accept them
    #[command(after_long_help = "\
EXAMPLES:
  autorename-pdf names review
  autorename-pdf names review --accept \"ACME Corp\"
  autorename-pdf names --field journal_name review --accept-all")]
    Review {
        /// Clear the review flag of this entry (repeatable)
        #[arg(long, value_name = "NAME")]
        accept: Vec<String>,
        /// Clear every review flag
        #[arg(long, conflicts_with = "accept")]
        accept_all: bool,
    },
    /// Change an entry's canonical spelling (OLD stays as an alias)
    #[command(after_long_help = "\
EXAMPLES:
  autorename-pdf names rename \"ACME Corporation\" ACME

Refuses when NEW already belongs to another entry: merge instead.")]
    Rename {
        #[arg(value_name = "OLD")]
        old: String,
        #[arg(value_name = "NEW")]
        new: String,
    },
    /// Merge entry FROM into INTO (FROM's names become aliases)
    #[command(after_long_help = "\
EXAMPLES:
  autorename-pdf names merge \"Acme Corp\" ACME
  autorename-pdf names --field journal_name merge \"Nat. Phys.\" \"Nature Physics\"")]
    Merge {
        #[arg(value_name = "FROM")]
        from: String,
        #[arg(value_name = "INTO")]
        into: String,
    },
    /// Add an entry or aliases to an existing entry
    #[command(after_long_help = "\
EXAMPLES:
  autorename-pdf names add ACME --alias \"ACME Corporation\" --alias \"Acme Corp\"")]
    Add {
        /// Canonical name (used in filenames)
        #[arg(value_name = "NAME")]
        name: String,
        /// Variant that maps to NAME (repeatable)
        #[arg(long, value_name = "ALIAS")]
        alias: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

/// Show, validate, locate or create the config
#[derive(Debug, Subcommand)]
#[command(long_about = "\
Show, validate, locate or create config.yaml.

Discovery: --config > $AUTORENAME_PDF_CONFIG > OS config dir
(~/.config/autorename-pdf, %APPDATA%\\autorename-pdf,
~/Library/Application Support/autorename-pdf). `${VAR}` values are read from
the environment, then from a .env file beside the config. No network use.")]
pub enum ConfigCmd {
    /// Print the effective config (defaults merged, API key redacted)
    Show,
    /// Validate; exit 3 when invalid
    Validate,
    /// Print config, .env, name lists, journal and models locations
    Path,
    /// Write a commented starter config (refuses to overwrite without --force)
    #[command(after_long_help = "\
SIDE EFFECTS:
  Creates the config file (and its folder). Nothing else.

EXAMPLES:
  autorename-pdf config init
  autorename-pdf --config ./config.yaml config init --force")]
    Init {
        /// Overwrite an existing file
        #[arg(long)]
        force: bool,
    },
}

// ---------------------------------------------------------------------------
// models
// ---------------------------------------------------------------------------

/// Manage offline OCR model packs
#[derive(Debug, Subcommand)]
#[command(long_about = "\
Manage offline OCR model packs (PP-OCRv6, hash-pinned). OCR never downloads
models implicitly; without them `--ocr` fails with `ocr_models_missing`.

Location: $AUTORENAME_MODELS_DIR, else <OS data dir>/autorename-pdf/models.")]
pub enum ModelsCmd {
    /// List packs and their install state
    List,
    /// Download a pack (network: GitHub releases; no document data is sent)
    #[command(after_long_help = "\
EXAMPLES:
  autorename-pdf models fetch
  autorename-pdf models fetch latin")]
    Fetch {
        /// Pack id [default: the pack for ocr.language]
        #[arg(value_name = "PACK")]
        pack: Option<String>,
    },
    /// Install a pack from a local folder (offline machines; hash-verified)
    Import {
        /// The pack folder (e.g. `.../models/latin`) or its parent `models`
        /// folder, copied from a machine that ran `models fetch`
        #[arg(value_name = "DIR")]
        dir: PathBuf,
        /// Pack id [default: the pack for ocr.language]
        #[arg(long, value_name = "PACK")]
        pack: Option<String>,
    },
    /// Delete an installed pack
    Remove {
        #[arg(value_name = "PACK")]
        pack: String,
    },
}

// ---------------------------------------------------------------------------
// schema / gui
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SchemaKind {
    /// Output of `rename` / `apply`
    Result,
    /// Output of `rename --dry-run` / `--plan`, input of `apply`
    Plan,
    /// config.yaml
    Config,
    /// Error document
    Error,
    /// NDJSON progress event
    Event,
    /// Output of `inspect`
    Inspect,
    /// Output of `setup`
    Setup,
    /// Output of `integrate install|remove|status`
    Integrate,
    /// Output of `undo`
    Undo,
}

/// Print JSON Schemas of the machine-readable documents
#[derive(Debug, Args)]
#[command(after_long_help = "\
EXAMPLES:
  autorename-pdf schema            # all schemas, keyed by name
  autorename-pdf schema plan")]
pub struct SchemaArgs {
    /// Which schema [default: all]
    #[arg(value_enum, value_name = "KIND")]
    pub kind: Option<SchemaKind>,
}

const GUI_LONG: &str = "\
Open the GUI, with PATHs pre-loaded into the preview.

With --rename-now (the file-manager \"Rename now\" entry) the files are
analyzed and renamed right away when nothing needs a look: no errors, no
company flagged for review. Otherwise the window stays in the preview so
you can check the flagged rows first. Either way the result screen offers
Undo. Config (enrich, learn, provider) applies as for `rename`.";

/// Open the GUI (preview pre-loaded with PATHs)
#[derive(Debug, Args)]
#[command(long_about = GUI_LONG)]
pub struct GuiArgs {
    /// Files or folders to preview
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,
    /// Rename right after the analysis unless a file needs review
    #[arg(long)]
    pub rename_now: bool,
    /// Also open the paths listed in FILE (one per line, UTF-8), then delete
    /// FILE (Windows launcher hand-off; only `req-*.txt` files)
    #[arg(long, value_name = "FILE")]
    pub request_file: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// setup
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DeciderArg {
    /// Small structured call on the main provider (default)
    Ai,
    OpenaiDecisions,
    Jev,
    /// Never ask: ambiguous names become `review: true` entries
    Off,
}

impl DeciderArg {
    pub fn id(self) -> &'static str {
        match self {
            DeciderArg::Ai => "ai",
            DeciderArg::OpenaiDecisions => "openai-decisions",
            DeciderArg::Jev => "jev",
            DeciderArg::Off => "off",
        }
    }
}

const SETUP_LONG: &str = "\
Set up AutoRename-PDF in one non-interactive, idempotent command.

Creates config.yaml (with comments) if missing, then edits only the keys you
pass, keeping comments and everything else. API keys go to the .env file
beside config.yaml (owner-only, mode 0600); config.yaml only references them
as `${VAR}`. Keys are never printed. Running the same command again changes
nothing (every step `unchanged`).

Steps (JSON `steps[]`: done | unchanged | failed | missing):
  config, privacy, provider, base_url, model, company, profile, offline,
  api_key, decider, decider_key, validate, ocr_models, integrate.

Changing the provider resets ai.base_url to the new provider's default
endpoint unless --base-url (or --ollama-url) is given in the same run. Saved
offline mode stays until --cloud: a cloud --provider alone is refused.

With --check nothing changes: it reports what is missing (exit 0 when ready
to rename, 3 otherwise). Network: only --ocr-models downloads (refused
offline).";

const SETUP_AFTER: &str = "\
EXAMPLES:
  # Cloud (OpenAI), key already in the environment (bash):
  export OPENAI_API_KEY=sk-...
  autorename-pdf setup --provider openai --api-key-env OPENAI_API_KEY \\
      --company Spqrk

  # The same in Windows PowerShell:
  $env:OPENAI_API_KEY = \"sk-...\"
  autorename-pdf setup --provider openai --api-key-env OPENAI_API_KEY `
      --company Spqrk

  # Key held by an agent (never on the command line):
  printf %s \"$KEY\" | autorename-pdf setup --provider anthropic \\
      --api-key-stdin --company Spqrk

  # Fully offline with a local Ollama (fetch OCR models first, online;
  # CPU-only machines: gemma4:e2b):
  autorename-pdf setup --ocr-models
  autorename-pdf setup --offline --ollama-model gemma4 --company Spqrk

  # Back from offline mode to a cloud provider:
  autorename-pdf setup --cloud --provider openai --api-key-env OPENAI_API_KEY

  # What is still missing?
  autorename-pdf setup --check -o json

FOR AGENTS:
  1. `setup --check -o json`; read `steps[]` (status `missing` + `hint`).
  2. Ask the user only for the API key (or whether to run offline).
  3. `setup ... -o json`; exit 0 = ready, 3 = config still invalid (see the
     failed step's hint), 2 = bad flags, 1 = I/O error.
  4. Verify: `autorename-pdf rename --dry-run examples/demo -o json`.";

/// Set up config, API key, OCR models and desktop integration (idempotent)
#[derive(Debug, Args)]
#[command(long_about = SETUP_LONG, after_long_help = SETUP_AFTER)]
pub struct SetupArgs {
    /// Report what is missing; change nothing
    #[arg(long)]
    pub check: bool,
    /// AI provider (sets ai.provider; the model falls back to its default)
    #[arg(long, value_enum, value_name = "NAME")]
    pub provider: Option<ProviderArg>,
    /// Provider endpoint (ai.base_url), e.g. a gateway; "" = the default
    #[arg(long, value_name = "URL", conflicts_with = "ollama_url")]
    pub base_url: Option<String>,
    /// Leave offline mode (privacy: cloud) to use a cloud provider
    #[arg(long)]
    pub cloud: bool,
    /// Model id (ai.model)
    #[arg(long, value_name = "ID")]
    pub model: Option<String>,
    /// YOUR company name (company.name): tells bills from invoices you sent
    #[arg(long, value_name = "NAME")]
    pub company: Option<String>,
    /// Naming profile (business, academic, or a custom id)
    #[arg(long, value_name = "ID")]
    pub profile: Option<String>,
    /// Read the API key from environment variable VAR; store VAR=... in .env
    /// and `ai.api_key: "${VAR}"` in config.yaml
    #[arg(long, value_name = "VAR")]
    pub api_key_env: Option<String>,
    /// Read the API key from stdin (stored like --api-key-env, under VAR or
    /// the provider's default variable, e.g. OPENAI_API_KEY)
    #[arg(long)]
    pub api_key_stdin: bool,
    /// Company-name decider for ambiguous matches
    #[arg(long, value_enum, value_name = "NAME")]
    pub decider: Option<DeciderArg>,
    /// Read the decider key from VAR (harmonization.decider_api_key: "${VAR}")
    #[arg(long, value_name = "VAR")]
    pub decider_key_env: Option<String>,
    /// With --offline: Ollama URL [default: http://localhost:11434]
    #[arg(long, value_name = "URL")]
    pub ollama_url: Option<String>,
    /// With --offline: Ollama model [default: the provider default]
    #[arg(long, value_name = "MODEL")]
    pub ollama_model: Option<String>,
    /// Download the OCR model pack for ocr.language (refused when offline)
    #[arg(long)]
    pub ocr_models: bool,
    /// Install the desktop / file-manager integration (`integrate install`)
    #[arg(long)]
    pub integrate: bool,
}

// ---------------------------------------------------------------------------
// integrate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Scope {
    /// The current user only (no admin rights needed)
    User,
}

#[derive(Debug, Args)]
pub struct IntegrateArgs {
    /// Install scope (only `user` is supported)
    #[arg(long, value_enum, default_value = "user", value_name = "SCOPE")]
    pub scope: Scope,
    /// Executable the entries launch [default: this executable, or $APPIMAGE]
    #[arg(long, value_name = "PATH")]
    pub exe: Option<PathBuf>,
}

/// Add or remove desktop and file-manager integration
#[derive(Debug, Subcommand)]
#[command(
    long_about = "\
Add or remove desktop and file-manager integration for the current user:
\"AutoRename-PDF > Rename now / Preview\" on PDFs and folders.

  Linux    .desktop entry (PDF + folder MIME, actions), hicolor icons,
           Dolphin service menu, Nemo actions, Thunar custom actions
           (merged into uca.xml), Nautilus scripts (Scripts submenu, no
           icons: host limitation). File-manager entries only for file
           managers found on this system. Nemo/Thunar entries are
           skipped (\"path contains %\") when both the executable and the
           data dir paths contain `%`.
           CLI on PATH: ~/.local/bin/autorename-pdf symlink (AppImage,
           portable), unless `autorename-pdf` on PATH already is this exe.
  Windows  HKCU classic context menu for .pdf and folders (Windows 11:
           under \"Show more options\"), launched without a console window;
           the install folder on the user PATH (HKCU\\Environment Path).
  macOS    Finder Quick Actions in ~/Library/Services.

Idempotent. Every entry is marked as ours; `remove` deletes only marked
entries and never touches other files or menu items. Exit 5 when an entry
was kept because a file or key with our name is not ours. No network use.",
    after_long_help = "\
EXAMPLES:
  autorename-pdf integrate install
  autorename-pdf integrate status -o json
  autorename-pdf integrate remove"
)]
pub enum IntegrateCmd {
    /// Install (or update) the entries
    Install(IntegrateArgs),
    /// Remove our entries
    Remove(IntegrateArgs),
    /// Show what is installed
    Status(IntegrateArgs),
}
