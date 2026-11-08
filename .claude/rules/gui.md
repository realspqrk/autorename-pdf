---
paths:
  - "crates/app/src/gui/**"
  - "crates/app/assets/**"
---

# GUI

egui/eframe 0.37 in `crates/app/src/gui/` (feature `gui`, default on).

- Spec: `docs/superpowers/specs/2026-11-07-gui-design.md` is authoritative (tokens, screens, components, keyboard map, §9 checklist).
- Tokens only: colors, px sizes, radii, type and motion come from `gui/theme.rs` (`space::`, `size::`, `typ::`, `radius::`, `motion::`, `Palette`). No `Color32::from_rgb` or numeric `vec2(` elsewhere. Layout on the 4-pt grid.
- Widgets: use `gui/components/` (Button, Badge, Select, Segmented, Toggle, Field, Banner, Progress, PlanRow, DropZone, Dialog). egui defaults only for ScrollArea, popup lists, TextEdit internals, tooltips.
- Every interactive widget reports `widget_info` (AccessKit label + role); kittest fails on unlabeled controls.
- Manual geometry: place with `ui.interact`, `show_at(ui, rect)` or `components::in_rect`, never `ui.put`/`allocate_rect` (they move the parent cursor).
- Core only through `autorename_core` (`pipeline::plan_files`/`apply_plan`, `journal`, `harmonize`, `config`, `ocr::models`). Long work runs on `gui/worker.rs` threads with results over the channel. Never block the UI thread.
- Config edits go through `autorename_core::config_edit` (comment- and byte-preserving; free text via `quoted`); API keys go into `.env` (values encoded for dotenvy via `set_env`), the config references `${VAR}`. Name-list loads/edits run on `gui/worker.rs`.
- Demo data: `gui/demo.rs` (`AUTORENAME_GUI_DEMO=1`), no disk or network.
- Tests: `cargo test -p autorename-app --bin autorename-pdf gui::`. Snapshots in `crates/app/tests/snapshots/gui/`; refresh with `UPDATE_SNAPSHOTS=force` and look at every changed PNG before committing. Duplicate-id panics ("No glyph for 🔥") mean an id clash: salt the id.
