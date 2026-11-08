//! egui/eframe desktop GUI (feature `gui`). Spec:
//! `docs/superpowers/specs/2026-11-07-gui-design.md`.

pub mod app;
pub mod components;
pub mod demo;
pub mod model;
pub mod motion;
pub mod pages;
pub mod screens;
pub mod theme;
#[cfg(target_os = "linux")]
mod wayland_dnd;
pub mod worker;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use autorename_core::ExitCode;

/// Is there a display to open a window on? (No args + display → GUI.)
pub fn has_display() -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        return true;
    }
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
}

/// Window/taskbar icon: the 256 px app icon (`cargo xtask icons`, spec §6).
fn window_icon() -> egui::IconData {
    const PNG: &[u8] =
        include_bytes!("../../../../assets/icons/generated/png/autorename-pdf-256.png");
    eframe::icon_data::from_png_bytes(PNG).unwrap_or_default()
}

/// How the window starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Launch {
    /// Normal window (preview pre-loaded with the paths).
    #[default]
    Preview,
    /// `gui --rename-now` (file-manager "Rename now"): compact window that
    /// renames right after the analysis unless something needs review.
    RenameNow,
}

pub fn run(
    paths: Vec<PathBuf>,
    config: Option<PathBuf>,
    offline: bool,
    launch: Launch,
) -> ExitCode {
    let size = match launch {
        Launch::Preview => theme::size::WINDOW_DEFAULT,
        Launch::RenameNow => theme::size::WINDOW_MIN,
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title("AutoRename-PDF")
        .with_app_id("autorename-pdf")
        .with_inner_size(size)
        .with_min_inner_size(theme::size::WINDOW_MIN)
        .with_drag_and_drop(true)
        .with_icon(window_icon());
    let options = eframe::NativeOptions {
        viewport,
        // The compact rename-now window must not resize the main window.
        persist_window: launch == Launch::Preview,
        ..Default::default()
    };
    let result = eframe::run_native(
        "AutoRename-PDF",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, paths, config, offline, launch)))),
    );
    match result {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            eprintln!("error: could not open the GUI: {e}");
            eprintln!("hint: use the CLI instead, see `autorename-pdf --help`");
            ExitCode::General
        }
    }
}
