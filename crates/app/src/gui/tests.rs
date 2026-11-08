//! egui_kittest snapshot + accessibility tests (spec §9). Snapshots live in
//! `crates/app/tests/snapshots/gui/`; refresh with `UPDATE_SNAPSHOTS=force`.

use egui::{Key, Modifiers, Rect, Role};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};

use super::app::App;
use super::components::set_reduce_motion;
use super::model::{Model, Phase};
use super::pages::page;
use super::theme::Tone;
use super::{demo, theme};

/// kittest's wgpu renderer is not safe to create/use from several test
/// threads at once (SIGSEGV in the driver under the default parallel
/// `cargo test`). Every GUI test holds this lock for its whole run.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const SIZES: [(f32, f32); 2] = [(1280.0, 800.0), (960.0, 640.0)];

fn harness(model: fn() -> Model, size: (f32, f32), dark: bool) -> Harness<'static, App> {
    let theme_ = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    let mut h = Harness::builder()
        .with_size(theme::v2(size.0, size.1))
        .with_theme(theme_)
        .with_options(SnapshotOptions::new().output_path("tests/snapshots/gui"))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            let mut m = model();
            m.prefs.theme = if dark { 2 } else { 1 };
            m.prefs.reduce_motion = 1;
            let mut app = App::with_model(m, None);
            app.apply_prefs(&cc.egui_ctx);
            set_reduce_motion(&cc.egui_ctx, true);
            app
        });
    h.run();
    h
}

