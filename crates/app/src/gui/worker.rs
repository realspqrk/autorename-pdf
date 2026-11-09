//! Background work: analysis, apply, undo, provider ping and OCR model
//! downloads. Results come back over a channel; the UI never blocks.
//! Analysis and apply are cancellable through core's `cancel` flags.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Instant, SystemTime};

use autorename_core::config::Settings;
use autorename_core::harmonize::{self, NameDb};
use autorename_core::journal::{self, UndoReport, UndoTarget};
use autorename_core::ocr;
use autorename_core::pipeline::{
    self, ApplyOptions, ApplyReport, ErrorInfo, Event, PlanItem, PlanOptions,
};
use autorename_core::providers::{self, ExtractRequest, ProviderSettings};

pub enum Msg {
    Event(u64, Event),
    Items(u64, Vec<PlanItem>, Vec<String>),
    AnalysisFailed(u64, ErrorInfo),
    AnalysisDone(u64),
    Applied(Result<ApplyReport, ErrorInfo>),
    /// Batch id (for History), result.
    Undone(String, Result<UndoReport, ErrorInfo>),
    /// Latency in ms, or a short error.
    Tested(Result<u64, String>),
    ModelProgress {
        downloaded: u64,
        total: u64,
    },
    ModelDone(Result<String, String>),
    /// Name list loaded or edited (see [`NamesDone`]).
    Names(NamesDone),
    /// The system viewer/browser could not be started for this target.
    OpenFailed(PathBuf, String),
}

/// Edit of a name list, run under its lock by `harmonize::update_db`.
pub type NameEdit = Box<dyn FnOnce(&mut NameDb) -> autorename_core::Result<()> + Send>;

/// Name list job: a load, or an edit with the toast shown on success.
pub struct NamesJob {
    pub seq: u64,
    pub path: PathBuf,
    pub field: String,
    pub edit: Option<(NameEdit, String)>,
}

pub struct NamesDone {
    pub seq: u64,
    pub path: PathBuf,
    /// The list as on disk after the job, and its mtime (taken before the
    /// read, so a later change always shows up as a newer mtime).
    pub result: Result<(NameDb, Option<SystemTime>), String>,
    /// Toast of a successful edit; `None` for a load.
    pub done: Option<String>,
    pub edit: bool,
}

