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
    /// Open Settings on a page (`pages::page::*`), scrolled to the top.
    OpenSettings(usize),
    ShowLicense,
    OpenUrl(String),
    /// Leave Settings for the tab that was active before it.
    CloseSettings,
    SaveSetup,
    SelectProfile(String),
    SetFilter(Filter),
    Activate(usize),
    Toggle(usize),
    StartEdit(usize),
    CommitEdit,
    CancelEdit,
    ResetName(usize),
    DontLearn(usize),
    CopyText(String),
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