type Screen = (&'static str, fn() -> Model);

fn screens() -> Vec<Screen> {
    vec![
        ("empty", demo::empty_model),
        ("empty_offline", demo::offline_setup_model),
        ("empty_configured", demo::configured_empty_model),
        ("preview", demo::preview_model),
        ("progress", demo::progress_model),
        ("renaming", demo::renaming_model),
        ("done", demo::done_model),
        ("partial", demo::partial_model),
        ("edit_invalid", demo::invalid_edit_model),
        ("edit_valid", demo::valid_edit_model),
        ("banners_danger_warning", || {
            demo::banners_model([Tone::Warning, Tone::Danger])
        }),
        ("banners_info_success", || {
            demo::banners_model([Tone::Success, Tone::Accent])
        }),
        ("settings_general", || demo::settings_model(page::GENERAL)),
        ("settings_processing", || {
            demo::settings_model(page::PROCESSING)
        }),
        ("settings_privacy_dirty", demo::settings_dirty_model),
        ("settings_naming", || demo::settings_model(page::NAMING)),
        ("settings_about", || demo::settings_model(page::ABOUT)),
        ("companies_review", demo::companies_model),
        ("companies_merge", demo::merge_model),
        ("history", demo::history_model),
    ]
}

fn snap_name(name: &str, size: (f32, f32), dark: bool) -> String {
    format!(
        "{name}_{}x{}_{}",
        size.0 as u32,
        size.1 as u32,
        if dark { "dark" } else { "light" }
    )
}

#[test]
fn snapshots() {
    let _gpu = gpu_lock();
    let mut results = SnapshotResults::new();
    for (name, model) in screens() {
        for size in SIZES {
            for dark in [false, true] {
                let mut h = harness(model, size, dark);
                results.add(h.try_snapshot(snap_name(name, size, dark)));
            }
        }
    }
    results.unwrap();
}

/// Settings pages scrolled to a lower group: (name, model, node label).
type Scrolled = (&'static str, fn() -> Model, &'static str);

fn scrolled_settings() -> Vec<Scrolled> {
    vec![
        (
            "settings_processing_reading",
            || demo::settings_model(page::PROCESSING),
            "Read page images with AI",
        ),
        (
            "settings_processing_unreadable",
            demo::settings_ocr_unreadable_model,
            "Scan languages: Automatic (all installed languages)",
        ),
        (
            "settings_processing_advanced",
            demo::settings_advanced_model,
            "Decrease OCR threads",
        ),
        (
            "settings_naming_matching",
            || demo::settings_model(page::NAMING),
            "Extra instructions",
        ),
        (
            "settings_naming_jev",
            demo::settings_decider_jev_model,
            "Jev key",
        ),
        (
            "settings_naming_invalid",
            demo::settings_naming_invalid_model,
            "Custom date format",
        ),
        (
            "settings_naming_academic",
            demo::settings_naming_academic_model,
            "Extra instructions",
        ),
    ]
}

#[test]
fn settings_scrolled_snapshots() {
    let _gpu = gpu_lock();
    let mut results = SnapshotResults::new();
    for (name, model, target) in scrolled_settings() {
        for size in SIZES {
            for dark in [false, true] {
                let mut h = harness(model, size, dark);
                h.get_by_label(target).scroll_to_me();
                h.run();
                results.add(h.try_snapshot(snap_name(name, size, dark)));
            }
        }
    }
    results.unwrap();
}

/// Keyboard focus on a row, on Rename and hover on a row, all sizes/themes.
#[test]
fn focus_and_hover_snapshots() {
    let _gpu = gpu_lock();
    let mut results = SnapshotResults::new();
    for size in SIZES {
        for dark in [false, true] {
            let mut h = harness(demo::preview_model, size, dark);
            tab_to(&mut h, |l| l.contains("Rechnung (2).pdf"));
            results.add(h.try_snapshot(snap_name("focus_row", size, dark)));
            tab_to(&mut h, |l| l == "Rename 10 files");
            results.add(h.try_snapshot(snap_name("focus_rename", size, dark)));
            let mut h = harness(demo::preview_model, size, dark);
            let r = h
                .get_by_label_contains("scan_0041.pdf, will be renamed")
                .rect();
            h.hover_at(r.center());
            h.run();
            results.add(h.try_snapshot(snap_name("hover_row", size, dark)));
            // First and last row focus (ring stays inside the scroll clip).
            let mut h = harness(demo::preview_model, size, dark);
            tab_to(&mut h, |l| l.contains("Rechnung (2).pdf"));
            h.key_press(Key::Home);
            h.run();
            results.add(h.try_snapshot(snap_name("focus_first_row", size, dark)));
            h.key_press(Key::End);
            h.run();
            results.add(h.try_snapshot(snap_name("focus_last_row", size, dark)));
            // Pressed primary (pointer held down on Rename).
            let mut h = harness(demo::preview_model, size, dark);
            let r = h.get_by_label("Rename 10 files").rect();
            h.drag_at(r.center());
            h.step();
            h.step();
            results.add(h.try_snapshot(snap_name("pressed_rename", size, dark)));
            // Detail scrolled to the expanded Warnings section.
            let mut h = harness(demo::warning_detail_model, size, dark);
            h.get_by_label_contains("Warnings section").scroll_to_me();
            h.run();
            results.add(h.try_snapshot(snap_name("detail_warnings", size, dark)));
        }
    }
    results.unwrap();
}

fn tab_to(h: &mut Harness<'_, App>, pred: impl Fn(&str) -> bool) -> Vec<String> {
    let mut order = Vec::new();
    for _ in 0..80 {
        h.key_press(Key::Tab);
        h.run();
        let l = focused_label(h);
        order.push(l.clone());
        if pred(&l) {
            return order;
        }
    }
    panic!("focus target not reached; order: {order:#?}");
}

fn focused_label(h: &Harness<'_, App>) -> String {
    h.query_by(|n| n.is_focused())
        .and_then(|n| n.accesskit_node().label())
        .unwrap_or_default()
}

const INTERACTIVE: [Role; 9] = [
    Role::RadioGroup,
    Role::Button,
    Role::CheckBox,
    Role::ComboBox,
    Role::Tab,
    Role::RadioButton,
    Role::Switch,
    Role::ListItem,
    Role::TextInput,
];

fn on_grid(v: f32) -> bool {
    (v / 4.0).fract().abs() < 0.01
}

/// Every interactive rect on ≥ 5 screens: 4-pt grid, ≥ 24 px targets,
/// 32 px standard controls, rows 56/72, labels present (checklist 2/5/7).
#[test]
fn measured_geometry_all_interactive_nodes() {
    let _gpu = gpu_lock();
    let samples: Vec<Screen> = vec![
        ("empty", demo::empty_model),
        ("preview", demo::preview_model),
        ("progress", demo::progress_model),
        ("done", demo::done_model),
        ("edit_invalid", demo::invalid_edit_model),
        ("settings_general", || demo::settings_model(page::GENERAL)),
        ("settings_processing", || {
            demo::settings_model(page::PROCESSING)
        }),
        (
            "settings_ocr_unreadable",
            demo::settings_ocr_unreadable_model,
        ),
        ("settings_advanced", demo::settings_advanced_model),
        ("settings_naming", || demo::settings_model(page::NAMING)),
        ("settings_naming_jev", demo::settings_decider_jev_model),
        ("companies_review", demo::companies_model),
        ("history", demo::history_model),
    ];
    let mut problems = Vec::new();
    for (name, model) in samples {
        for size in SIZES {
            let h = harness(model, size, false);
            let window = Rect::from_min_size(egui::Pos2::ZERO, theme::v2(size.0, size.1));
            for role in INTERACTIVE {
                for n in h.query_all_by_role(role) {
                    let r = n.rect();
                    let label = n.accesskit_node().label().unwrap_or_default();
                    if !window.intersects(r) || r.height() <= 0.0 {
                        continue;
                    }
                    let id = format!("{name} {size:?} {role:?} {label:?} {r:?}");
                    if label.is_empty() {
                        problems.push(format!("unlabeled: {id}"));
                    }
                    if r.width() < 24.0 || r.height() < 24.0 {
                        problems.push(format!("target < 24: {id}"));
                    }
                    let grid_ok = match role {
                        // Text inputs: our 32 px frame is on the grid; the
                        // inner edit is inset 12 and ends at the adornment.
                        Role::TextInput => on_grid(r.top()) && on_grid(r.height()),
                        // Profile rows are one radio each (72 h, §4.7).
                        Role::RadioButton if r.height() == theme::size::ROW_EDIT => {
                            on_grid(r.left()) && on_grid(r.top()) && on_grid(r.width())
                        }
                        // Segments sit inside the 32 px group with the 2 px
                        // inset (§5.4); the group is checked instead.
                        Role::RadioButton => r.height() == theme::size::SEGMENT,
                        // Rows span the φ list width (791 / 591, §3).
                        Role::ListItem => {
                            on_grid(r.left()) && on_grid(r.top()) && on_grid(r.height())
                        }
                        _ => {
                            on_grid(r.left())
                                && on_grid(r.top())
                                && on_grid(r.width())
                                && on_grid(r.height())
                        }
                    };
                    // Rows clipped by the scroll viewport are partial.
                    let clipped = role == Role::ListItem && r.height() < theme::size::ROW;
                    if !grid_ok && !clipped {
                        problems.push(format!("off 4-pt grid: {id}"));
                    }
                    match role {
                        Role::Button
                            if r.height() != 24.0 && r.height() != 32.0 && r.height() != 40.0 =>
                        {
                            problems.push(format!("button height: {id}"))
                        }
                        // Multi-line field: 4 rows inside 8 px padding (96 h).
                        Role::TextInput if label == "Extra instructions" && r.height() == 80.0 => {}
                        Role::ComboBox | Role::Tab | Role::TextInput if r.height() != 32.0 => {
                            problems.push(format!("control height: {id}"))
                        }
                        Role::ListItem
                            if !clipped
                                && ![theme::size::ROW, theme::size::ROW_EDIT]
                                    .contains(&r.height()) =>
                        {
                            problems.push(format!("row height: {id}"))
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// Pane geometry: φ split ±1, header 48, toolbar 40, action bar 56, progress
/// confined to the list (checklist 2, report items 4/10).
#[test]
fn layout_regions_and_phi_split() {
    let _gpu = gpu_lock();
    for size in SIZES {
        let h = harness(demo::preview_model, size, false);
        let detail = theme::detail_width(size.0);
        let row = h
            .get_by_label_contains("scan_0041.pdf, will be renamed to")
            .rect();
        assert_eq!(row.left(), 0.0);
        assert!(
            (row.width() - (size.0 - detail - 1.0)).abs() <= 1.0,
            "{size:?} {row:?}"
        );
        assert_eq!(row.top(), theme::size::HEADER + theme::size::TOOLBAR);
        let rename = h.get_by_label("Rename 10 files").rect();
        assert_eq!(size.1 - rename.center().y, theme::size::ACTION_BAR / 2.0);
        assert_eq!(size.0 - rename.right(), theme::space::LG);
        let tab = h.get_by_role_and_label(Role::Tab, "Rename").rect();
        assert_eq!(tab.center().y, theme::size::HEADER / 2.0);
        let h = harness(demo::progress_model, size, false);
        let p = h.get_by_role(Role::ProgressIndicator).rect();
        assert_eq!(p.left(), 0.0);
        assert!(
            (p.width() - (size.0 - detail - 1.0)).abs() <= 1.0,
            "progress {p:?}"
        );
        assert_eq!(p.height(), theme::size::PROGRESS_H);
    }
}

/// Row labels carry old name, new name and status words (§8, checklist 7).
#[test]
fn accessibility_labels() {
    let _gpu = gpu_lock();
    let h = harness(demo::preview_model, SIZES[0], false);
    let row = h.get_by_label_contains(
        "scan_0041.pdf, will be renamed to 20251103 Amazon AP 12,13.pdf, text extraction, metadata will be written, included",
    );
    assert_eq!(row.accesskit_node().role(), Role::ListItem);
    h.get_by_label_contains("broken.pdf, error:");
    h.get_by_label_contains(
        "Vertrag_signed.pdf, will be renamed to 20250901 Stadtwerke München Contract.pdf, text extraction, metadata skipped, signed PDF",
    );
    let excl = h
        .get_by_label_contains("statement_oct.pdf, will be renamed to")
        .accesskit_node()
        .label()
        .unwrap_or_default();
    assert!(excl.ends_with("excluded"), "{excl}");
    h.get_by_label("Privacy: cloud, OpenAI gpt-6-luna. Open privacy settings");
    h.get_by_label("Copy original path");
    h.get_by_label("Edit proposed name");
    h.get_by_label("Profile: Business");
    h.get_by_label_contains("Company match section, expanded");
}

/// Tab order header → toolbar → list → detail → action bar; one row stop.
#[test]
fn keyboard_tab_order_and_enter_on_rename() {
    let _gpu = gpu_lock();
    let mut h = harness(demo::preview_model, SIZES[0], false);
    let order = tab_to(&mut h, |l| l == "Rename 10 files");
    let pos = |needle: &str| {
        order
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} not in tab order {order:#?}"))
    };
    let (header, settings, search, row, copy, rename) = (
        pos("Rename"),
        pos("Settings"),
        pos("Search files"),
        pos("will be renamed to"),
        pos("Copy original path"),
        pos("Rename 10 files"),
    );
    assert!(
        header < settings && settings < search && search < row && row < copy && copy < rename,
        "{order:#?}"
    );
    assert_eq!(
        order
            .iter()
            .filter(|l| l.contains("will be renamed to"))
            .count(),
        1
    );
    h.key_press(Key::Enter);
    h.run();
    assert!(matches!(h.state().m.phase, Phase::Done));
}

/// First run → setup → picker (Ctrl+O) → edit → exclude → rename → undo,
/// keyboard only (checklist 6).
#[test]
fn keyboard_only_flow_from_first_run() {
    let _gpu = gpu_lock();
    let mut h = harness(demo::empty_model, SIZES[0], false);
    assert!(!h.state().m.is_set_up());
    tab_to(&mut h, |l| l == "API key");
    for c in "sk-test".chars() {
        h.event(egui::Event::Text(c.to_string()));
    }
    h.run();
    assert_eq!(h.state().m.setup.api_key, "sk-test");
    tab_to(&mut h, |l| l == "Save and continue");
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().m.is_set_up(), "setup saved");
    h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    h.run();
    assert_eq!(h.state().m.rows.len(), 12, "picked files are listed");
    tab_to(&mut h, |l| l.contains("scan_0041.pdf"));
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().m.active, Some(1));
    assert!(focused_label(&h).contains("Rechnung (2).pdf"));
    h.key_press(Key::F2);
    h.run();
    assert!(h.state().m.editing.is_some());
    for _ in 0..12 {
        h.key_press(Key::Backspace);
    }
    for c in " Telekom".chars() {
        h.event(egui::Event::Text(c.to_string()));
    }
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().m.editing.is_none(), "Enter commits");
    let edited = h.state().m.rows[1].edited.clone().unwrap_or_default();
    assert!(edited.ends_with("Telekom.pdf"), "{edited}");
    h.key_press(Key::ArrowUp);
    h.run();
    h.key_press(Key::Space);
    h.run();
    assert!(!h.state().m.rows[0].checked, "Space excludes");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    let rep = h.state().m.report.clone().expect("renamed");
    assert!(
        rep.files
            .iter()
            .all(|f| !f.source.ends_with("scan_0041.pdf"))
    );
    tab_to(&mut h, |l| l == "Undo rename");
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().m.undone);
}

/// Ctrl+Enter only triggers the visible, enabled primary of the current
/// tab: never a hidden Rename behind Settings or during an inline edit.
#[test]
fn ctrl_enter_follows_visible_primary() {
    let _gpu = gpu_lock();
    let mut h = harness(demo::settings_dirty_model, SIZES[0], false);
    h.state_mut().m.rows = demo::preview_model().rows;
    h.state_mut().m.meta = demo::preview_model().meta;
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().m.report.is_none(), "no rename from Settings");
    assert!(h.state().m.settings.dirty(), "invalid form is not saved");
    let mut h = harness(demo::invalid_edit_model, SIZES[0], false);
    let b = h.get_by_label("Rename 10 files");
    assert!(
        b.accesskit_node().is_disabled(),
        "Rename disabled while editing"
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().m.report.is_none(), "no rename while editing");
    // Valid Settings change: Ctrl+Enter saves (demo: in memory).
    let mut h = harness(|| demo::settings_model(page::PROCESSING), SIZES[0], false);
    h.state_mut().m.settings.form.enrich = false;
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(!h.state().m.settings.dirty(), "saved");
}

/// Retry after a partial result builds a fresh plan of failed/not-reached
/// files only.
#[test]
fn retry_failed_after_partial() {
    let _gpu = gpu_lock();
    let mut h = harness(demo::partial_model, SIZES[0], false);
    let targets = h.state().m.retry_targets().len();
    assert!(targets >= 2);
    h.get_by_label_contains("Retry failed").click();
    h.run();
    assert!(h.state().m.report.is_none());
    assert_eq!(h.state().m.rows.len(), targets);
}

/// Inline edit rejects a name another row already uses (§4.3).
#[test]
fn edit_collision_is_reported() {
    let _gpu = gpu_lock();
    let h = harness(demo::invalid_edit_model, SIZES[0], false);
    let m = &h.state().m;
    let (i, t) = m.editing.clone().unwrap();
    assert!(
        m.edit_error(i, &t)
            .is_some_and(|e| e.contains("already uses"))
    );
}

/// Reduce motion: one frame after a state change, nothing more is
/// requested (checklist 10).
#[test]
fn reduce_motion_settles_in_one_frame() {
    let _gpu = gpu_lock();
    for model in [demo::progress_model as fn() -> Model, demo::preview_model] {
        let mut h = harness(model, SIZES[0], false);
        h.key_press(Key::ArrowDown);
        // kittest feeds key down and key up on two frames (input-driven
        // repaints); the frame after that must not ask for another.
        h.step();
        h.step();
        h.step();
        assert!(
            !h.ctx.has_requested_repaint(),
            "still requesting repaints after reduce-motion state change: {:?}",
            h.ctx.repaint_causes()
        );
    }
}

#[test]
fn copy_lint() {
    let _gpu = gpu_lock();
    for (name, model) in screens() {
        eprintln!("copy_lint screen {name}");
        let h = harness(model, SIZES[0], false);
        for n in h.query_all_by_role(Role::Button) {
            let l = n.accesskit_node().label().unwrap_or_default();
            assert!(!l.contains('!'), "{l}");
            assert_ne!(l, "OK");
            let words: Vec<&str> = l.split_whitespace().collect();
            for pair in words.windows(2) {
                let (prev, w) = (pair[0], pair[1]);
                let cap = w.chars().next().is_some_and(char::is_uppercase);
                let lower_rest = w.chars().skip(1).all(|c| !c.is_uppercase());
                let sentence_start = prev.ends_with('.') || prev.ends_with(':');
                let proper = [
                    "Telekom",
                    "Deutschland",
                    "Austria",
                    "Amazon",
                    "Hetzner",
                    "Online",
                    "Müller",
                    "Bau",
                    "Sponsors",
                ]
                .iter()
                .any(|p| w.trim_matches(|c: char| !c.is_alphanumeric()) == *p);
                assert!(
                    !cap || !lower_rest
                        || sentence_start
                        || proper
                        || w.contains('.')
                        || w.contains('_'),
                    "Title Case button: {l}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Policy regressions (no rendering needed)
// ---------------------------------------------------------------------------

/// A local URL nothing listens on (port bound then dropped), so offline
/// tests never reach a real Ollama: requests fail fast and deterministically.
fn closed_local_url() -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_or(9, |a| a.port());
    format!("http://127.0.0.1:{port}")
}

/// Demo config switched to Offline/Ollama at a closed local port.
fn offline_settings(m: &Model) -> autorename_core::config::Settings {
    use autorename_core::config;
    let mut tree = m.config.tree.clone().unwrap();
    tree["privacy"] = "offline".into();
    tree["ai"]["provider"] = "ollama".into();
    tree["ai"]["model"] = config::default_model("ollama").unwrap_or("gemma4").into();
    tree["ai"]["base_url"] = closed_local_url().into();
    tree["harmonization"]["decider"] = "off".into();
    config::settings(&tree, None).expect("offline settings")
}

fn total(app: &App) -> usize {
    match &app.m.phase {
        Phase::Analyzing(a) => a.total,
        _ => 0,
    }
}

fn bare_app(m: Model) -> (App, egui::Context) {
    (App::with_model(m, None), egui::Context::default())
}

/// `--offline` (or saved offline privacy) never lets a connection test or a
/// model download reach the network.
#[test]
fn offline_blocks_cloud_tests_and_model_downloads() {
    use super::screens::Action;
    // Setup card: cloud provider chosen although --offline is set.
    let mut m = demo::empty_model();
    m.demo = false;
    m.force_offline = true;
    m.setup.offline = false;
    m.setup.provider = 0;
    m.setup.api_key = "sk-test".into();
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::TestSetup);
    assert!(!app.m.setup_testing, "no request started");
    assert!(matches!(&app.m.setup_test, Some(Err(e)) if e.contains("--offline")));

    // Settings form edited to cloud while --offline.
    let mut m = demo::settings_model(page::PROCESSING);
    m.demo = false;
    m.force_offline = true;
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::TestSettings);
    assert!(!app.m.settings.testing);
    assert!(matches!(&app.m.settings.test, Some(Err(e)) if e.contains("--offline")));

    // Saved config offline, form switched to cloud but not saved.
    let mut m = demo::settings_model(page::PROCESSING);
    m.demo = false;
    if let Some(s) = m.config.settings.as_mut() {
        s.privacy = autorename_core::config::Privacy::Offline;
    }
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::TestSettings);
    assert!(!app.m.settings.testing);
    assert!(matches!(&app.m.settings.test, Some(Err(e)) if e.contains("Save the switch")));

    // Model download under --offline: refused before any worker starts.
    let mut m = demo::settings_model(page::PROCESSING);
    m.settings.model_busy = None;
    m.demo = false;
    m.force_offline = true;
    let (mut app, ctx) = bare_app(m);
    let id = autorename_core::ocr::PACKS[0].id.to_string();
    app.act(&ctx, Action::ModelFetch(id));
    assert!(app.m.settings.model_busy.is_none());
    assert!(matches!(&app.m.settings.model_msg, Some(Err(e)) if e == super::app::DOWNLOAD_BLOCKED));
}

/// Changing the profile makes every result stale (fingerprint) and
/// re-analyzes the whole plan with that profile: one plan never mixes
/// profiles.
#[test]
fn profile_change_rebinds_plan_settings() {
    use super::screens::Action;
    let mut m = demo::preview_model();
    m.demo = false;
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SelectProfile("academic".into()));
    let m = &app.m;
    assert_eq!(
        m.meta.as_ref().map(|x| x.profile.as_str()),
        Some("academic")
    );
    assert_eq!(m.run_fp, m.current_fp(), "run uses the current profile");
    assert_eq!(m.rows.len(), 12);
    assert!(m.rows.iter().all(|r| r.item.is_none()), "rows re-queued");
    assert_eq!(total(&app), 12);
    app.act(&ctx, Action::Stop);
}

/// Settings can't be saved (privacy/provider change) under a running job.
#[test]
fn settings_save_blocked_while_busy() {
    use super::screens::Action;
    let mut m = demo::progress_model();
    m.settings.form.offline = true;
    m.settings.form.provider = 4;
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.dirty(), "not saved");
    assert!(
        app.m
            .settings
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Wait"))
    );
}

/// Cloud results + Offline now current → Retry re-analyzes every stale row
/// with the offline settings, never the old cloud ones.
#[test]
fn saved_offline_unbinds_cloud_plan() {
    use super::screens::Action;
    let mut m = demo::preview_model();
    m.demo = false;
    m.config.settings = Some(offline_settings(&m));
    let (mut app, ctx) = bare_app(m);
    assert_eq!(app.m.stale_count(), 12, "cloud results are stale");
    assert!(!app.m.can_rename());
    app.sync_stale_banner();
    assert!(
        app.m
            .banners
            .iter()
            .any(|b| b.action.as_deref() == Some(super::app::REANALYZE))
    );
    app.act(&ctx, Action::Retry(2));
    assert_eq!(total(&app), 12, "all rows, not only the retried one");
    assert_eq!(app.m.run_fp, app.m.current_fp());
    assert_eq!(
        app.m.meta.as_ref().map(|x| x.provider.as_str()),
        Some("ollama")
    );
    assert!(
        !app.m
            .banners
            .iter()
            .any(|b| b.action.as_deref() == Some(super::app::REANALYZE)),
        "no stale rows left"
    );
    app.act(&ctx, Action::Stop);
}

/// Codex NEW-3 probe: analyzed plan → Offline current → Add one file: every
/// old row stays and is re-analyzed together with the new file; rows without
/// a result are never dropped when the run ends.
#[test]
fn add_after_settings_change_reanalyzes_all_rows() {
    use super::screens::Action;
    let dir = tempfile::tempdir().unwrap();
    let new_pdf = dir.path().join("new.pdf");
    std::fs::write(&new_pdf, b"%PDF-1.4\n%%EOF\n").unwrap();
    let mut m = demo::preview_model();
    m.demo = false;
    m.config.settings = Some(offline_settings(&m));
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::AddPaths(vec![new_pdf.clone()]));
    assert_eq!(app.m.rows.len(), 13, "old rows kept, new row added");
    assert_eq!(
        total(&app),
        13,
        "all stale rows + the new file are analyzed"
    );
    assert_eq!(app.m.run_fp, app.m.current_fp());
    let generation = app.m.generation;
    app.worker_send_done(generation);
    app.frame_poll(&ctx);
    assert_eq!(app.m.rows.len(), 13, "no row is silently removed");
    assert!(
        app.m
            .rows
            .iter()
            .all(|r| r.item.is_some() || r.not_analyzed)
    );
    assert!(
        app.m.retry_targets().len() >= 12,
        "unanalyzed rows offer Retry"
    );
}

/// Real SaveSettings on a temp config.yaml, cloud → Offline.
fn saved_offline_app(base_url: &str) -> (tempfile::TempDir, App) {
    use super::model::ConfigState;
    use super::pages::SettingsForm;
    use super::screens::Action;
    use autorename_core::config::Privacy;
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.yaml");
    std::fs::write(&cfg, crate::cmd::STARTER_CONFIG).unwrap();
    std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=sk-test\n").unwrap();
    let mut m = demo::preview_model();
    m.demo = false;
    m.config = ConfigState::load(Some(&cfg), false);
    let cloud = m.config.settings.clone().expect("cloud settings");
    assert_eq!(cloud.privacy, Privacy::Cloud);
    m.stamp_current();
    let form = SettingsForm::from_settings(&cloud);
    m.settings.saved = form.clone();
    m.settings.form = form;
    m.settings.loaded = true;
    m.settings.form.offline = true;
    m.settings.form.provider = 4;
    m.settings.form.model = String::new();
    m.settings.form.base_url = base_url.to_string();
    m.settings.form.decider = 3;
    let (mut app, ctx) = bare_app(m);
    assert!(
        app.m.stale_count() == 0 && app.m.can_rename(),
        "cloud plan current before save"
    );
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("privacy: offline"), "saved to disk");
    assert_eq!(
        app.m.config.settings.as_ref().map(|s| s.privacy),
        Some(Privacy::Offline)
    );
    (dir, app)
}