/// mtime of a name list (`None` = missing/unreadable).
pub fn names_mtime(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub struct Worker {
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    pub cancel: Arc<AtomicBool>,
    /// OCR model download cancel (separate from analysis/apply).
    pub model_cancel: Arc<AtomicBool>,
    pub repaint: Option<egui::Context>,
    /// Tests: targets of [`Worker::open`] (nothing is launched).
    #[cfg(test)]
    pub opened: Vec<PathBuf>,
}

impl Worker {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Worker {
            tx,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            model_cancel: Arc::new(AtomicBool::new(false)),
            repaint,
            #[cfg(test)]
            opened: Vec::new(),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn spawn(&self, f: impl FnOnce(&Sender<Msg>, &(dyn Fn() + Sync)) + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.repaint.clone());
        std::thread::spawn(move || {
            let repaint = || {
                if let Some(c) = &ctx {
                    c.request_repaint();
                }
            };
            f(&tx, &repaint);
            repaint();
        });
    }

    /// Open a file, folder or URL with the system's default program. The
    /// process launch runs here, never on the UI thread; a failure comes
    /// back as [`Msg::OpenFailed`].
    pub fn open(&mut self, target: PathBuf) {
        #[cfg(test)]
        {
            self.opened.push(target);
        }
        #[cfg(not(test))]
        self.spawn(move |tx, _| {
            let cmd = if cfg!(windows) {
                "explorer"
            } else if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            if let Err(e) = std::process::Command::new(cmd).arg(&target).spawn() {
                let _ = tx.send(Msg::OpenFailed(target, e.to_string()));
            }
        });
    }

    /// One `plan_files` run; Stop sets core's cancel flag (unanalyzed files
    /// come back as `cancelled` and leave the list).
    pub fn analyze(
        &mut self,
        generation: u64,
        files: Vec<PathBuf>,
        settings: Settings,
        mut opts: PlanOptions,
    ) {
        self.cancel = Arc::new(AtomicBool::new(false));
        opts.cancel = Some(self.cancel.clone());
        self.spawn(move |tx, repaint| {
            let progress = |e: &Event| {
                let _ = tx.send(Msg::Event(generation, e.clone()));
                repaint();
            };
            match pipeline::plan_files(&files, &settings, &opts, &progress) {
                Ok(plan) => {
                    let _ = tx.send(Msg::Items(generation, plan.items, plan.warnings));
                }
                Err(e) => {
                    let _ = tx.send(Msg::AnalysisFailed(generation, ErrorInfo::from_error(&e)));
                }
            }
            let _ = tx.send(Msg::AnalysisDone(generation));
        });
    }

    /// Apply; Stop takes effect between files (core never stops mid-file).
    pub fn apply(&mut self, plan: pipeline::Plan, settings: Settings, mut opts: ApplyOptions) {
        self.cancel = Arc::new(AtomicBool::new(false));
        opts.cancel = Some(self.cancel.clone());
        self.spawn(move |tx, repaint| {
            let progress = |e: &Event| {
                let _ = tx.send(Msg::Event(0, e.clone()));
                repaint();
            };
            let r = pipeline::apply_plan(&plan, &settings, &opts, &progress)
                .map_err(|e| ErrorInfo::from_error(&e));
            let _ = tx.send(Msg::Applied(r));
        });
    }

    pub fn undo(&self, journal_path: PathBuf, batch: String) {
        self.spawn(move |tx, _| {
            let r = journal::undo_at(
                &journal_path,
                &UndoTarget::Batch(batch.clone()),
                chrono::Utc::now(),
            )
            .map_err(|e| ErrorInfo::from_error(&e));
            let _ = tx.send(Msg::Undone(batch, r));
        });
    }

    /// Provider ping: a minimal extraction with a one-field schema.
    pub fn test_connection(&self, settings: Settings) {
        self.spawn(move |tx, _| {
            let mut cfg = ProviderSettings::from_settings(&settings);
            cfg.timeout = std::time::Duration::from_secs(30);
            cfg.retry.max_retries = 0;
            let req = ExtractRequest {
                system_prompt: "Answer with JSON only.".into(),
                user_text: "Return {\"ok\": \"yes\"}.".into(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {"ok": {"type": "string"}},
                    "required": ["ok"],
                    "additionalProperties": false
                }),
                schema_name: "ping".into(),
                images: Vec::new(),
                max_tokens: 32,
            };
            let start = Instant::now();
            let r = providers::extract(&req, &cfg)
                .map(|_| start.elapsed().as_millis() as u64)
                .map_err(|e| {
                    let i = ErrorInfo::from_error(&e);
                    format!("{} ({})", i.message, i.code)
                });
            let _ = tx.send(Msg::Tested(r));
        });
    }

    /// Load or edit a name list off the UI thread: `update_db` blocks on
    /// the list's `.lock` while another process writes it.
    pub fn names(&self, job: NamesJob) {
        self.spawn(move |tx, _| {
            let NamesJob {
                seq,
                path,
                field,
                edit,
            } = job;
            let is_edit = edit.is_some();
            let (result, done) = match edit {
                Some((f, done)) => {
                    let r = harmonize::update_db(&path, &field, |db| {
                        f(db)?;
                        Ok(db.clone())
                    })
                    .map(|db| (db, names_mtime(&path)))
                    .map_err(|e| e.to_string());
                    (r, Some(done))
                }
                None => {
                    let mtime = names_mtime(&path);
                    let r = match harmonize::load_db(&path, &field) {
                        Ok(db) => Ok((db, mtime)),
                        // No list yet: empty, not an error.
                        Err(_) if !path.exists() => Ok((NameDb::empty(&field), None)),
                        Err(e) => Err(e.to_string()),
                    };
                    (r, None)
                }
            };
            let _ = tx.send(Msg::Names(NamesDone {
                seq,
                path,
                result,
                done,
                edit: is_edit,
            }));
        });
    }

    /// Download (explicit user action), import or remove an OCR pack.
    pub fn cancel_models(&self) {
        self.model_cancel.store(true, Ordering::SeqCst);
    }

    pub fn models(&mut self, op: ModelOp) {
        self.model_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.model_cancel.clone();
        self.spawn(move |tx, repaint| {
            let Some(root) = ocr::models::default_root() else {
                let _ = tx.send(Msg::ModelDone(Err("No data folder for OCR models".into())));
                return;
            };
            let r = match op {
                ModelOp::Fetch(pack) => {
                    let mut last = Instant::now();
                    let mut progress = |p: ocr::models::Progress<'_>| {
                        if last.elapsed().as_millis() >= 100 || p.downloaded == p.total {
                            last = Instant::now();
                            let _ = tx.send(Msg::ModelProgress {
                                downloaded: p.downloaded,
                                total: p.total,
                            });
                            repaint();
                        }
                    };
                    ocr::models::fetch(&root, pack, &mut progress, Some(&cancel))
                        .map(|()| format!("Installed {}", pack.id))
                }
                ModelOp::Import(pack, dir) => {
                    ocr::models::import(&root, pack, &dir).map(|()| format!("Imported {}", pack.id))
                }
                ModelOp::Remove(pack) => {
                    ocr::models::remove(&root, pack).map(|()| format!("Removed {}", pack.id))
                }
            };
            let _ = tx.send(Msg::ModelDone(r.map_err(|e| e.to_string())));
        });
    }
}

pub enum ModelOp {
    Fetch(&'static ocr::Pack),
    Import(&'static ocr::Pack, PathBuf),
    Remove(&'static ocr::Pack),
}
