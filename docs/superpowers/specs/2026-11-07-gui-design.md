# AutoRename-PDF 4.0 GUI: design system and screen spec

Date 2026-11-07 · Target egui/eframe 0.37 (`crates/app`, feature `gui`) · Parent spec: `2026-11-07-rust-rebuild.md` · Status: draft for review gate (phase 6)

Clean break from v3. Nothing from the Tauri GUI (`gui/src/`, screenshot) carries over except the brand mark (red rounded square + page glyph). Every number below is a token. Implementations must not use literal px/colors outside `theme.rs`.

**Sources** (cited by tag): [WCAG] https://www.w3.org/TR/WCAG22/ (1.4.3 https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html, 1.4.11 https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html, 2.4.7/2.4.11 https://www.w3.org/WAI/WCAG22/Understanding/focus-not-obscured-minimum.html, 2.4.13 https://www.w3.org/WAI/WCAG22/Understanding/focus-appearance.html, 2.5.8 https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html, 2.3.3, 2.2.1) · [M3] Material 3 https://m3.material.io/foundations/layout/understanding-layout/spacing, https://m3.material.io/styles/elevation/overview, https://m3.material.io/styles/motion/easing-and-duration · [HIG] Apple https://developer.apple.com/design/human-interface-guidelines/accessibility, https://developer.apple.com/design/human-interface-guidelines/app-icons · [F2] Fluent 2 https://fluent2.microsoft.design/layout, https://fluent2.microsoft.design/typography, https://fluent2.microsoft.design/elevation, https://learn.microsoft.com/windows/apps/design/style/iconography/app-icon-construction · [RUI] Refactoring UI https://www.refactoringui.com/ · [TS] Tim Brown, More Meaningful Typography https://alistapart.com/article/more-meaningful-typography/ ; Butterick line spacing/length https://practicaltypography.com/line-spacing.html, https://practicaltypography.com/line-length.html · [OK] Ottosson, Oklab https://bottosson.github.io/posts/oklab/ · [FD] freedesktop icon theme spec https://specifications.freedesktop.org/icon-theme-spec/latest/ · [EGUI] https://docs.rs/egui/0.37.0/egui/style/struct.Style.html

## 1. Principles
1. **Plan first, act second.** The screen always shows what *will* happen (old → new) before anything is written; one primary action per screen [RUI].
2. **Calm density.** Desktop productivity density (Fluent "medium"): 32 px controls, 8-pt grid, no decorative chrome. Hierarchy by weight and color, not size alone [RUI].
3. **Privacy is visible.** The header always states where document text goes (cloud provider + model, or Offline).
4. **Everything reversible is labeled reversible.** Undo is a first-class, always-reachable action.
5. **Keyboard complete, AccessKit complete.** Every action has a focusable control and a label; mouse is optional.

## 2. Tokens (`crates/app/src/gui/theme.rs`)
Naming rule: doc token `group.name` ↔ Rust `theme::group::NAME` (const `f32`/`u8`) or `Palette.field` (`Color32`). Example `space.md` → `theme::space::MD`, `color.text_muted` → `palette.text_muted`.

### 2.1 Spacing (4-pt base, 8-pt rhythm) [M3][F2]
| token | px | use |
|---|---|---|
| space.xxs | 2 | focus-ring gap, segmented inset |
| space.xs | 4 | icon↔badge, badge gaps, label↔helper |
| space.sm | 8 | default `item_spacing`, icon↔label, row inner vertical |
| space.md | 12 | control horizontal padding, list column gap |
| space.lg | 16 | pane padding, row horizontal padding, header padding |
| space.xl | 24 | page margin, dialog padding, section gap |
| space.xxl | 32 | between settings groups, empty-state blocks |
| space.xxxl | 48 | onboarding vertical rhythm, header height |
Rule: all widget sizes, offsets and pane sizes are multiples of 4; layout distances (margins, gaps between groups) multiples of 8. 2 px is only for strokes/insets.

### 2.2 Size / hit targets [WCAG 2.5.8][HIG][F2]
`size.control = 32` (button, field, segmented, row-action hit area; preferred desktop) · `size.control_sm = 24` (in-row icon buttons, chips that act; WCAG AA floor) · `size.control_lg = 40` (onboarding CTA) · `size.icon = 16`, `size.icon_sm = 12`, `size.icon_lg = 24`, `size.icon_xl = 48` (empty state) · `size.row = 56` · `size.header = 48` · `size.toolbar = 40` · `size.action_bar = 56`. Visual size may be smaller than hit size (checkbox 16 visual in 32 hit); hit rects never overlap; 24 px targets keep ≥ 8 px spacing to neighbours.

### 2.3 Type scale: ratio 1.2 (minor third), base 14 px [TS][F2]
Ratio 1.2 is the densest scale that still gives distinguishable steps (≥ 2.4 px per step at body size); 1.25+ produces oversized headings in a 48 px header; 1.125 collapses body vs title. Sizes = 14 × 1.2ⁿ rounded to whole px. Line heights snap to the 4-pt grid; ratio falls from ~1.43 (body) to ~1.24 (display) per [TS]/Butterick (larger text needs less leading).
| token | n | size | line | weight | font | use |
|---|---|---|---|---|---|---|
| type.display | +4 | 29 | 36 | 600 | Inter SemiBold | onboarding hero only |
| type.headline | +3 | 24 | 32 | 600 | Inter SemiBold | Done summary number, dialog-free page titles |
| type.title | +2 | 20 | 28 | 600 | Inter SemiBold | page/section title, dialog title (`TextStyle::Heading`) |
| type.title_sm | +1 | 17 | 24 | 500 | Inter Medium | pane titles, detail filename |
| type.body | 0 | 14 | 20 | 400 | Inter Regular | default (`TextStyle::Body`) |
| type.body_strong | 0 | 14 | 20 | 500 | Inter Medium | proposed name, buttons (`TextStyle::Button`), labels |
| type.caption | −1 | 12 | 16 | 400 | Inter Regular | helper, metadata, timestamps (`TextStyle::Small`) |
| type.caption_strong | −1 | 12 | 16 | 500 | Inter Medium | badges, column headers |
| type.mono | — | 13 | 20 | 400 | JetBrains Mono | filenames (old/new), paths, extracted text (`TextStyle::Monospace`) |
| type.mono_sm | — | 12 | 16 | 400 | JetBrains Mono | original filename under proposed name |
Weight hierarchy: exactly three weights (400/500/600); never 700+, never italics for emphasis. Max measure for prose 560 px (≈ 70 ch at 14 px, Butterick 45–90). Mono for filenames because `YYYYMMDD` prefixes then align vertically across rows (egui has no OpenType `tnum`). Line height is applied via `TextFormat.line_height = Some(line)` in a `LayoutJob` helper `theme::text(job_style)`; plain `Label`s rely on font metrics (acceptable deviation ≤ 2 px).