/// Codex NEW-4: through the REAL Settings save: cloud plan → save Offline →
/// Retry one row re-analyzes the whole plan; Rename stays blocked meanwhile.
#[test]
fn real_save_offline_then_retry_reanalyzes_whole_plan() {
    use super::screens::Action;
    let (_d, mut app) = saved_offline_app(&closed_local_url());
    let ctx = egui::Context::default();
    assert!(app.m.stale_count() > 0, "results stale after the real save");
    assert!(!app.m.can_rename(), "Rename blocked while stale");
    assert!(
        app.m
            .banners
            .iter()
            .any(|b| b.action.as_deref() == Some(super::app::REANALYZE))
    );
    app.act(&ctx, Action::Retry(2));
    assert_eq!(total(&app), 12, "whole plan, not only the retried row");
    assert_eq!(app.m.run_fp, app.m.current_fp());
    assert_eq!(
        app.m.meta.as_ref().map(|x| x.provider.as_str()),
        Some("ollama")
    );
    app.act(&ctx, Action::Stop);
}

/// Codex NEW-4 residue: RetryFailed (= Ctrl+R) after a real save schedules
/// every stale row even though no row is an error.
#[test]
fn real_save_offline_then_retry_failed_reanalyzes_whole_plan() {
    let _gpu = gpu_lock();
    use super::screens::Action;
    let (_d, mut app) = saved_offline_app(&closed_local_url());
    let ctx = egui::Context::default();
    app.act(&ctx, Action::RetryFailed);
    assert_eq!(total(&app), 12, "Ctrl+R re-analyzes every stale row");
    app.act(&ctx, Action::Stop);
    let (_d, mut app) = saved_offline_app(&closed_local_url());
    app.m.tab = super::model::Tab::Rename;
    let mut h = Harness::builder()
        .with_size(theme::v2(SIZES[0].0, SIZES[0].1))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            app.apply_prefs(&cc.egui_ctx);
            app
        });
    h.run();
    let before = h.state().m.generation;
    h.key_press_modifiers(Modifiers::COMMAND, Key::R);
    h.step();
    h.step();
    assert!(h.state().m.generation > before, "Ctrl+R started a run");
    assert_eq!(h.state().m.run_fp, h.state().m.current_fp());
    h.state_mut().act(&egui::Context::default(), Action::Stop);
}

