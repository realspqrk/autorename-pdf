//! Screens (spec §4). Each paints into a rect and pushes `Action`s; the app
//! applies them after the frame's layout pass.

pub mod companies;
pub mod detail;
pub mod empty;
pub mod header;
pub mod history;
pub mod preview;
pub mod settings;

use std::path::PathBuf;

use super::model::{Filter, Tab};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    ChooseFiles,
    ChooseFolder,
    AddPaths(Vec<PathBuf>),
    Rename,
    Stop,
    ClearList,
    Undo,
    RenameMore,
    Retry(usize),
    RetryFailed,
    OpenFolder,
    SwitchTab(Tab),
    /// Gear: open Settings, or close it back to the previous tab.
    ToggleSettings,
    /// About: show the license or the third-party notices dialog.
    ShowDoc(super::pages::TextDoc),
    OpenUrl(String),
    /// Leave Settings for the tab that was active before it.
    CloseSettings,
    SaveSetup,
    SetFilter(Filter),
    /// Show/hide the details panel (toolbar toggle, Ctrl+\).
    ToggleDetails,
    Activate(usize),
    Toggle(usize),
    /// Check every checkable row (Ctrl+A, row menu).
    IncludeAll,
    /// Remove a row from the plan, not from disk (Delete, row menu).
    RemoveRow(usize),
    StartEdit(usize),
    /// Commit the inline edit of `row` if it is still the open one and
    /// valid; `refocus` returns focus to the row (Enter, Save); after a
    /// click away focus stays where it went.
    CommitEdit {
        row: usize,
        refocus: bool,
    },
    CancelEdit,
    ResetName(usize),
    DontLearn(usize),
    /// Copy to the clipboard, then show the toast.
    CopyText(String, &'static str),
    OpenPath(PathBuf),
    TestSetup,
    TestSettings,
    SaveSettings,
    DiscardSettings,
    ModelFetch(String),
    ModelImport,
    ModelCancel,
    ModelRemove(String),
    OpenConfigFile,
    OpenConfigFolder,
    OpenCompaniesFile,
    KeepCompany(String),
    MergeCompany(String, String),
    RenameCompany(String, String),
    RemoveAlias(String, String),
    UndoBatch(String),
    DismissBanner(usize),
    BannerPrimary(usize),
}