### 2.4 Fonts (bundled via `include_bytes!`, `FontData::from_static`)
- **Inter 4.1** (SIL OFL 1.1, https://rsms.me/inter/): designed for screens, tall x-height (0.55 em) reads well unhinted (egui rasterises without hinting), disambiguated `Il1`, wide Latin/Greek/Cyrillic coverage. Static Regular/Medium/SemiBold (egui 0.37 cannot select variable-font axes), registered as `FontFamily::Proportional` (Regular) plus `FontFamily::Name("inter-500")`, `Name("inter-600")`.
- **JetBrains Mono 2.304** (OFL 1.1, https://www.jetbrains.com/lp/mono/): high x-height matches Inter, clear `0O`/`1l`, distinct `,` vs `.` (important: names contain `12,13`).
- **Lucide icon font** (ISC, https://lucide.dev/license) as `Name("icons")`, see §6.
- Subset all with `pyftsubset` to Latin-1, Latin Ext-A/B, Greek, Cyrillic, General Punctuation, €, arrows; ship licenses in `crates/app/assets/fonts/LICENSES/`. Budget ≤ 900 KB total. Fallback chain per family: Inter → egui default (`Ubuntu-Light`, emoji) so unsupported scripts in filenames render, not tofu. CJK system fallback = stretch.

### 2.5 Color (OKLCH-authored, sRGB hex, neutral hue 260, accent hue 264) [OK]
Accent is **blue, not brand red**: red is reserved for errors/destructive actions so semantics never collide (M3/F2 convention); brand red appears only in the app mark. Neutrals carry a 0.003–0.017 chroma tint toward the accent hue for a cohesive cool grey [RUI]. Hex computed from OKLCH (gamut-clipped). Ratios are WCAG 2 relative-luminance contrast.
| token | light OKLCH | light hex | dark OKLCH | dark hex |
|---|---|---|---|---|
| color.bg (window) | 0.985 0.003 260 | #F9FAFC | 0.180 0.008 260 | #101215 |
| color.surface (panes, cards, fields) | 1.000 0 0 | #FFFFFF | 0.215 0.009 260 | #171A1E |
| color.surface_sunken (wells, tracks, header) | 0.962 0.005 260 | #F0F2F6 | 0.155 0.008 260 | #0A0C10 |
| color.surface_hover | 0.945 0.007 260 | #EAEDF2 | 0.260 0.011 260 | #212429 |
| color.selection (selected row) | 0.930 0.040 264 | #DAE8FF | 0.330 0.070 264 | #233459 |
| color.border (dividers, decorative) | 0.905 0.008 260 | #DCE0E5 | 0.300 0.012 260 | #2A2E34 |
| color.border_strong (control boundaries) | 0.620 0.015 260 | #81868F | 0.520 0.016 260 | #646972 |
| color.text | 0.220 0.015 260 | #171B22 | 0.940 0.006 260 | #E9EBEF |
| color.text_muted | 0.470 0.017 260 | #555B65 | 0.740 0.014 260 | #A6ABB4 |
| color.text_disabled | 0.680 0.010 260 | #95989F | 0.520 0.012 260 | #656970 |
| color.accent | 0.500 0.190 264 | #2858CD | 0.700 0.140 264 | #719CF5 |
| color.accent_hover | 0.450 0.180 264 | #1E49B7 | 0.750 0.130 264 | #84ACFF |
| color.accent_pressed | 0.400 0.160 264 | #183D9C | 0.660 0.140 264 | #668FE8 |
| color.accent_subtle | 0.950 0.025 264 | #E6EFFF | 0.300 0.060 264 | #1E2D4C |
| color.accent_text (links, accent icons) | 0.480 0.190 264 | #2251C6 | 0.780 0.120 264 | #90B6FF |
| color.on_accent (text on accent/danger) | 1.000 0 0 | #FFFFFF | 0.180 0.030 264 | #0B111F |
| color.focus | 0.550 0.200 264 | #3266E4 | 0.760 0.140 264 | #83AFFF |
| color.success / success_subtle | 0.50 0.13 150 / 0.955 0.035 150 | #137738 / #E0F7E4 | 0.78 0.14 150 / 0.29 0.05 150 | #6FD087 / #17321E |
| color.warning / warning_subtle | 0.52 0.12 65 / 0.96 0.045 85 | #975800 / #FFF0D0 | 0.82 0.13 80 / 0.30 0.05 75 | #F0BA59 / #3C2A0E |
| color.danger / danger_subtle | 0.52 0.20 27 / 0.955 0.025 20 | #C2181D / #FFEAE9 | 0.72 0.16 25 / 0.30 0.06 25 | #F97770 / #47211E |
| color.danger_hover | 0.470 0.190 27 | #AC0311 | 0.770 0.140 25 | #FF8E86 |
| color.brand (icon only) | 0.577 0.215 27.3 | #DC2626 | 0.637 0.208 25.3 | #EF4444 |
| color.scrim | black 40 % | #00000066 | black 60 % | #00000099 |

**Contrast ratios** (requirement: text ≥ 4.5:1 [1.4.3]; control boundaries, focus, icons, progress fill ≥ 3:1 [1.4.11]). Light / dark:
| fg / bg | light | dark | | fg / bg | light | dark |
|---|---|---|---|---|---|---|
| text / bg | 16.53 | 15.72 | | accent_text / surface | 6.86 | 8.58 |
| text / surface | 17.27 | 14.63 | | accent_text / bg | 6.57 | 9.22 |
| text / surface_sunken | 15.40 | 16.40 | | accent_text / accent_subtle | 5.93 | 6.72 |
| text / surface_hover | 14.71 | 13.04 | | accent_text / selection | 5.54 | 6.05 |
| text / selection | 13.95 | 10.31 | | accent_text / surface_hover | 5.85 | 7.65 |
| text / accent_subtle | 14.93 | 11.46 | | on_accent / accent | 6.24 | 6.98 |
| text / success_subtle | 15.29 | 11.64 | | on_accent / accent_hover | 7.80 | 8.37 |
| text / warning_subtle | 15.31 | 11.50 | | on_accent / accent_pressed | 9.60 | 5.98 |
| text / danger_subtle | 14.96 | 11.70 | | on_accent / danger | 6.10 | 7.09 |
| text_muted / bg | 6.55 | 8.13 | | on_accent / danger_hover | 7.57 | 8.50 |
| text_muted / surface | 6.84 | 7.57 | | success / surface · / subtle · / hover | 5.64 · 4.99 · 4.81 | 9.18 · 7.31 · 8.19 |
| text_muted / surface_sunken | 6.10 | 8.49 | | warning / surface · / subtle · / hover | 5.66 · 5.02 · 4.82 | 9.87 · 7.76 · 8.80 |
| text_muted / surface_hover | 5.83 | 6.75 | | danger / surface · / subtle · / hover | 6.10 · 5.29 · 5.20 | 6.57 · 5.25 · 5.86 |
| text_muted / selection | 5.53 | 5.33 | | border_strong / surface · bg · sunken | 3.66 · 3.50 · 3.26 | 3.16 · 3.40 · 3.55 |
| text_muted / accent_subtle | 5.91 | 5.93 | | focus / surface · bg · sunken · selection | 5.05 · 4.84 · 4.51 · 4.08 | 7.93 · 8.52 · 8.89 · 5.59 |
| accent / surface · sunken (progress fill) | 6.24 · 5.56 | 6.46 · 7.25 | | text_disabled / surface (exempt, inactive) | 2.89 | 3.17 |
`color.border` (1.33 / 1.28) is decorative only (dividers between regions that are also separated by fill); never the sole boundary of an interactive control. White on brand #DC2626 = 4.83 (icon glyph, non-text ≥ 3 ✓). Status is never conveyed by color alone: every semantic color pairs with an icon and a word [WCAG 1.4.1].

### 2.6 Radii, stroke, elevation, motion
- **radius**: `radius.sm 4` (checkbox, badge-square, tooltip), `radius.md 6` (buttons, fields, segmented, rows), `radius.lg 8` (cards, banners, panes inset), `radius.xl 12` (dialogs, drop zone, toasts), `radius.full 255` (chips, toggle, progress). Nested rule: inner radius = outer − padding (≥ 2) [RUI].
- **stroke**: `stroke.hairline 1` (borders), `stroke.focus 2`, `stroke.selected_bar 2`.
- **elevation** (egui `Shadow{offset:[i8;2], blur:u8, spread:u8, color}`; dark mode adds a 1 px `border` and a lighter fill because shadows vanish on dark bg [M3 tonal elevation]):
| token | use | light shadow | dark |
|---|---|---|---|
| elev.0 | panes, rows, fields | none (border/fill only) | none |
| elev.1 | cards, selected segment, toasts at rest | [0,1] blur 3 spread 0, black 8 % | fill surface_hover + border, black 40 % [0,1] blur 3 |
| elev.2 | menus, combo popups, tooltips, toasts | [0,4] blur 12, black 12 % | fill surface_hover + border, black 50 % [0,4] blur 12 |
| elev.3 | dialogs (+ scrim) | [0,12] blur 32, black 18 % | fill surface_hover + border, black 60 % [0,12] blur 32 |
- **motion** [M3 durations, F2 motion]: `motion.fast 100 ms` (hover/press color, toggle thumb), `motion.normal 160 ms` (expand detail sections, segmented indicator, banner in), `motion.slow 240 ms` (dialog/toast enter, drop overlay), exits = 0.75× enter. Easing: enter/move `emath::easing::cubic_out`, exit `cubic_in`, indeterminate progress `linear` 1200 ms loop. `style.animation_time = 0.1`. **Reduce motion** (Settings › General, default = OS: Windows `SPI_GETCLIENTAREAANIMATION`, macOS `accessibilityDisplayShouldReduceMotion`, GNOME `gtk-enable-animations`): all durations → 0, indeterminate bar → static 40 % striped fill + text [WCAG 2.3.3]. Nothing flashes > 3 Hz.
- **focus ring**: 2 px `color.focus` stroke drawn *outside* the widget rect with 2 px gap (`space.xxs`) filled with the underlying surface; follows widget radius + 2. Shown for keyboard focus only (`ctx.memory(|m| m.focused())` + last input was key). Never clipped by scroll areas: rows draw an inset ring (−2 px) instead [WCAG 2.4.11/2.4.13: ≥ 2 px perimeter, ≥ 3:1 vs both states].

### 2.7 Mapping to egui (`theme::apply(ctx, Theme)`, called once per theme via `ctx.set_style_of(egui::Theme::Light|Dark, style)`; `ctx.options_mut(|o| o.theme_preference = ThemePreference::System)` default)
- `Spacing`: `item_spacing = (space.sm, space.sm)`, `button_padding = (space.md, 6)` (14/20 label + 2×6 = 32), `interact_size = (size.control, size.control)`, `icon_width = 16`, `icon_width_inner = 8`, `icon_spacing = space.sm`, `menu_margin = space.sm`, `window_margin = space.xl`, `indent = space.lg`, `combo_height = 240`, `text_edit_width = 280`, `scroll = ScrollStyle::thin()` with `bar_width = 8`, `slider_rail_height = 4`.
- `text_styles`: Small→caption, Body→body, Button→body_strong (`inter-500`), Heading→title (`inter-600`), Monospace→mono, plus `TextStyle::Name("display"|"headline"|"title_sm"|"caption_strong"|"mono_sm")`.
- `Visuals`: `panel_fill = bg`, `window_fill = surface`, `extreme_bg_color = surface` (text-edit bg), `faint_bg_color = surface_sunken`, `code_bg_color = surface_sunken`, `hyperlink_color = accent_text`, `warn_fg_color = warning`, `error_fg_color = danger`, `window_corner_radius = radius.xl`, `menu_corner_radius = radius.lg`, `window_shadow = elev.3`, `popup_shadow = elev.2`, `window_stroke = 1 border`, `selection.bg_fill = selection`, `selection.stroke = 1 text`, `text_cursor.stroke = 2 accent`, `striped = false`, `slider_trailing_fill = true`, `handle_shape = Circle`.
- `widgets` (`WidgetVisuals{bg_fill, weak_bg_fill, bg_stroke, corner_radius, fg_stroke, expansion}`; all `corner_radius = radius.md`, `expansion = 0`): `noninteractive` fg text, stroke 1 border · `inactive` weak_bg surface, bg surface_sunken, stroke 1 border_strong, fg text · `hovered` weak_bg/bg surface_hover, stroke 1 border_strong, fg text · `active` bg surface_hover darkened via accent_subtle, stroke 1 accent, fg text · `open` = hovered.
- egui defaults are acceptable only for: `ScrollArea`, `ComboBox` popups, `TextEdit` internals, tooltips, context menus. **Custom widgets** (module `gui/widgets/`): `Button` (variant + loading + icon; egui `Button` cannot express primary/danger per instance without `fill()` + manual hover), `Badge`, `PlanRow`, `Segmented`, `Toggle`, `Field` (label/helper/error wrapper around `TextEdit`), `Banner`, `Toast` stack, `Progress`, `DropZone`, `Dialog` (modal via `egui::Modal` + scrim), `FocusRing` painter, `Icon`.
- Every custom widget calls `response.widget_info(|| WidgetInfo::labeled(type, enabled, label))` (§8).

## 3. Layout grid
- Window: **default 1280 × 800**, **min 960 × 640** (`ViewportBuilder::with_min_inner_size`), position/size persisted (eframe `persist_window`). Content is fluid; no max width except prose (560) and settings column (640).
- Regions (top→bottom): **Header** 48 (fill surface_sunken, bottom border) · **Toolbar** 40 (Preview/Companies only) · **Content** (flex) · **Action bar** 56 (fill surface, top border; present on Preview/Done). Header/toolbar/action-bar horizontal padding `space.lg`.
- **Details panel is opt-in (expert view, F2 2026-11-09).** Hidden by default; visibility persisted per computer (eframe storage, like theme). Toggle: icon button (`panel-right`, 32, ghost) at the right end of the list toolbar, tooltip "Show details (Ctrl+\\)" / "Hide details (Ctrl+\\)", pressed state while open; Companies uses the same toggle and storage. Selecting a row never opens it. Open: the split below.
- **List : detail = φ (1.618 : 1).** Detail width = round₈(window_w × 0.382), clamped [320, 560]; list gets the rest (1280 → 791 | 1 | 488; 960 → 591 | 1 | 368). Rationale: the list holds two filename lines and needs the larger share; the detail is a reading pane whose 488 px ≈ 62 ch of body text sits in the optimal measure. User-resizable divider (8 px hit, 1 px visual), double-click resets to φ. `Ctrl+\` hides detail (list then full width).
- Settings: left nav 200 (≈ 1280 × 0.156, fixed) + form column max 640 starting at `space.xl`. Companies: same φ split as Preview.
- Vertical rhythm: section titles have `space.xl` above, `space.sm` below; fields in a group `space.lg` apart; groups `space.xxl` apart.

## 4. Screens
Wireframes at 1280 px; 1 char ≈ 10 px horizontally, annotations give exact px. `[ ]` button, `( )` segmented/radio, `[x]` checkbox, `◉` badge.

### 4.1 Header (all screens)
```
| ■ AutoRename-PDF   Rename  Companies(3)  History                                     [◉ Offline]  ⚙ |
  20px mark+title_sm  tabs: 32h, 12px pad, active = text+2px accent underline           badge 20     icon-btn 32
```
Settings live only in Settings (one source of truth): no profile combo, no provider chip, no quick toggles in the header. Tabs are a `Segmented`-free tab list (role Tab). Name-matching tab: label = plural from core `field_label` of the active profile's `harmonize_field` (business: "Companies"), shown whenever the profile has one (stable navigation; Ctrl+2 does nothing otherwise); count = entries with `review: true` (warning badge). Offline badge: passive success badge `shield-check` "Offline", only when the saved privacy is offline; tooltip "Documents stay on this computer. Change this in Settings › Processing." Not interactive.

### 4.2 Empty / onboarding (first run = no usable provider config)
```
| header                                                                                                    |
|                         ┌───────────────── 640 × 280, radius.xl, dashed border_strong ──────────────┐     |
|                         │                     [file-up icon 48, text_muted]                         │     |
|                         │              Drop PDFs or folders here            (title_sm)              │     |
|                         │      Nothing is renamed until you review the plan. (caption, muted)      │     |
|                         │              [Choose files]  [Choose folder]   (secondary, 32h)          │     |
|                         └──────────────────────────────────────────────────────────────────────────┘     |
|                 24px gap                                                                               |
|                         ┌─ Set up AI (card, elev.1, 640 wide, padding 24) ───────────────────────────┐   |
|                         │ Where should document text be processed?                                 │   |
|                         │ ( Cloud provider | Offline on this computer )   segmented 32h             │   |
|                         │ Cloud: Provider [OpenAI ▾]  Model [gpt-6-luna ▾]                        │   |
|                         │        API key [••••••••••    ][eye]  ✓ Found in OPENAI_API_KEY          │   |
|                         │ Offline: Ollama at [http://localhost:11434] ● Connected · 3 models        │   |
|                         │        OCR models ◉ Not installed  [Download OCR models (size 38 MB (latin pack))]         │   |
|                         │                                         [Test connection] [Save and continue] │
|                         └──────────────────────────────────────────────────────────────────────────┘   |
```
Card hidden once config validates; drop zone then fills the content area (min 240 h, centered, 640 wide). While the OCR models for `ocr.language` are not installed (OCR not off, downloads allowed, not the offline card, which has its own OCR row), one optional recommendation sits below the card / drop zone (640 wide, radius.lg, hairline border, padding 12/16, info icon): title "Read scans on this computer (optional)", caption by configured route (height from the wrapped caption): AI image reading on → "Pages that need image reading are sent to <Provider>. Download the scan reader to read scans on this computer first."; off → "Scans can't be read with your current settings. Download the scan reader or enable AI image reading in Processing settings."; before setup → "Reads scanned text on this computer before using AI image reading.", [Download scan reader (38.9 MB)] (secondary; never automatic; while downloading: progress caption + [Cancel download]). Drop zone states: rest / drag-over valid (2 px accent solid, accent_subtle fill, text "Drop to add 3 PDFs") / drag-over invalid (danger border, "Only PDFs and folders") / disabled during analysis. Whole-window drag shows an overlay (scrim 20 %, same text) on every screen; drop on Settings/Companies switches to Rename. Offline option never asks for an API key; download size and hash source shown; download progress inline (§5.8), cancellable.

### 4.3 Preview (plan list)
```
| header                                                                                                    |
| 12 files · 2 need a look                                   (All 12|Needs a look 2)   [+ Add…]   [▯▮]     |  toolbar 40
|--list (full width; φ split with §4.4 when details are open)-----------------------------------------------|
|[x] ✓  20251103 Amazon AP 12,13.pdf                                                                        |
|       from scan_0041.pdf                                                                                  |  row 56
|[x] !  20251028 Telekom Deutschland AP 54,99.pdf  ✎                                                        |
|       from Rechnung (2).pdf · Two dates were found. Check the date.                                       |
|[ ] ✕  Couldn't read the file · The AI service didn't answer in time                       [Try again]     |
|       from broken.pdf                                                                                     |
|---------------------------------------------------------------------------------------------------------|
| 10 of 12 selected · 1 name edited                                        [Clear list]  [Rename 10 files] | action bar 56
```
- Toolbar: plain summary "N files" + " · K need a look" (K = review + error rows; omitted when 0); while analyzing "Analyzing 4 of 12 · about 20 s left". Filter `Segmented` (All N | Needs a look K) only while K > 0 (otherwise the filter resets to All). No search box. [+ Add…] secondary, then the details toggle (§3).
- **PlanRow** (56 h, padding 8/16): checkbox (32 hit) · status icon 16 (`circle-check` success, `triangle-alert` warning, `circle-x` danger, spinner while analyzing) · gap 12 · line 1 proposed name `type.mono` in `text` · line 2 `type.mono_sm` "from <old>" + `·` + one plain problem sentence in caption warning color (core warning strings mapped to plain copy by `model::plain_warning`; unknown strings fall back to the raw text as a sentence; the raw text stays in the details panel). No badges in rows: reading tier, metadata and name-matching outcome live in the details panel. Line 1 middle ellipsis, line 2 end ellipsis, full text in tooltip.
- States: rest surface · hover surface_hover · selected selection fill + 2 px accent left bar · unchecked (excluded) line 1 text_muted · focus inset ring · analyzing: line 1 = step text in muted · error: line 1 plain error summary, `Try again` ghost sm button (24 h).
- **Inline edit** (F2, double-click line 1, or ✎ in detail): line 1 becomes `Field` (mono, 32 h) with `.pdf` fixed suffix adornment; validates live with core filename rules (invalid chars, length, collision with another row or existing file → danger helper below, row grows to 72). Enter commits, Esc reverts. Edited rows show ✎ `pencil` icon; "Reset to suggested" in detail.
- Sorting: plan order (input order); toolbar filter is a `Segmented`. Errors and review rows are never auto-checked off; errors are unchecked and uncheckable.
- Primary button label always contains the count; disabled with tooltip "Select at least one file" when 0. Rename uses `apply` on the shown plan (no new AI calls); source-hash mismatch → row error "File changed since analysis · Re-analyze".

### 4.4 Details panel (expert view, opt-in §3; selected row; padding 16, scrolls independently)
```
| 20251028 Telekom Deutschland AP 54,99.pdf  [pencil]   (title_sm mono, wraps) |
| from C:\Scans\Rechnung (2).pdf  [copy] [external-link]          (caption)   |
| ── Fields ──────────────────────────────────────────────────────────────── |
| Company    Telekom Deutschland        Date     2025-10-28 (from "28.10.25") |
| Type       AP                         Total    54,99 EUR                     |
| ── Company match ───────────────────────────────────────────────────────── |
| "TELEKOM DEUTSCHLAND GMBH" → Telekom Deutschland · similarity 0.95 ≥ 0.92  |
| ◉ Alias added   Will be added to companies list on rename.  [Don't learn]   |
| ── Metadata ─────────────────────────────────────────────────────────────── |
| ◉ Will write title, subject, keywords + XMP + JSON attachment     | or ◉ Skipped: signed PDF   |
| ── Extraction ───────────────────────────────────────────────────────────── |
| Tier OCR · PP-OCRv6 small · 2 pages · OpenAI gpt-6-luna · 3.1 s          |
| ┌ extracted text (mono 12/16, sunken well, 8 lines, "Show all") ────────┐  |
| └────────────────────────────────────────────────────────────────────────┘  |
| ── Warnings ── ! Low confidence on date: two dates found (28.10.25, 01.11.25)|
```
Field grid: 2 columns, label column 72 px caption text_muted, value body. Company match explains the decision band verbatim (≥ 0.92 alias / 0.75–0.92 decider choice with candidates listed / < 0.75 new entry / offline → new entry "Needs review"). Section headers `type.caption_strong` text_muted with 1 px border rule; sections collapsible (`motion.normal`). Empty selection: centered caption "Select a file to see details". Multi-select: "3 files selected" + bulk actions [Include] [Exclude].

### 4.5 Progress
Analysis: toolbar shows determinate `Progress` (4 h, full list width, under toolbar) + caption "Analyzing 4 of 12 · about 20 s left" (ETA after 3 files; hidden before); rows fill in as results stream; list remains navigable; action bar primary becomes `[Stop]` secondary, Rename disabled until ≥ 1 ready file and analysis is stopped or done. Renaming: action bar replaces summary with progress bar 240 w + "Renaming 7 of 10"; non-cancellable once a file write starts (core is journaled), cancellable between files. Live text exposed to AccessKit with polite live region; announced at most every 2 s.

### 4.6 Done / summary
```
|  ✓ (icon 24 success)  Renamed 10 files                         (headline 24/32)            |
|  1 couldn't be renamed · 1 new company needs a look           (caption; only non-zero parts) |
|  [Undo rename]  [Open folder]  [Review companies]                         [Rename more files] |
|  ── result list (PlanRow read-only, status = rename_status) ────────────────────────────────  |
```
Summary is a card (elev.1, padding 24) at top of the list pane; the details panel keeps its visibility. Plain copy: no metadata/tier counts (those are in the details panel). [Review companies] (field noun) only when the batch created entries needing review. Undo reverts renames, metadata and learned aliases of the batch (copy says so in tooltip). After undo: success toast "Undid 10 renames" ; conflict per file → banner "2 files changed after renaming and were not restored" + list. **History** tab: list of batches (date, file count, folder) each with `[Undo]`; undone batches greyed with "Undone" badge.

### 4.7 Settings (left nav 200: General, Processing, Naming, About)
Four pages; each page is one scroll column (max 640) of groups. Group heading `type.title_sm` + one-line muted description; `space.xxl` between groups. Every control has a label, a one-line helper that says what it does in plain words (no config key names), and a live status line where the outcome depends on other settings. Every config key except profile templates/fields is editable here (templates/fields: YAML, [Open settings file]). Saving writes only the keys that changed (comments and untouched bytes stay identical: CRLF line endings, the final newline, and the whole old value of an edited key, block scalars and flow values included, are handled by `autorename_core::config_edit`). Free-text values are always written quoted. Storage details (`.env`, variable names, config keys) go into tooltips, never the primary text.

- **General** — *Appearance* (stored per computer, applies immediately): Theme (System | Light | Dark), Fewer animations (System | On | Off; helper "Reduces movement throughout the app."), App size (90/100/110/125 %, `ctx.set_zoom_factor`; helper "Changes text and control sizes."). *Files*: settings file, name lists folder, undo history (mono_sm, middle ellipsis) + [Open settings file] [Open settings folder]. About has no file list.
- **Processing** — what happens to each PDF before it is named.
  - *Privacy*: `Segmented` (Cloud | Offline) + explanation (body, 560 max). While the choice differs from the saved mode: caption "Current mode: <saved>" beside the control and the explanation starts "After saving, …" (the header chip keeps showing the saved mode). Offline with a cloud provider / non-local base URL / cloud decider → inline danger banner with an instruction ("Choose Ollama below to use Offline mode."), Save disabled.
  - *AI provider*: Provider `Select` (OpenAI, Anthropic, Gemini, xAI, Ollama), Model (mono, helper "Provider default: …"), API key `Field` (secret, show/hide, status line leads with the outcome: "API key saved" / "API key found on this computer" / "Paste your key again to store it safely" (warning) / "Add your <Provider> API key to use <Provider>." (danger, Save blocked with "Add your <Provider> API key to save"); variable and `.env` details in the tooltip), Custom server address (optional) (Ollama or OpenAI-compatible; must start with http:// or https://), [Test connection] → result badge (latency or error). Switching the provider rebinds the key to the new provider's variable (`ai.api_key: "${ANTHROPIC_API_KEY}"`, empty for Ollama) and drops a typed key; status line, Test connection and Save share this resolution (`SettingsForm::ai_credential`).
  - *Reading*: Pages read per PDF (stepper 1–10, `pdf.max_pages`); one helper "Auto uses a step only for pages the step before couldn't fully read: scans, and pages with pictures such as logos. On uses it for every page."; reading route 1 Embedded text → 2 Read scans on this computer (helper introduces OCR once; Auto | On | Off) → 3 Read page images with AI (Auto | On | Off), each with live status; "Scan languages" `Select` (`ocr.language`): first "Automatic (all installed languages)" (`auto`, default), then one option per known pack, e.g. "Latin script: English, German, French + 45 more" (writes the pack id; the full language list in the tooltip), no per-language options; a hand-written code in config shows as its pack's option, an unknown one as itself with a warning status; helper "A scan reader reads all languages of its script at once, so mixed English and German pages work."; Models table (human title "Scan reader · English and other Latin-script languages", may wrap to two lines; size; badge Installed / Not installed / Downloading, [Download] [Remove] [Cancel download]; engine/version and the full language list in the row tooltip) + [Import from folder…]. Never downloads without click. Banner when scans can't be read (no models and vision off).
  - *Metadata*: Toggle "Write metadata to renamed PDFs" + caption (what is written, what is skipped).
  - *Advanced* (chevron group heading, collapsed by default, `motion.normal`, shared `components::reveal`): Minimum text readability (`pdf.text_quality_threshold`, stepper 0.50–1.00, step 0.05; helper "Pages scoring below this are read again. Pages with pictures or drawings start at 0.75, so a limit above that reads them with OCR. 1.00 means clean text with nothing in pictures."), Temperature (0.0–1.0 step 0.1; "Lower values make answers more consistent."), Retries on provider errors (0–5), OCR threads (1–logical CPUs).
- **Naming** — how the new filename is built. Only settings the selected profile uses are shown (`settings::Uses`): Your company and the invoice codes for profiles with `direction`, Amount for profiles with `amount`, the document-type language for profiles whose instructions use `{language}`; Date always.
  - *Profile*: radio rows (72 h, the whole row is the radio: name, fields, live sample name rendered by core from sample values and the current form's formats), default badge on the saved one; [Open settings file] to edit templates.
  - *Your company*: Field "Your company name" (`company.name`), helper: never used as the other party; decides AP (bills you receive) vs AR (invoices you send). Warning status when empty: "Add your company so the AI can tell bills you receive from invoices you send."
  - *Name format*: Code for bills you receive (`pdf.incoming_invoice`, default AP) and for invoices you send (`pdf.outgoing_invoice`, default AR); Date format (`Select` of presets 20251103 / 2025-11-03 / 03.11.2025 / 2025_11_03 + Custom → labelled "Custom date format" mono field with strftime legend and live example, both kept under an error; a forbidden character says "Use dots or dashes instead of / or :, e.g. %d.%m.%Y."); Amount format (`Select` of presets `{currency} {amount}` / `{amount} {currency}` / `{amount}`, plus a hand-written config value as an extra option; live example); Language for document type names (`output.language`, `Select` of common languages + "Other…" free-text field).
  - *Name matching* (`harmonize_field` of the selected profile; label from core `field_label`, e.g. "Companies"): Toggle "Learn new <plural> automatically", decider `Select` labelled "Who checks similar <singular> names?" (Your selected AI provider | OpenAI Decisions | Jev | Off; cloud deciders unavailable offline) + key field when needed (status as for the API key; a missing key blocks Save; a decider switch rebinds or clears `decider_api_key` like a provider switch), list file path + [Open file]. Profile without `harmonize_field`: caption "The <Profile> profile doesn't match names." and no controls.
  - *Extra instructions*: multi-line field (`prompt_extension`), helper with an example fitting the profile (business: brand vs legal name; academic: "Keep paper titles in their original language.").
- **About**: version, license line + [View license], support card, third-party notices, imprint.
- Saving: explicit [Save changes] / [Discard] in a sticky footer when dirty; validation errors as field helpers + footer message (exit-3 class). Appearance applies immediately and is not part of the footer.

### 4.8 Name matching (tab label from the active field, e.g. Companies)
Toolbar: (All | Needs review 3 | Learned | Manual) + search. List rows 56: canonical name (body_strong), caption "4 aliases · learned 2026-11-02" + badges `Review`/`Auto`. Detail: aliases list (mono_sm, remove ✕ 24 hit), source, last used; review actions: [Keep as new entry] (copy uses the field noun, e.g. "Keep as new company") primary, [Merge into…] secondary, [Rename]. Merge dialog (560): searchable list of canonical targets with similarity score, preview "3 aliases move to Telekom Deutschland", [Merge] primary, [Cancel]. Manual entries show "Hand-written · not editable here" with [Open file]. [Rename] changes the spelling of one entry (`harmonize::rename`, old spelling kept as alias); a name owned by another entry is refused with a pointer to Merge. Loads and edits run on the worker (`update_db` waits for the list's `.lock` there, never on the UI thread); edit actions are disabled while one runs. The list reloads after apply/undo, on tab entry and when the file's mtime changes; the selection is kept by name.

### 4.9 Error states (component · copy · action)
| condition (core code) | where | copy | action |
|---|---|---|---|
| no API key / auth (11) | global banner danger | "OpenAI rejected the API key." | [Open provider settings] |
| provider error (10, retryable) | row error | "Provider timeout · retried 2×" | [Retry] row, [Retry failed] toolbar |
| `ocr_models_missing` | banner warning | "This scan needs OCR models, which aren't installed." | [Download OCR models] |
| offline violation (3) | banner danger | "Offline mode blocks requests to api.openai.com. Use an address on this computer or your network." | [Open privacy settings] |
| no PDFs found (4) | drop zone invalid + toast | "No PDFs in the dropped folder." | — |
| partial (5) | Done card warning variant | "Renamed 8 of 10 files · 2 failed" | failed rows keep [Retry] |
| locked/permission/collision | row error | "File is open in another program." | [Retry] |
| undo conflict | banner warning | "2 files changed after renaming and were not restored." | [Show files] |
| config invalid (3) | Settings banner + field helpers | "config.yaml has 2 problems." | [Open config file] |
Banners sit under the toolbar (full width, radius.lg inset 16), max 2 stacked (newest first, others collapsed into "+1 more"); dismissible unless blocking.

## 5. Components
States for all interactive components: **rest · hover (motion.fast) · pressed · focus (ring §2.6) · disabled (text_disabled, no hover, `enabled=false` to AccessKit, tooltip states why) · loading (where noted)**.
1. **Button** — heights 24 (sm, ghost/icon only) / 32 (md) / 40 (lg); padding x 12 / 12 / 16; icon 16 + gap 8; label body_strong; radius.md; min width 64 (md) for text buttons; icon-only = square + tooltip + AccessKit label. *Primary*: accent → accent_hover → accent_pressed, text on_accent. *Secondary*: surface, 1 px border_strong, text; hover surface_hover. *Ghost*: transparent, text (or text_muted for toolbar icons); hover surface_hover. *Danger*: danger → danger_hover, on_accent; only for irreversible actions (Remove model, Delete alias). Loading: 16 spinner replaces icon/left of label, width frozen, clicks ignored. One primary per view.
2. **Badge / chip** — 20 h, padding x 8, gap 4, icon 12, caption_strong, radius.full. Variants: neutral (surface_sunken / text_muted), accent (accent_subtle / accent_text), success, warning, danger (semantic_subtle / semantic). Interactive chip (privacy chip) = 24 h, hover surface_hover, focus ring. Badge vocabulary: Text, OCR, Vision, Metadata, Metadata skipped, Alias added, New company, Review, Edited, Skipped, Offline, Installed, Undone.
3. **List row (PlanRow, CompanyRow)** — §4.3. Whole row is one AccessKit node (role ListItem, selected state); checkbox and row actions are children.
4. **Segmented** — container 32 h, surface_sunken, radius.md, inset 2; segments 28 h, padding x 12, body (selected body_strong) ; selected segment surface + elev.1, text; unselected text_muted, hover text. Indicator slides `motion.normal`. Keys: ←/→ move + select, Home/End. Role RadioGroup/RadioButton.
5. **Toggle** — track 36 × 20 radius.full; thumb 16, inset 2. Off: track surface_sunken + 1 px border_strong, thumb border_strong. On: track accent, thumb on_accent. Thumb travel `motion.fast`. Hit area = toggle + label row (≥ 32 h). Space toggles. Role Switch (WidgetType::Checkbox, selected).
6. **Text field (Field)** — label above (body_strong, gap 4), control 32 h, padding x 12, surface, 1 px border_strong, radius.md; hover border text_muted; focus 2 px focus border (replaces ring); error 1 px danger + helper (caption danger, `circle-x` 12); helper caption text_muted; disabled surface_sunken. Adornments (suffix `.pdf`, eye toggle) inside, 24 hit.
7. **Banner** — radius.lg, padding 12/16, semantic_subtle fill, 1 px semantic border at 40 % alpha, icon 16 semantic, title body_strong, body body, actions ghost/secondary right; dismiss ✕ 24.
8. **Toast** — bottom-center 16 above action bar, width 360–480, surface + elev.2, radius.xl, padding 12/16; icon + one line + optional action (ghost accent_text) + ✕. Auto-dismiss 6 s, paused on hover/focus; never the only path to an action (Undo also in History) [WCAG 2.2.1]. Max 3 stacked, 8 apart.
9. **Progress** — 4 h track surface_sunken, fill accent, radius.full; determinate animates width `motion.normal`; indeterminate 30 % segment linear 1200 ms; label caption above-right "4 of 12". Role ProgressIndicator with value.
10. **Drop zone** — §4.2; min 240 h, radius.xl, 1 px dashed border_strong (dash 6, gap 4 via `Shape::dashed_line`), keyboard-focusable: Enter = Choose files.
11. **Dialog** — `egui::Modal`, widths 440 / 560, padding 24, radius.xl, elev.3, scrim; title (title) + body + footer (gap 8). Button order: Windows primary-left `[Merge] [Cancel]` per F2; macOS/Linux `[Cancel] [Merge]` per HIG/GNOME; always right-aligned. Focus trapped, initial focus = primary (destructive dialogs: Cancel), Esc cancels, Enter primary.
12. **Tooltip** — inverse colors (fill `text`, label `bg`; same ratios as text/bg) caption, radius.sm, padding 4/8, delay 500 ms, elev.2; never holds essential info except for icon-only buttons (which also have AccessKit labels).

## 6. Iconography and app icon
- **Lucide** (ISC; https://lucide.dev) — 24-grid, 2 px stroke, rendered from `lucide.ttf` (lucide-static, pinned) subset to used glyphs; codepoints in generated `theme::icon` consts. Sizes 12/16/24/48 only; color = adjacent text color or semantic. Set: `file-text` file, `file-up` drop, `folder-open`, `plus`, `play`/`check` rename, `undo-2`, `history`, `settings`, `building-2` companies, `git-merge`, `cloud`, `shield-check` offline, `key-round`, `scan-text` OCR, `eye`/`eye-off`, `image` vision, `file-pen-line` metadata, `user-plus` new company, `link` alias, `flag` review, `pencil`, `circle-check`, `triangle-alert`, `circle-x`, `info`, `x`, `chevron-down`, `chevron-right`, `arrow-right`, `loader-circle` (spinner, 800 ms rotation), `search`, `copy`, `external-link`, `download`, `trash-2`, `rotate-ccw` retry.
- **App mark refinement** (justified: at 16 px the current 1024 master shrinks the glyph to ~10 px with a 0.9 px stroke and a 3-line motif that merges into a grey smear). Keep master SVG for ≥ 48 px. Add hand-hinted small masters: **16/20/24/32 px** — flat `#DC2626` (no gradient; gradient is invisible < 32 px), corner radius 3/4/5/6, white page outline with folded corner, stroke snapped to whole pixels (1 px at 16/20, 1.5→2 px at 24/32 with pixel-aligned coordinates), only **two** text lines (drop the short third line), glyph occupies 62 % of the tile. White on #DC2626 = 4.83:1. Dark-taskbar check: 1 px `#991B1B` outer edge keeps the tile visible on red/dark shells.
- **Windows .ico**: 16, 20, 24, 32, 40, 48, 64, 256 (PNG-compressed 256) [F2 app-icon construction]; context-menu verb icon uses the same .ico (Explorer picks 16/20/24/32 at 100/125/150/200 % scale). **macOS .icns**: 16, 32, 128, 256, 512 @1x and @2x (1024), body = 824×824 rounded rect on 1024 canvas with macOS-style continuous corners and subtle drop shadow [HIG]. **Linux hicolor**: `apps/autorename-pdf.png` at 16, 22, 24, 32, 48, 64, 128, 256, 512 + `scalable/apps/autorename-pdf.svg` [FD]; window icon via `ViewportBuilder::with_icon` (256 PNG).

## 7. Copywriting
- Sentence case everywhere ("Download OCR models", not "Download OCR Models"); product name "AutoRename-PDF" unchanged.
- Buttons = verb + object, with count when it applies: "Rename 10 files", "Undo rename", "Retry failed", "Merge into…". Never "OK"/"Yes"; dialogs name the action. Ellipsis "…" only when another step (dialog/picker) follows.
- Plain words over internals: "Metadata" not "enrich", "Text/OCR/Vision" not "tier 1/2a/2b", provider names as brands. Error copy: what happened + what to do, ≤ 2 sentences, no codes in the headline (code in tooltip/detail for support).
- Numbers: digits always ("2 files"), singular/plural correct ("1 file"); thousands separator per OS locale in counts; file sizes SI with one decimal ("118 MB", "4.2 MB"); durations "3.1 s", "about 20 s left", "2 min"; percentages integers; text-layer quality as word + number in the detail ("good (0.94)"); dates in UI ISO `2025-10-28` (unambiguous, matches filenames); money in detail as shown in the generated name.
- Tone: calm, specific, no exclamation marks, no blame ("couldn't read the file", not "invalid file").

## 8. Keyboard map and accessibility
| key (Cmd on macOS) | action |
|---|---|
| Ctrl+O / Ctrl+Shift+O | Choose files / folder |
| Ctrl+1 / 2 / 3 | Rename / name-matching (when shown, e.g. Companies) / History tab |
| Ctrl+, | Settings |
| ↑ ↓ / Home End / PgUp PgDn | move active row |
| Shift+↑↓, Ctrl+A | extend selection, select all |
| Space | include/exclude active (or selected) rows |
| F2 / Enter on name | edit proposed name; Enter commit, Esc revert |
| Delete | remove row(s) from plan (not from disk) |
| Ctrl+Enter | primary action (Rename N files / Save changes) |
| Ctrl+Z | Undo last batch (Done/History; confirms if > 1 batch back) |
| Ctrl+R | Retry failed rows |
| Ctrl+\ | show/hide details panel |
| Esc | close dialog/toast/edit, stop analysis (asks) |
| Tab / Shift+Tab | header → toolbar → list → detail → action bar; inside list, arrows move rows and Tab leaves |
AccessKit labels (via `widget_info`): PlanRow "scan_0041.pdf, will be renamed to 20251103 Amazon AP 12,13.pdf, text extraction, metadata will be written, included"; status icons are not separate nodes (folded into row label); icon buttons "Copy original path", "Edit proposed name"; badges contribute words not colors; details toggle "Show details" / "Hide details" (role ToggleButton); progress "Analyzing, 4 of 12" with polite live updates; dialogs role Dialog with title as label. Tested with `egui_kittest` a11y tree queries (`get_by_label`).

## 9. Visual acceptance checklist (review gate, all must pass)
1. `theme.rs` is the only file with color literals and px numbers (grep for `Color32::from_rgb`/numeric `vec2(` outside theme/widgets = 0 hits).
2. Every widget x/y/w/h and gap in kittest snapshots at 1280×800 and 960×640 is a multiple of 4 (pixel-measured on 5 sampled screens); pane splits within ±1 px of φ formula.
3. All fg/bg pairs in the §2.5 table recomputed by a unit test from the hex tokens: text ≥ 4.5, non-text ≥ 3.0, both themes.
4. Only 3 font weights and the 10 type tokens appear; no text < 12 px; prose width ≤ 560.
5. Interactive hit rects ≥ 24 × 24 (100 %), ≥ 32 h for buttons/fields/rows (except sm ghost buttons); checked via kittest response rects.
6. Keyboard-only run completes: first-run setup → drop (Ctrl+O) → edit one name → exclude one → rename → undo, with focus ring visible on every stop and never clipped.
7. AccessKit tree: no unlabeled interactive node; every row label contains old name, new name and status words.
8. Light and dark snapshots for: empty, onboarding, preview (mixed states incl. error, edited, review), detail, analyzing, renaming, done, partial, settings each page (incl. Advanced expanded, profile without name matching), companies review, merge dialog, each banner type.
9. Status never color-only: every success/warning/danger instance has icon + text (snapshot review).
10. Reduce motion = On: no animation frames requested after state change (`ctx.has_requested_repaint` settles within 1 frame); no element flashes.
11. Icon renders: 16/20/24/32 PNG small masters reviewed at 100 % on Windows light/dark taskbar and Explorer context menu; glyph strokes on whole pixels.
12. Copy lint: no Title Case buttons, no "OK", counts present on primary actions, no exclamation marks.