/// Codex NEW-5: Stop while the LAST file's request is in flight. Core lets
/// that request finish; the complete, current-settings plan must be
/// renamable (no stranded state). Local mock delays the response.
#[test]
fn stop_during_last_inflight_request_keeps_plan_renamable() {
    use super::screens::Action;
    use std::io::{Read, Write};
    let _gpu = gpu_lock();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("local mock");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        let mut buf = vec![0; 1 << 20];
        let (mut read, mut need) = (0, usize::MAX);
        while read < need {
            let n = stream.read(&mut buf[read..]).unwrap();
            assert!(n > 0, "client closed early");
            read += n;
            if let Some(pos) = buf[..read].windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                let len = head
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|n| n.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                need = pos + 4 + len;
            }
        }
        accepted_tx.send(()).unwrap();
        release_rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .unwrap();
        let fields = serde_json::json!({
            "document_date": "07.11.2026", "company_name": "Probe Co",
            "issuer": "Probe Co", "recipient": "", "document_type": "AP",
            "total_amount": "42", "currency": "", "description": ""
        });
        let body = serde_json::json!({"message": {"content": fields.to_string()}, "done": true})
            .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let (dir, mut app) = saved_offline_app(&url);
    // One real PDF row.
    let pdf = dir.path().join("probe.pdf");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/minimal_text.pdf"
        ),
        &pdf,
    )
    .unwrap();
    app.m.rows.truncate(1);
    app.m.rows[0].source = pdf.clone();
    if let Some(it) = app.m.rows[0].item.as_mut() {
        it.source = pdf.clone();
    }
    app.m.active = Some(0);
    let ctx = egui::Context::default();
    app.act(&ctx, Action::Retry(0));
    assert_eq!(total(&app), 1);
    accepted_rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("request reached the mock");
    app.act(&ctx, Action::Stop);
    release_tx.send(()).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.m.busy() && std::time::Instant::now() < end {
        app.poll_once(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    server.join().unwrap();
    assert!(!app.m.busy(), "worker finished");
    let r = &app.m.rows[0];
    assert!(
        r.item.is_some(),
        "in-flight request completed: {:?}",
        r.item
    );
    assert_eq!(
        r.fp,
        app.m.current_fp(),
        "result stamped with current settings"
    );
    assert!(
        app.m.can_rename(),
        "complete current plan stays renamable after Stop"
    );
    assert!(
        !app.m
            .banners
            .iter()
            .any(|b| b.action.as_deref() == Some(super::app::REANALYZE))
    );
}

/// Gear toggles Settings and returns to the previous tab; the gear shows its
/// pressed state while open; Esc also returns; unsaved edits are kept.
#[test]
fn settings_gear_toggles_and_esc_returns_to_previous_tab() {
    use super::model::Tab;
    use egui::accesskit::Toggled;
    let _gpu = gpu_lock();
    let mut h = harness(demo::companies_model, SIZES[0], false);
    assert_eq!(h.state().m.tab, Tab::Companies);
    h.get_by_label("Settings").click();
    h.run();
    assert_eq!(h.state().m.tab, Tab::Settings);
    assert_eq!(
        h.get_by_label("Settings").accesskit_node().toggled(),
        Some(Toggled::True),
        "gear pressed while Settings is open"
    );
    h.get_by_label("Settings").click();
    h.run();
    assert_eq!(h.state().m.tab, Tab::Companies, "gear closes Settings");
    assert_ne!(
        h.get_by_label("Settings").accesskit_node().toggled(),
        Some(Toggled::True)
    );
    // Esc returns too; a dirty form survives the round trip.
    h.get_by_label("Settings").click();
    h.run();
    h.state_mut().m.settings.form.enrich = !h.state().m.settings.form.enrich;
    h.run();
    assert!(h.state().m.settings.dirty());
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().m.tab, Tab::Companies, "Esc closes Settings");
    assert!(h.state().m.settings.dirty(), "unsaved edits kept");
    h.get_by_label("Settings").click();
    h.run();
    assert!(
        h.get_by_label("Save changes")
            .accesskit_node()
            .label()
            .is_some()
    );
}

/// Esc in a focused Settings text field only leaves the field; the next Esc
/// closes Settings.
#[test]
fn esc_in_settings_field_first_drops_focus() {
    use super::model::Tab;
    let _gpu = gpu_lock();
    let mut h = harness(|| demo::settings_model(page::PROCESSING), SIZES[0], false);
    h.state_mut().m.prev_tab = Tab::History;
    h.get_by_label("Custom server address (optional)")
        .scroll_to_me();
    h.run();
    h.get_by_label("Custom server address (optional)").click();
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().m.tab, Tab::Settings, "first Esc leaves the field");
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().m.tab, Tab::History);
}

/// Decider settings: offline offers only AI/Off; key status copy; a typed
/// Jev key is saved to .env and referenced as ${JEV_API_KEY} (real save).
#[test]
fn decider_settings_choices_status_and_real_save() {
    use super::model::ConfigState;
    use super::pages::SettingsForm;
    use super::screens::Action;
    use super::screens::settings::{decider_choices, decider_key_status};
    assert_eq!(
        decider_choices(true),
        vec![0, 3],
        "offline: AI and Off only"
    );
    assert_eq!(decider_choices(false).len(), 4);

    let mut m = demo::settings_model(page::NAMING);
    m.settings.form.decider = 1; // openai-decisions on the OpenAI provider
    assert_eq!(
        decider_key_status(&m).map(|s| s.1),
        Some("Uses your OpenAI key".to_string())
    );
    m.settings.form.decider = 2; // jev, no key anywhere
    m.config.env.clear();
    if std::env::var("JEV_API_KEY").is_err() {
        assert_eq!(
            decider_key_status(&m).map(|s| s.1),
            Some("Add your Jev API key to use Jev.".to_string())
        );
    }
    m.settings.form.decider = 0;
    assert!(decider_key_status(&m).is_none(), "AI needs no own key");
    {
        let _gpu = gpu_lock();
        let h = harness(demo::settings_decider_offline_model, SIZES[0], false);
        h.get_by_label("Who checks similar company names?: AI on this computer");
    }

    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.yaml");
    std::fs::write(&cfg, crate::cmd::STARTER_CONFIG).unwrap();
    std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=sk-test\n").unwrap();
    let mut m = demo::settings_model(page::NAMING);
    m.demo = false;
    m.config = ConfigState::load(Some(&cfg), false);
    let form = SettingsForm::from_settings(m.config.settings.as_ref().unwrap());
    m.settings.saved = form.clone();
    m.settings.form = form;
    m.settings.form.decider = 2;
    m.settings.form.decider_key = "jev-secret".into();
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("decider: jev"), "{text}");
    assert!(
        text.contains("decider_api_key: \"${JEV_API_KEY}\""),
        "{text}"
    );
    assert!(
        !text.contains("jev-secret"),
        "key never lands in config.yaml"
    );
    let env = std::fs::read_to_string(dir.path().join(".env")).unwrap();
    use autorename_core::config_edit::get_env;
    assert_eq!(get_env(&env, "JEV_API_KEY").as_deref(), Some("jev-secret"));
    assert_eq!(get_env(&env, "OPENAI_API_KEY").as_deref(), Some("sk-test"));
    assert_eq!(
        app.m.config.decider_key_source("jev", "openai"),
        Some(super::model::KeySource::DotEnv("JEV_API_KEY".into()))
    );
}

/// OCR route statuses are derived from real state; threshold stepper edits
/// `pdf.text_quality_threshold`; unreadable-scan warning.
#[test]
fn ocr_route_status_and_threshold() {
    use super::screens::settings::{ocr_status, scans_unreadable, vision_status};
    assert!(
        ocr_status(0, false, true, false)
            .1
            .contains("scans go to AI vision")
    );
    assert!(
        ocr_status(1, false, true, false)
            .1
            .contains("scans will fail until you download")
    );
    assert_eq!(ocr_status(1, false, true, false).0, Tone::Danger);
    assert_eq!(
        ocr_status(0, false, true, true).1,
        "Auto · downloading models — scans go to AI vision until done"
    );
    assert!(
        vision_status(0, true, "Ollama", "gemma4")
            .1
            .contains("local Ollama model gemma4")
    );
    assert!(
        vision_status(0, false, "OpenAI", "gpt-6-luna")
            .1
            .contains("sends page images to OpenAI")
    );
    assert!(scans_unreadable(0, false, 2));
    assert!(scans_unreadable(2, true, 2));
    assert!(!scans_unreadable(0, true, 2));
    assert!(!scans_unreadable(0, false, 0));

    let _gpu = gpu_lock();
    let mut h = harness(demo::settings_ocr_unreadable_model, SIZES[0], false);
    h.get_by_label_contains("Scans can't be read");
    let before = h.state().m.settings.form.threshold;
    // The readability limit lives in Advanced (collapsed by default).
    assert!(
        h.query_by_label("Increase minimum text readability")
            .is_none()
    );
    h.state_mut().m.settings.advanced = true;
    h.run();
    h.get_by_label("Increase minimum text readability")
        .scroll_to_me();
    h.run();
    h.get_by_label("Increase minimum text readability").click();
    h.run();
    let after = h.state().m.settings.form.threshold;
    assert!((after - before - 0.05).abs() < 1e-9, "{before} → {after}");
    let edits = h.state().m.settings.form.edits();
    assert!(edits.iter().any(|(s, k, v)| *s == Some("pdf")
        && *k == "text_quality_threshold"
        && v == &format!("{after:.2}")));
    assert!(h.state().m.settings.dirty());
}

/// Scan languages: `auto` first, then one option per pack; a hand-written
/// code shows as its pack, an unknown one as itself; the tooltip lists
/// every language.
#[test]
fn scan_language_select_is_automatic_or_a_pack() {
    use super::screens::settings::{
        ocr_language_details, ocr_language_index, ocr_language_options,
    };
    let opts = ocr_language_options("auto");
    assert_eq!(
        opts,
        vec![
            (
                "auto".to_string(),
                "Automatic (all installed languages)".to_string()
            ),
            (
                "latin".to_string(),
                "Latin script: English, German, French + 45 more".to_string()
            ),
        ]
    );
    assert_eq!(ocr_language_index("auto"), 0);
    assert_eq!(ocr_language_index(""), 0);
    assert_eq!(ocr_language_index("de"), 1);
    assert_eq!(ocr_language_index("latin"), 1);
    assert_eq!(ocr_language_index("ja"), 2);
    assert_eq!(ocr_language_options("ja")[2], ("ja".into(), "ja".into()));
    let all = ocr_language_details("auto");
    assert!(
        all.starts_with("Uses every installed scan reader."),
        "{all}"
    );
    assert!(
        all.contains("German") && all.contains("Vietnamese"),
        "{all}"
    );
    assert!(
        ocr_language_details("de")
            .starts_with("Latin script: English, German, French, Spanish, Italian, Afrikaans")
    );

    let _gpu = gpu_lock();
    let mut h = harness(|| demo::settings_model(page::PROCESSING), SIZES[0], false);
    h.get_by_label("Scan languages: Automatic (all installed languages)")
        .scroll_to_me();
    h.run();
    h.state_mut().m.settings.form.ocr_language = "de".into();
    h.run();
    h.get_by_label("Scan languages: Latin script: English, German, French + 45 more");
    assert!(h.query_by_label_contains("Extra checks").is_none());
    assert!(h.query_by_label_contains("Logo check").is_none());
}

/// The detail pane says what the score measured: the extracted text, or
/// the text before AI image reading (not the AI's result).
#[test]
fn detail_quality_caption_names_what_was_measured() {
    use super::screens::detail::quality_caption;
    use autorename_core::extract::Tier;
    assert_eq!(
        quality_caption(Some(Tier::VisionNeeded), 0.12),
        "Text readability before AI image reading: poor (0.12)"
    );
    assert_eq!(
        quality_caption(Some(Tier::Ocr), 0.94),
        "Extracted text readability: good (0.94)"
    );
}

/// Empty screen: one optional scan-reader recommendation while the models
/// are missing; gone once installed or when downloads are blocked.
#[test]
fn empty_screen_recommends_the_scan_reader() {
    let _gpu = gpu_lock();
    let mut h = harness(demo::configured_empty_model, SIZES[0], false);
    let note = h
        .get_by_label_contains("Read scans on this computer (optional)")
        .accesskit_node()
        .label()
        .unwrap_or_default();
    assert!(
        note.ends_with(
            "Pages that need image reading are sent to OpenAI. Download the scan reader to read scans on this computer first."
        ),
        "{note}"
    );
    // The body follows the configured route: vision off, before setup.
    use super::screens::empty::ocr_hint_body;
    let mut m = demo::configured_empty_model();
    if let Some(s) = m.config.settings.as_mut() {
        s.pdf.vision = autorename_core::config::Toggle::Off;
    }
    assert!(ocr_hint_body(&m).starts_with("Scans can't be read with your current settings."));
    m.config.settings = None;
    assert_eq!(
        ocr_hint_body(&m),
        "Reads scanned text on this computer before using AI image reading."
    );
    assert_eq!(
        h.query_all_by_label_contains("Download scan reader (")
            .count(),
        1
    );
    for st in &mut h.state_mut().m.settings.packs {
        st.installed = true;
    }
    h.run();
    assert!(
        h.query_by_label_contains("Read scans on this computer")
            .is_none()
    );
    let mut h = harness(demo::configured_empty_model, SIZES[0], false);
    h.state_mut().m.force_offline = true;
    h.run();
    assert!(h.query_by_label_contains("Download scan reader").is_none());
}

/// About: license line, View license dialog, support links, imprint.
#[test]
fn about_license_dialog_and_support_links() {
    use super::screens::settings::{IMPRINT, LICENSE_LINE};
    let _gpu = gpu_lock();
    let mut h = harness(|| demo::settings_model(page::ABOUT), SIZES[0], false);
    for l in ["GitHub Sponsors", "Ko-fi", "PayPal", "PhraseVault"] {
        h.get_by_label(l);
    }
    assert!(LICENSE_LINE.contains("without warranty") && IMPRINT.contains("VAT ATU69739438"));
    assert!(!IMPRINT.contains('+'), "no phone number");
    h.get_by_label("View license").click();
    h.run();
    assert!(h.state().m.license_open);
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().m.license_open, "Esc closes the dialog");
    assert_eq!(
        h.state().m.tab,
        super::model::Tab::Settings,
        "…not Settings"
    );
}

/// Temp config (starter + `extra` appended / `profiles` replaced) loaded
/// into a non-demo Settings model on `section`.
fn temp_settings(profiles: &str, section: usize) -> (tempfile::TempDir, Model) {
    use super::model::ConfigState;
    use super::pages::SettingsForm;
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.yaml");
    let text = crate::cmd::STARTER_CONFIG.replace("profiles: {}", profiles);
    std::fs::write(&cfg, text).unwrap();
    std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=sk-test\n").unwrap();
    let mut m = demo::settings_model(section);
    m.demo = false;
    m.config = ConfigState::load(Some(&cfg), false);
    let form = SettingsForm::from_settings(m.config.settings.as_ref().expect("valid config"));
    m.settings.saved = form.clone();
    m.settings.form = form;
    m.companies = Default::default();
    (dir, m)
}

/// Company name, date format, max pages, language and multi-line extra
/// instructions become config edits and survive a real save + reload;
/// untouched lines (comments, `model: null`) stay byte-identical.
#[test]
fn naming_and_reading_edits_round_trip_real_save() {
    use super::pages::FormField;
    use super::screens::Action;
    let (dir, mut m) = temp_settings("profiles: {}", page::NAMING);
    let f = &mut m.settings.form;
    f.company = "1234".into();
    f.date_format = "%Y-%m-%d".into();
    f.max_pages = 5;
    f.out_language = "German".into();
    f.incoming = "ER".into();
    f.prompt_extension = "Use the \"brand\" name.\nNever # translate.".into();
    assert!(f.problem(FormField::DateFormat).is_none());
    assert!(f.save_blocker().is_none());
    let changed = f.changed_edits(&m.settings.saved);
    assert!(changed.contains(&(Some("company"), "name", "\"1234\"".into())));
    assert!(changed.contains(&(Some("pdf"), "max_pages", "5".into())));
    assert!(
        !changed
            .iter()
            .any(|e| e.1 == "model" || e.1 == "temperature"),
        "untouched keys are not rewritten: {changed:?}"
    );
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let text = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
    assert!(text.contains("  name: \"1234\""), "{text}");
    assert!(text.contains("  max_pages: 5"), "{text}");
    assert!(text.contains("  date_format: \"%Y-%m-%d\""), "{text}");
    assert!(text.contains("  model: null                   # null = provider default"));
    assert_eq!(
        text.lines().count(),
        crate::cmd::STARTER_CONFIG.lines().count(),
        "one line per key, nothing appended"
    );
    let s = app.m.config.settings.as_ref().expect("reloaded");
    assert_eq!(s.company_name, "1234");
    assert_eq!(s.date_format, "%Y-%m-%d");
    assert_eq!(s.pdf.max_pages, 5);
    assert_eq!(s.pdf.incoming_invoice, "ER");
    assert_eq!(s.output_language, "German");
    assert_eq!(
        s.prompt_extension,
        "Use the \"brand\" name.\nNever # translate."
    );
    assert!(!app.m.settings.dirty(), "form reloaded from the saved file");
}

/// Invalid name format input shows field helpers and blocks Save (button,
/// Ctrl+Enter and the action).
#[test]
fn invalid_name_format_blocks_save() {
    use super::pages::FormField;
    use super::screens::Action;
    let m = demo::settings_naming_invalid_model();
    let f = &m.settings.form;
    assert_eq!(
        f.problem(FormField::DateFormat).as_deref(),
        Some(super::pages::DATE_FORBIDDEN_HINT)
    );
    assert!(f.problem(FormField::Outgoing).is_some(), "same code twice");
    assert_eq!(f.save_blocker().as_deref(), Some("Fix 2 problems to save"));
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.dirty(), "not saved");

    let _gpu = gpu_lock();
    let mut h = harness(demo::settings_naming_invalid_model, SIZES[0], false);
    assert!(
        h.get_by_label("Save changes")
            .accesskit_node()
            .is_disabled(),
        "Save is disabled"
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().m.settings.dirty(), "Ctrl+Enter does not save");
}

/// The name-matching tab is labelled by the active profile's field and
/// hidden (Ctrl+2 a no-op) for a profile without one; the Naming page
/// follows the selected profile.
#[test]
fn name_tab_follows_profile() {
    use super::model::Tab;
    use super::screens::Action;
    let _gpu = gpu_lock();
    let mut h = harness(demo::configured_empty_model, SIZES[0], false);
    h.get_by_role_and_label(Role::Tab, "Companies, 3 need review");
    let ctx = h.ctx.clone();
    h.state_mut()
        .act(&ctx, Action::SelectProfile("academic".into()));
    h.run();
    assert!(
        h.query_by_label_contains("Companies").is_none(),
        "no name matching, no tab"
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num2);
    h.run();
    assert_eq!(h.state().m.tab, Tab::Rename, "Ctrl+2 does nothing");

    // A profile matching journal names: "Venues", venue copy.
    let mut m = demo::companies_model();
    let mut tree = m.config.tree.clone().unwrap();
    tree["profiles"] = serde_json::json!({"academic": {"harmonize_field": "journal_name"}});
    m.config.tree = Some(tree);
    m.config.select_profile("academic");
    if let Some(db) = m.companies.db.as_mut() {
        db.field = "journal_name".into();
    }
    let mut h = Harness::builder()
        .with_size(theme::v2(SIZES[0].0, SIZES[0].1))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            set_reduce_motion(&cc.egui_ctx, true);
            App::with_model(m, None)
        });
    h.run();
    h.get_by_label_contains("Venues");
    h.get_by_label("Keep as new venue");
    h.get_by_label("Search venues");
}

/// Real disk: a profile switch reloads the name list at once (also while
/// the tab is open); a profile without a list leaves the tab.
#[test]
fn profile_switch_reloads_name_list() {
    use super::model::Tab;
    use super::screens::Action;
    use autorename_core::harmonize;
    let (dir, mut m) = temp_settings(
        "profiles:\n  papers:\n    extends: academic\n    harmonize_field: journal_name\n",
        page::GENERAL,
    );
    let lists = dir.path().join("name-lists");
    std::fs::create_dir_all(&lists).unwrap();
    for (field, name) in [("company_name", "Amazon"), ("journal_name", "NeurIPS")] {
        let mut db = harmonize::NameDb::empty(field);
        harmonize::add_user(&mut db, name, &[]).unwrap();
        harmonize::save_db(&db, &lists.join(format!("{field}.yaml"))).unwrap();
    }
    m.tab = Tab::Companies;
    let _gpu = gpu_lock();
    let mut h = Harness::builder()
        .with_size(theme::v2(SIZES[0].0, SIZES[0].1))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            set_reduce_motion(&cc.egui_ctx, true);
            App::with_model(m, None)
        });
    h.run();
    run_until(&mut h, "Amazon list", |a| names(a) == ["Amazon"]);
    let ctx = h.ctx.clone();
    h.state_mut()
        .act(&ctx, Action::SelectProfile("papers".into()));
    assert!(
        h.state().m.companies.loading && h.state().m.companies.db.is_none(),
        "the new list loads on the worker; the old one is gone at once"
    );
    run_until(&mut h, "NeurIPS list", |a| names(a) == ["NeurIPS"]);
    h.get_by_role_and_label(Role::Tab, "Venues");
    h.state_mut()
        .act(&ctx, Action::SelectProfile("academic".into()));
    h.run();
    assert_eq!(h.state().m.tab, Tab::Rename, "tab gone, back to Rename");
    assert!(h.state().m.companies.db.is_none());
}

fn names(a: &App) -> Vec<String> {
    a.m.companies
        .entries()
        .iter()
        .map(|e| e.canonical.clone())
        .collect()
}

/// Run frames until `done` (worker results arrive between frames).
fn run_until(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    let start = std::time::Instant::now();
    while !done(h.state()) {
        assert!(
            start.elapsed().as_secs() < 10,
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run();
    }
}

/// Naming: the sample names use the form's date/amount format and codes;
/// a profile without harmonize_field says so instead of showing controls.
#[test]
fn naming_samples_and_no_matching_caption() {
    use super::screens::settings::sample_name;
    let m = demo::settings_model(page::NAMING);
    let tree = m.config.tree.clone().unwrap();
    let profiles = autorename_core::profiles::resolve_profiles(&tree).unwrap();
    let mut f = m.settings.form.clone();
    assert_eq!(
        sample_name("business", &profiles["business"], &f),
        "20251103 Amazon AP EUR 12,13.pdf"
    );
    f.date_format = "%Y-%m-%d".into();
    f.amount_format = "{amount} {currency}".into();
    f.incoming = "ER".into();
    assert_eq!(
        sample_name("business", &profiles["business"], &f),
        "2025-11-03 Amazon ER 12,13 EUR.pdf"
    );

    let _gpu = gpu_lock();
    let h = harness(demo::settings_naming_academic_model, SIZES[0], false);
    h.get_by_label_contains("Academic profile, for example");
    assert!(h.query_by_label_contains("Learn new").is_none());
    let h = harness(|| demo::settings_model(page::NAMING), SIZES[0], false);
    h.get_by_label("Learn new companies automatically");
    h.get_by_label("Who checks similar company names?: Your selected AI provider");
}

/// The privacy chip opens Settings › Processing at the top.
#[test]
fn privacy_chip_opens_processing() {
    use super::model::Tab;
    let _gpu = gpu_lock();
    let mut h = harness(demo::preview_model, SIZES[0], false);
    h.get_by_label("Privacy: cloud, OpenAI gpt-6-luna. Open privacy settings")
        .click();
    h.run();
    assert_eq!(h.state().m.tab, Tab::Settings);
    assert_eq!(h.state().m.settings.section, page::PROCESSING);
    h.get_by_label("Privacy mode");
}

// ---------------------------------------------------------------------------
// Review regressions: credentials, quoting, name-list worker
// ---------------------------------------------------------------------------

/// Temp settings with `env` as `.env` (config reloaded so the GUI sees it).
fn temp_settings_env(env: &str, section: usize) -> (tempfile::TempDir, Model) {
    use super::model::ConfigState;
    use super::pages::SettingsForm;
    let (dir, mut m) = temp_settings("profiles: {}", section);
    std::fs::write(dir.path().join(".env"), env).unwrap();
    m.config = ConfigState::load(Some(&dir.path().join("config.yaml")), false);
    let form = SettingsForm::from_settings(m.config.settings.as_ref().expect("valid config"));
    m.settings.saved = form.clone();
    m.settings.form = form;
    (dir, m)
}

/// A provider switch rebinds `ai.api_key` to the new provider's variable:
/// status line, Test connection and Save all use the new provider's key,
/// never the previous one. Without that key Save is blocked.
#[test]
fn provider_switch_rebinds_api_key() {
    use super::model::KeySource;
    use super::pages::{Credential, KeyState};
    use super::screens::Action;
    let (dir, mut m) = temp_settings_env(
        "OPENAI_API_KEY=sk-openai-secret\nANTHROPIC_API_KEY=sk-ant-secret\n",
        page::PROCESSING,
    );
    m.settings.form.provider = 1; // Anthropic, nothing typed
    let (f, saved) = (&m.settings.form, &m.settings.saved);
    assert_eq!(f.ai_credential(saved), Credential::Env("ANTHROPIC_API_KEY"));
    assert_eq!(
        f.ai_key_state(saved, &m.config),
        Some(KeyState::Found(KeySource::DotEnv(
            "ANTHROPIC_API_KEY".into()
        )))
    );
    assert!(f.changed_edits(saved).contains(&(
        Some("ai"),
        "api_key",
        "\"${ANTHROPIC_API_KEY}\"".into()
    )));
    let (mut app, ctx) = bare_app(m);
    let st = app.form_settings().expect("test settings");
    assert_eq!(
        st.ai.api_key, "sk-ant-secret",
        "Test connection uses the new key"
    );
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let text = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
    assert!(text.contains("api_key: \"${ANTHROPIC_API_KEY}\""), "{text}");
    let s = app.m.config.settings.as_ref().unwrap();
    assert_eq!(s.ai.provider.id(), "anthropic");
    assert_eq!(s.ai.api_key, "sk-ant-secret");

    // No key for the new provider: status says so and Save is blocked.
    let (dir, mut m) = temp_settings_env("OPENAI_API_KEY=sk-openai-secret\n", page::PROCESSING);
    m.settings.form.provider = 2; // Gemini
    if std::env::var("GEMINI_API_KEY").is_err() {
        assert_eq!(
            m.settings.blocker(&m.config).as_deref(),
            Some("Add your Gemini API key to save")
        );
        let before = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
        let (mut app, ctx) = bare_app(m);
        let st = app.form_settings().expect("test settings");
        assert_eq!(st.ai.api_key, "", "never the OpenAI key");
        app.act(&ctx, Action::SaveSettings);
        assert!(app.m.settings.error.is_some());
        let after = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
        assert_eq!(before, after, "nothing written");
    }

    // Ollama needs no key: the cloud reference is cleared.
    let (_dir, mut m) = temp_settings_env("OPENAI_API_KEY=sk-openai-secret\n", page::PROCESSING);
    m.settings.form.provider = 4;
    let f = &m.settings.form;
    assert_eq!(f.ai_credential(&m.settings.saved), Credential::Clear);
    assert!(
        f.changed_edits(&m.settings.saved)
            .contains(&(Some("ai"), "api_key", "\"\"".into()))
    );
    let (app, _) = bare_app(m);
    assert_eq!(app.form_settings().unwrap().ai.api_key, "");
}

/// A provider switch never keeps the previous provider's server address
/// (it would get the new provider's requests and key): the field is
/// cleared, the saved one returns when switching back, and an address typed
/// after the switch is saved.
#[test]
fn provider_switch_clears_the_old_server_address() {
    use super::model::ConfigState;
    use super::pages::SettingsForm;
    use super::screens::Action;
    use autorename_core::config_edit::{quoted, set};
    let (dir, mut m) = temp_settings_env(
        "OPENAI_API_KEY=sk-openai-secret\nANTHROPIC_API_KEY=sk-ant-secret\n",
        page::PROCESSING,
    );
    let cfg = dir.path().join("config.yaml");
    let text = std::fs::read_to_string(&cfg).unwrap();
    let url = "https://proxy.example/v1";
    std::fs::write(&cfg, set(&text, Some("ai"), "base_url", &quoted(url))).unwrap();
    m.config = ConfigState::load(Some(&cfg), false);
    let form = SettingsForm::from_settings(m.config.settings.as_ref().expect("valid config"));
    assert_eq!(form.base_url, url);
    m.settings.saved = form.clone();
    m.settings.form = form;

    let saved = m.settings.saved.clone();
    let f = &mut m.settings.form;
    f.provider = 1; // Anthropic
    f.provider_changed(&saved);
    assert_eq!(f.base_url, "");
    assert!(
        f.changed_edits(&saved)
            .contains(&(Some("ai"), "base_url", "\"\"".into()))
    );
    f.provider = 0; // back to OpenAI: its saved address returns
    f.provider_changed(&saved);
    assert_eq!(f.base_url, url);
    assert!(!f.changed_edits(&saved).iter().any(|e| e.1 == "base_url"));
    f.provider = 1;
    f.provider_changed(&saved);
    f.base_url = "https://gw.example/v1".into(); // typed after the switch
    assert!(f.changed_edits(&saved).contains(&(
        Some("ai"),
        "base_url",
        "\"https://gw.example/v1\"".into()
    )));
    f.base_url.clear();
    let (mut app, ctx) = bare_app(m);
    assert_eq!(
        app.form_settings().unwrap().ai.base_url,
        "",
        "Test connection"
    );
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let s = app.m.config.settings.as_ref().unwrap();
    assert_eq!(
        (s.ai.provider.id(), s.ai.base_url.as_str()),
        ("anthropic", "")
    );
}

/// Save writes `.env` only when dotenvy reads the final text back: a
/// multi-line value elsewhere stays intact, a file that would not parse is
/// refused with nothing written.
#[test]
fn save_validates_the_final_env_text() {
    use super::screens::Action;
    use autorename_core::config_edit::get_env;
    let env = "CERT=\"line one\nOPENAI_API_KEY=inside\"\nOPENAI_API_KEY=\"old\nkey\"\nOTHER=ok\n";
    let (dir, mut m) = temp_settings_env(env, page::PROCESSING);
    m.settings.form.api_key = "sk-new".into();
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let after = std::fs::read_to_string(dir.path().join(".env")).unwrap();
    assert_eq!(
        after,
        "CERT=\"line one\nOPENAI_API_KEY=inside\"\nOPENAI_API_KEY=\"sk-new\"\nOTHER=ok\n"
    );
    assert_eq!(get_env(&after, "OTHER").as_deref(), Some("ok"));
    assert_eq!(app.m.config.settings.as_ref().unwrap().ai.api_key, "sk-new");

    // An unterminated quote swallows everything after it: refused.
    let broken = "OPENAI_API_KEY=sk-old\nNOTE=\"open\n";
    let (dir, mut m) = temp_settings_env(broken, page::PROCESSING);
    m.settings.form.api_key = "sk-new".into();
    let config = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    let err = app.m.settings.error.clone().unwrap_or_default();
    assert!(err.starts_with(".env "), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).unwrap(),
        broken
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.yaml")).unwrap(),
        config
    );
}

/// A decider switch rebinds (or clears) `decider_api_key` the same way.
#[test]
fn decider_switch_rebinds_decider_key() {
    use super::pages::Credential;
    use super::screens::Action;
    let (dir, mut m) = temp_settings_env(
        "OPENAI_API_KEY=sk-openai-secret\nJEV_API_KEY=jev-secret\n",
        page::NAMING,
    );
    m.settings.form.decider = 2; // jev
    assert_eq!(
        m.settings.form.decider_credential(&m.settings.saved),
        Credential::Env("JEV_API_KEY")
    );
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let text = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
    assert!(
        text.contains("decider_api_key: \"${JEV_API_KEY}\""),
        "{text}"
    );
    assert_eq!(
        app.m.config.settings.as_ref().unwrap().decider_api_key,
        "jev-secret"
    );
    // Jev → OpenAI Decisions on the OpenAI provider: uses the main key,
    // so the Jev reference is cleared.
    let mut m = app.m;
    m.settings.form.decider = 1;
    let f = &m.settings.form;
    assert_eq!(f.decider_credential(&m.settings.saved), Credential::Clear);
    assert!(f.changed_edits(&m.settings.saved).contains(&(
        Some("harmonization"),
        "decider_api_key",
        "\"\"".into()
    )));
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    assert_eq!(
        app.m.config.settings.as_ref().unwrap().decider_api_key,
        "sk-openai-secret",
        "falls back to ai.api_key"
    );
    // Then a provider switch: OpenAI Decisions now needs OPENAI_API_KEY.
    let mut m = app.m;
    m.settings.form.provider = 1;
    assert_eq!(
        m.settings.form.decider_credential(&m.settings.saved),
        Credential::Env("OPENAI_API_KEY")
    );
}

/// Free-text values are always YAML strings: a model or profile id that
/// looks like a number or bool still saves and loads as that string.
#[test]
fn number_like_model_and_profile_stay_strings() {
    use super::screens::Action;
    let (dir, mut m) = temp_settings(
        "profiles:\n  \"2024\":\n    extends: business\n",
        page::PROCESSING,
    );
    m.settings.form.model = "1234".into();
    m.settings.form.profile = "2024".into();
    let changed = m.settings.form.changed_edits(&m.settings.saved);
    assert!(changed.contains(&(Some("ai"), "model", "\"1234\"".into())));
    assert!(changed.contains(&(None, "profile", "\"2024\"".into())));
    let (mut app, ctx) = bare_app(m);
    app.act(&ctx, Action::SaveSettings);
    assert!(app.m.settings.error.is_none(), "{:?}", app.m.settings.error);
    let s = app.m.config.settings.as_ref().unwrap();
    assert_eq!(s.ai.model, "1234");
    assert_eq!(s.profile_id, "2024");
    let text = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
    assert!(text.contains("  model: \"1234\""), "{text}");
    // Empty model = provider default (bare null).
    let mut m = app.m;
    m.settings.form.model = " ".into();
    assert!(m.settings.form.changed_edits(&m.settings.saved).contains(&(
        Some("ai"),
        "model",
        "null".into()
    )));
}

/// GUI Rename changes an entry's spelling (`harmonize::rename`) and never
/// merges into another entry; Merge stays the explicit way to combine.
#[test]
fn rename_is_a_spelling_change_not_a_merge() {
    use super::app::rename_entry;
    use autorename_core::harmonize::{self, Source};
    let mut db = harmonize::parse_db(
        "ACME:\n  source: auto\n  learned: [ACME GmbH]\nGlobex:\n  source: auto\n",
        harmonize::COMPANY_FIELD,
    )
    .unwrap();
    rename_entry(&mut db, "ACME", "Acme").expect("case-only rename");
    assert_eq!(db.entries[0].canonical, "Acme");
    assert_eq!(db.entries[0].learned, ["ACME GmbH"], "aliases stay");
    assert_eq!(db.entries[0].source, Source::User);
    let before = db.clone();
    let e = rename_entry(&mut db, "Acme", "Globex").unwrap_err();
    assert!(e.to_string().contains("Merge into"), "{e}");
    assert_eq!(db, before, "no silent merge");
    rename_entry(&mut db, "Acme", "Acme Industries").unwrap();
    assert_eq!(
        db.entries[0].aliases,
        ["Acme"],
        "old spelling kept as alias"
    );
    assert_eq!(db.entries.len(), 2);
}

const REVIEW_LIST: &str = "ACME:\n  source: auto\n  review: true\nGlobex:\n  source: auto\n";

/// Harness on the Companies tab of a temp config whose list is `list`.
fn companies_harness(list: &str) -> (tempfile::TempDir, std::path::PathBuf, Harness<'static, App>) {
    use super::model::Tab;
    let (dir, mut m) = temp_settings("profiles: {}", page::GENERAL);
    let lists = dir.path().join("name-lists");
    std::fs::create_dir_all(&lists).unwrap();
    let path = lists.join("company_name.yaml");
    std::fs::write(&path, list).unwrap();
    m.tab = Tab::Companies;
    let h = Harness::builder()
        .with_size(theme::v2(SIZES[0].0, SIZES[0].1))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            set_reduce_motion(&cc.egui_ctx, true);
            App::with_model(m, None)
        });
    (dir, path, h)
}

/// Name-list edits run on the worker: a held `.lock` (another process
/// writing) never freezes the window; edits are disabled until it is done.
#[test]
fn name_list_edit_waits_on_worker_not_ui() {
    use super::screens::Action;
    let _gpu = gpu_lock();
    let (_dir, path, mut h) = companies_harness(REVIEW_LIST);
    h.run();
    run_until(&mut h, "list", |a| names(a) == ["ACME", "Globex"]);
    let lock = autorename_core::fsx::lock_sibling(&path).expect("lock");
    let ctx = h.ctx.clone();
    let t = std::time::Instant::now();
    h.state_mut().act(&ctx, Action::KeepCompany("ACME".into()));
    assert!(t.elapsed().as_millis() < 500, "the action returns at once");
    assert!(h.state().m.companies.pending);
    h.run();
    assert!(
        h.get_by_label("Merge into…").accesskit_node().is_disabled(),
        "conflicting edits are disabled while one runs"
    );
    // A second edit while pending is ignored (one at a time).
    h.state_mut()
        .act(&ctx, Action::RemoveAlias("ACME".into(), "x".into()));
    std::thread::sleep(std::time::Duration::from_millis(100));
    h.run();
    assert!(h.state().m.companies.pending, "still waiting for the lock");
    drop(lock);
    run_until(&mut h, "edit", |a| !a.m.companies.pending);
    let db = autorename_core::harmonize::load_db(&path, "company_name").unwrap();
    assert!(!db.entries[0].review, "kept: saved to disk");
    assert!(!h.state().m.companies.entries()[0].review, "and shown");
    h.state_mut().m.companies.active = Some(1); // Globex (ACME is now hand-kept)
    h.run();
    assert!(!h.get_by_label("Merge into…").accesskit_node().is_disabled());
}

/// The shown list follows the file: reloaded after apply/undo, on tab
/// entry and when its mtime changed; the selected entry stays selected.
#[test]
fn name_list_reloads_after_undo_tab_entry_and_external_edit() {
    use super::model::Tab;
    use super::screens::Action;
    use super::worker::Msg;
    use autorename_core::pipeline::ErrorInfo;
    let _gpu = gpu_lock();
    let (_dir, path, mut h) = companies_harness(REVIEW_LIST);
    h.run();
    run_until(&mut h, "list", |a| names(a) == ["ACME", "Globex"]);
    h.state_mut().m.companies.active = Some(1); // Globex
    // Keep the mtime check quiet so each trigger is tested on its own.
    let quiet = || Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
    h.state_mut().m.companies.checked = quiet();

    // Undo (another batch) finished → reload, even with no visible change.
    std::fs::write(&path, format!("Aaron:\n  source: auto\n{REVIEW_LIST}")).unwrap();
    let err = ErrorInfo::from_error(&autorename_core::Error::General("x".into()));
    h.state().worker_send(Msg::Undone("b".into(), Err(err)));
    run_until(&mut h, "reload after undo", |a| names(a).len() == 3);
    assert_eq!(
        h.state().m.companies.active,
        Some(2),
        "Globex stays selected at its new index"
    );

    // Tab entry.
    let ctx = h.ctx.clone();
    h.state_mut().act(&ctx, Action::SwitchTab(Tab::Rename));
    h.run();
    std::fs::write(&path, format!("Zed:\n  source: auto\n{REVIEW_LIST}")).unwrap();
    h.state_mut().m.companies.checked = quiet();
    h.state_mut().act(&ctx, Action::SwitchTab(Tab::Companies));
    run_until(&mut h, "reload on tab entry", |a| names(a)[0] == "Zed");

    // External edit while the tab is open: mtime check.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, format!("Yan:\n  source: auto\n{REVIEW_LIST}")).unwrap();
    h.state_mut().m.companies.checked = None;
    run_until(&mut h, "reload on mtime change", |a| names(a)[0] == "Yan");
    assert_eq!(h.state().m.config.review_count, 1);
}

/// Privacy shows the saved mode while a switch is pending; the Naming page
/// shows only what the profile uses; key status leads with the outcome.
#[test]
fn settings_copy_follows_state() {
    use super::model::KeySource;
    use super::pages::KeyState;
    use super::screens::settings::{Uses, instructions_example, key_status, privacy_text};
    assert!(privacy_text(true, true).starts_with("After saving, text"));
    assert!(privacy_text(true, false).starts_with("Offline: text"));
    let m = demo::settings_model(page::NAMING);
    let tree = m.config.tree.clone().unwrap();
    let ps = autorename_core::profiles::resolve_profiles(&tree).unwrap();
    assert_eq!(Uses::of(&tree, "business"), Some(Uses::ALL));
    assert_eq!(
        Uses::of(&tree, "academic"),
        Some(Uses {
            direction: false,
            amount: false,
            language: false
        })
    );
    assert!(instructions_example(Some(&ps["academic"])).contains("paper titles"));
    let (tone, text, details) = key_status(
        &KeyState::Found(KeySource::DotEnv("OPENAI_API_KEY".into())),
        "OpenAI",
        "OpenAI",
    );
    assert_eq!((tone, text.as_str()), (Tone::Success, "API key saved"));
    assert!(
        details.contains("OPENAI_API_KEY"),
        "storage only in the tooltip"
    );

    let _gpu = gpu_lock();
    let h = harness(demo::settings_naming_academic_model, SIZES[0], false);
    assert!(h.query_by_label("Code for bills you receive").is_none());
    assert!(h.query_by_label_contains("Amount format").is_none());
    assert!(h.query_by_label("Your company name").is_none());
    h.get_by_label_contains("Date format");
    let h = harness(demo::settings_decider_jev_model, SIZES[0], false);
    if std::env::var("JEV_API_KEY").is_err() {
        assert!(
            h.get_by_label("Save changes")
                .accesskit_node()
                .is_disabled(),
            "a selected cloud decider without a key blocks Save"
        );
    }
}
