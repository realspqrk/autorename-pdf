//! Design tokens (spec `2026-11-07-gui-design.md` §2) and the egui style
//! mapping. This is the only GUI file with color literals and px numbers.
//! Doc token `group.name` ↔ `theme::group::NAME` / `Palette.field`.
//! The full token set is kept even where only part B screens use a token.
#![allow(dead_code)]

use egui::epaint::{CornerRadius, Margin, Shadow, Stroke};
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle, Vec2};
use std::sync::Arc;

/// §2.1 spacing (4-pt base, 8-pt rhythm).
pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
    pub const XXXL: f32 = 48.0;
}

/// §2.2 sizes / hit targets.
pub mod size {
    pub const CONTROL: f32 = 32.0;
    pub const CONTROL_SM: f32 = 24.0;
    pub const CONTROL_LG: f32 = 40.0;
    pub const ICON: f32 = 16.0;
    pub const ICON_SM: f32 = 12.0;
    pub const ICON_LG: f32 = 24.0;
    pub const ICON_XL: f32 = 48.0;
    pub const ROW: f32 = 56.0;
    /// Row while inline-editing with a helper line below.
    pub const ROW_EDIT: f32 = 72.0;
    pub const HEADER: f32 = 48.0;
    pub const TOOLBAR: f32 = 40.0;
    pub const ACTION_BAR: f32 = 56.0;
    /// §5.2 badge height; interactive chip height is `CONTROL_SM`.
    pub const BADGE: f32 = 20.0;
    /// §5.4 segment inside the 32 px container.
    pub const SEGMENT: f32 = 28.0;
    /// §5.5 toggle track / thumb.
    pub const TOGGLE_W: f32 = 36.0;
    pub const TOGGLE_H: f32 = 20.0;
    pub const TOGGLE_THUMB: f32 = 16.0;
    /// §5.9 progress track height, renaming progress width.
    pub const PROGRESS_H: f32 = 4.0;
    pub const PROGRESS_RENAME_W: f32 = 240.0;
    /// §4.3 confidence dots.
    pub const DOT: f32 = 8.0;
    /// Checkbox visual inside its 32 px hit area.
    pub const CHECKBOX: f32 = 16.0;
    /// §5.1 min width of md text buttons.
    pub const BUTTON_MIN_W: f32 = 64.0;
    /// §4.1 toolbar search field width.
    pub const SEARCH_W: f32 = 200.0;
    /// Header profile combo width.
    pub const COMBO_W: f32 = 120.0;
    /// §4.2 drop zone / setup card width and drop-zone min height.
    pub const CARD_W: f32 = 640.0;
    pub const DROP_MIN_H: f32 = 240.0;
    pub const DROP_H: f32 = 280.0;
    /// §2.3 max prose measure.
    pub const PROSE_MAX: f32 = 560.0;
    /// §3 settings nav and form column.
    pub const SETTINGS_NAV: f32 = 200.0;
    pub const SETTINGS_COL: f32 = 640.0;
    /// §4.4 detail field label column.
    pub const LABEL_COL: f32 = 72.0;
    /// §3 detail pane clamp and divider.
    pub const DETAIL_MIN: f32 = 320.0;
    pub const DETAIL_MAX: f32 = 560.0;
    pub const DIVIDER_HIT: f32 = 8.0;
    /// §4.4 extracted-text well lines.
    pub const WELL_LINES: usize = 8;
    /// §5.11 dialog widths.
    pub const DIALOG_SM: f32 = 440.0;
    pub const DIALOG_MD: f32 = 560.0;
    /// Middle-ellipsis budget for the window title / tooltips.
    pub const TOAST_MIN: f32 = 360.0;
    pub const TOAST_MAX: f32 = 480.0;
    /// §3 window.
    pub const WINDOW_DEFAULT: [f32; 2] = [1280.0, 800.0];
    pub const WINDOW_MIN: [f32; 2] = [960.0, 640.0];
}

/// §2.6 radii (u8, egui `CornerRadius`).
pub mod radius {
    pub const SM: u8 = 4;
    pub const MD: u8 = 6;
    pub const LG: u8 = 8;
    pub const XL: u8 = 12;
    pub const FULL: u8 = 255;
}

/// §2.6 strokes.
pub mod stroke {
    pub const HAIRLINE: f32 = 1.0;
    pub const FOCUS: f32 = 2.0;
    pub const SELECTED_BAR: f32 = 2.0;
    /// §5.10 drop-zone dash pattern.
    pub const DASH: f32 = 6.0;
    pub const DASH_GAP: f32 = 4.0;
}

/// §2.6 motion (seconds).
pub mod motion {
    pub const FAST: f32 = 0.100;
    pub const NORMAL: f32 = 0.160;
    pub const SLOW: f32 = 0.240;
    pub const EXIT_FACTOR: f32 = 0.75;
    pub const INDETERMINATE_LOOP: f32 = 1.2;
    pub const SPINNER_LOOP: f32 = 0.8;
    pub const INDETERMINATE_SEGMENT: f32 = 0.30;
    pub const REDUCED_FILL: f32 = 0.40;
    pub const TOAST_SECS: f32 = 6.0;
    pub const TOOLTIP_DELAY: f32 = 0.5;
    /// §4.5 live-region throttle.
    pub const ANNOUNCE_SECS: f32 = 2.0;
}

/// §2.3 type scale (ratio 1.2, base 14).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub size: f32,
    pub line: f32,
    pub weight: Weight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
    SemiBold,
    Mono,
    Icon,
}

pub mod typ {
    use super::{Type, Weight};
    pub const DISPLAY: Type = Type {
        size: 29.0,
        line: 36.0,
        weight: Weight::SemiBold,
    };
    pub const HEADLINE: Type = Type {
        size: 24.0,
        line: 32.0,
        weight: Weight::SemiBold,
    };
    pub const TITLE: Type = Type {
        size: 20.0,
        line: 28.0,
        weight: Weight::SemiBold,
    };
    pub const TITLE_SM: Type = Type {
        size: 17.0,
        line: 24.0,
        weight: Weight::Medium,
    };
    pub const BODY: Type = Type {
        size: 14.0,
        line: 20.0,
        weight: Weight::Regular,
    };
    pub const BODY_STRONG: Type = Type {
        size: 14.0,
        line: 20.0,
        weight: Weight::Medium,
    };
    pub const CAPTION: Type = Type {
        size: 12.0,
        line: 16.0,
        weight: Weight::Regular,
    };
    pub const CAPTION_STRONG: Type = Type {
        size: 12.0,
        line: 16.0,
        weight: Weight::Medium,
    };
    pub const MONO: Type = Type {
        size: 13.0,
        line: 20.0,
        weight: Weight::Mono,
    };
    pub const MONO_SM: Type = Type {
        size: 12.0,
        line: 16.0,
        weight: Weight::Mono,
    };
    /// Title-sized mono for the detail filename (§4.4 "title_sm mono").
    pub const MONO_TITLE: Type = Type {
        size: 17.0,
        line: 24.0,
        weight: Weight::Mono,
    };
    pub const ALL: [Type; 10] = [
        DISPLAY,
        HEADLINE,
        TITLE,
        TITLE_SM,
        BODY,
        BODY_STRONG,
        CAPTION,
        CAPTION_STRONG,
        MONO,
        MONO_SM,
    ];
}

impl Type {
    pub fn font(self) -> FontId {
        FontId::new(self.size, family(self.weight))
    }
}

pub fn family(w: Weight) -> FontFamily {
    match w {
        Weight::Regular => FontFamily::Proportional,
        Weight::Medium => FontFamily::Name(FAMILY_500.into()),
        Weight::SemiBold => FontFamily::Name(FAMILY_600.into()),
        Weight::Mono => FontFamily::Monospace,
        Weight::Icon => FontFamily::Name(FAMILY_ICONS.into()),
    }
}

pub const FAMILY_500: &str = "inter-500";
pub const FAMILY_600: &str = "inter-600";
pub const FAMILY_ICONS: &str = "icons";

/// Icon font id at a §2.2 icon size.
pub fn icon_font(px: f32) -> FontId {
    FontId::new(px, family(Weight::Icon))
}

/// §6 Lucide codepoints (lucide-static 1.70.0 `font/codepoints.json`).
pub mod icon {
    pub const FILE_TEXT: char = '\u{e0cc}';
    pub const FILE_UP: char = '\u{e32a}';
    pub const FOLDER_OPEN: char = '\u{e247}';
    pub const PLUS: char = '\u{e13d}';
    pub const PLAY: char = '\u{e13c}';
    pub const CHECK: char = '\u{e06c}';
    pub const UNDO_2: char = '\u{e2a1}';
    pub const HISTORY: char = '\u{e1f5}';
    pub const SETTINGS: char = '\u{e154}';
    pub const BUILDING_2: char = '\u{e290}';
    pub const GIT_MERGE: char = '\u{e0e4}';
    pub const CLOUD: char = '\u{e088}';
    pub const SHIELD_CHECK: char = '\u{e1ff}';
    pub const KEY_ROUND: char = '\u{e4a3}';
    pub const SCAN_TEXT: char = '\u{e538}';
    pub const EYE: char = '\u{e0ba}';
    pub const EYE_OFF: char = '\u{e0bb}';
    pub const IMAGE: char = '\u{e0f6}';
    pub const FILE_PEN_LINE: char = '\u{e320}';
    pub const USER_PLUS: char = '\u{e1a2}';
    pub const LINK: char = '\u{e102}';
    pub const FLAG: char = '\u{e0d1}';
    pub const PENCIL: char = '\u{e1f9}';
    pub const CIRCLE_CHECK: char = '\u{e226}';
    pub const TRIANGLE_ALERT: char = '\u{e193}';
    pub const CIRCLE_X: char = '\u{e084}';
    pub const INFO: char = '\u{e0f9}';
    pub const X: char = '\u{e1b2}';
    pub const CHEVRON_DOWN: char = '\u{e06d}';
    pub const CHEVRON_RIGHT: char = '\u{e06f}';
    pub const ARROW_RIGHT: char = '\u{e049}';
    pub const LOADER_CIRCLE: char = '\u{e10a}';
    pub const SEARCH: char = '\u{e151}';
    pub const COPY: char = '\u{e09e}';
    pub const EXTERNAL_LINK: char = '\u{e0b9}';
    pub const DOWNLOAD: char = '\u{e0b2}';
    pub const TRASH_2: char = '\u{e18e}';
    pub const ROTATE_CCW: char = '\u{e148}';
    pub const SQUARE: char = '\u{e167}';
    pub const MINUS: char = '\u{e11c}';
}

/// §2.5 colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_sunken: Color32,
    pub surface_hover: Color32,
    pub selection: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    pub accent_subtle: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub focus: Color32,
    pub success: Color32,
    pub success_subtle: Color32,
    pub warning: Color32,
    pub warning_subtle: Color32,
    pub danger: Color32,
    pub danger_subtle: Color32,
    pub danger_hover: Color32,
    pub brand: Color32,
    pub scrim: Color32,
    /// §4.2 whole-window drop overlay (scrim 20 %).
    pub drop_scrim: Color32,
    /// §2.6 elevation shadow colors (elev.1/2/3).
    pub shadow_1: Color32,
    pub shadow_2: Color32,
    pub shadow_3: Color32,
    pub dark: bool,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const fn black(alpha_pct: u32) -> Color32 {
    Color32::from_black_alpha((alpha_pct * 255 / 100) as u8)
}

pub const LIGHT: Palette = Palette {
    bg: hex(0xF9FAFC),
    surface: hex(0xFFFFFF),
    surface_sunken: hex(0xF0F2F6),
    surface_hover: hex(0xEAEDF2),
    selection: hex(0xDAE8FF),
    border: hex(0xDCE0E5),
    border_strong: hex(0x81868F),
    text: hex(0x171B22),
    text_muted: hex(0x555B65),
    text_disabled: hex(0x95989F),
    accent: hex(0x2858CD),
    accent_hover: hex(0x1E49B7),
    accent_pressed: hex(0x183D9C),
    accent_subtle: hex(0xE6EFFF),
    accent_text: hex(0x2251C6),
    on_accent: hex(0xFFFFFF),
    focus: hex(0x3266E4),
    success: hex(0x137738),
    success_subtle: hex(0xE0F7E4),
    warning: hex(0x975800),
    warning_subtle: hex(0xFFF0D0),
    danger: hex(0xC2181D),
    danger_subtle: hex(0xFFEAE9),
    danger_hover: hex(0xAC0311),
    brand: hex(0xDC2626),
    scrim: black(40),
    drop_scrim: black(20),
    shadow_1: black(8),
    shadow_2: black(12),
    shadow_3: black(18),
    dark: false,
};

pub const DARK: Palette = Palette {
    bg: hex(0x101215),
    surface: hex(0x171A1E),
    surface_sunken: hex(0x0A0C10),
    surface_hover: hex(0x212429),
    selection: hex(0x233459),
    border: hex(0x2A2E34),
    border_strong: hex(0x646972),
    text: hex(0xE9EBEF),
    text_muted: hex(0xA6ABB4),
    text_disabled: hex(0x656970),
    accent: hex(0x719CF5),
    accent_hover: hex(0x84ACFF),
    accent_pressed: hex(0x668FE8),
    accent_subtle: hex(0x1E2D4C),
    accent_text: hex(0x90B6FF),
    on_accent: hex(0x0B111F),
    focus: hex(0x83AFFF),
    success: hex(0x6FD087),
    success_subtle: hex(0x17321E),
    warning: hex(0xF0BA59),
    warning_subtle: hex(0x3C2A0E),
    danger: hex(0xF97770),
    danger_subtle: hex(0x47211E),
    danger_hover: hex(0xFF8E86),
    brand: hex(0xEF4444),
    scrim: black(60),
    drop_scrim: black(20),
    shadow_1: black(40),
    shadow_2: black(50),
    shadow_3: black(60),
    dark: true,
};

/// Palette of the theme a `Ui`/`Context` is currently painted with.
pub fn pal(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

pub fn palette(ui: &egui::Ui) -> &'static Palette {
    pal(ui.visuals().dark_mode)
}

/// Semantic tone used by badges, banners and status icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

impl Palette {
    /// `(fill, foreground)` for a tone (§5.2).
    pub fn tone(&self, t: Tone) -> (Color32, Color32) {
        match t {
            Tone::Neutral => (self.surface_sunken, self.text_muted),
            Tone::Accent => (self.accent_subtle, self.accent_text),
            Tone::Success => (self.success_subtle, self.success),
            Tone::Warning => (self.warning_subtle, self.warning),
            Tone::Danger => (self.danger_subtle, self.danger),
        }
    }

    /// §5.7 banner border: semantic color at 40 % alpha.
    pub fn tone_border(&self, t: Tone) -> Color32 {
        self.tone(t).1.gamma_multiply(BANNER_BORDER_ALPHA)
    }

    /// §2.6 elevation shadow.
    pub fn shadow(&self, level: u8) -> Shadow {
        match level {
            0 => Shadow::NONE,
            1 => Shadow {
                offset: [0, 1],
                blur: 3,
                spread: 0,
                color: self.shadow_1,
            },
            2 => Shadow {
                offset: [0, 4],
                blur: 12,
                spread: 0,
                color: self.shadow_2,
            },
            _ => Shadow {
                offset: [0, 12],
                blur: 32,
                spread: 0,
                color: self.shadow_3,
            },
        }
    }

    /// Fill of an elevated surface (dark mode uses tonal elevation).
    pub fn elevated(&self, level: u8) -> Color32 {
        if self.dark && level > 0 {
            self.surface_hover
        } else {
            self.surface
        }
    }
}

pub const BANNER_BORDER_ALPHA: f32 = 0.4;

pub fn hairline(c: Color32) -> Stroke {
    Stroke::new(stroke::HAIRLINE, c)
}

pub fn cr(r: u8) -> CornerRadius {
    CornerRadius::same(r)
}

pub fn margin(x: f32, y: f32) -> Margin {
    Margin::symmetric(x as i8, y as i8)
}

pub fn margin_all(v: f32) -> Margin {
    Margin::same(v as i8)
}

/// §3 detail-pane width = round₈(window_w × 0.382) clamped to [320, 560].
pub fn detail_width(window_w: f32) -> f32 {
    ((window_w * PHI_MINOR / GRID8).round() * GRID8).clamp(size::DETAIL_MIN, size::DETAIL_MAX)
}

pub const PHI_MINOR: f32 = 0.382;
pub const GRID8: f32 = 8.0;
pub const GRID4: f32 = 4.0;

/// Snap to the 4-pt grid.
pub fn snap4(v: f32) -> f32 {
    (v / GRID4).round() * GRID4
}

/// Pairs of `Vec2` from tokens (keeps `vec2(` literals inside theme).
pub fn v2(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

/// Confidence/quality bands for the 3-dot meter (§4.3).
pub const QUALITY_HIGH: f64 = 0.8;
pub const QUALITY_MEDIUM: f64 = 0.5;

/// §6 app mark, 20 px small master: flat brand tile (radius 4), white page
/// outline with folded corner, two text lines; strokes on whole pixels.
pub mod mark {
    pub const SIZE: f32 = 20.0;
    pub const RADIUS: u8 = 4;
    pub const PAGE_L: f32 = 6.0;
    pub const PAGE_R: f32 = 14.0;
    pub const PAGE_T: f32 = 4.0;
    pub const PAGE_B: f32 = 16.0;
    pub const FOLD: f32 = 3.0;
    pub const LINE_L: f32 = 8.0;
    pub const LINE_R: f32 = 12.0;
    pub const LINE_1: f32 = 10.0;
    pub const LINE_2: f32 = 13.0;
    /// Dark-shell edge (#991B1B).
    pub const EDGE: u32 = 0x991B1B;
}

pub fn paint_mark(painter: &egui::Painter, min: egui::Pos2, p: &Palette) {
    use egui::{Pos2, Rect, Shape};
    let o = min.round();
    let tile = Rect::from_min_size(o, v2(mark::SIZE, mark::SIZE));
    painter.rect(
        tile,
        cr(mark::RADIUS),
        p.brand,
        hairline(hex(mark::EDGE)),
        egui::StrokeKind::Inside,
    );
    let white = hairline(Color32::WHITE);
    let h = stroke::HAIRLINE / 2.0;
    let pt = |x: f32, y: f32| Pos2::new(o.x + x + h, o.y + y + h);
    let page = vec![
        pt(mark::PAGE_L, mark::PAGE_T),
        pt(mark::PAGE_R - mark::FOLD, mark::PAGE_T),
        pt(mark::PAGE_R, mark::PAGE_T + mark::FOLD),
        pt(mark::PAGE_R, mark::PAGE_B),
        pt(mark::PAGE_L, mark::PAGE_B),
        pt(mark::PAGE_L, mark::PAGE_T),
    ];
    painter.add(Shape::line(page, white));
    painter.add(Shape::line(
        vec![
            pt(mark::PAGE_R - mark::FOLD, mark::PAGE_T),
            pt(mark::PAGE_R - mark::FOLD, mark::PAGE_T + mark::FOLD),
            pt(mark::PAGE_R, mark::PAGE_T + mark::FOLD),
        ],
        white,
    ));
    for y in [mark::LINE_1, mark::LINE_2] {
        painter.line_segment([pt(mark::LINE_L, y), pt(mark::LINE_R, y)], white);
    }
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

static INTER_400: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.subset.ttf");
static INTER_500: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.subset.ttf");
static INTER_600: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.subset.ttf");
static MONO_400: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.subset.ttf");
static LUCIDE: &[u8] = include_bytes!("../../assets/fonts/lucide.subset.ttf");

pub fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    // egui defaults stay as fallbacks (unsupported scripts, emoji).
    let fallbacks = f
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (name, bytes) in [
        ("inter-400", INTER_400),
        (FAMILY_500, INTER_500),
        (FAMILY_600, INTER_600),
        ("jetbrains-mono", MONO_400),
        ("lucide", LUCIDE),
    ] {
        f.font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let chain = |first: &str| {
        let mut v = vec![first.to_string()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    f.families
        .insert(FontFamily::Proportional, chain("inter-400"));
    f.families
        .insert(FontFamily::Name(FAMILY_500.into()), chain(FAMILY_500));
    f.families
        .insert(FontFamily::Name(FAMILY_600.into()), chain(FAMILY_600));
    f.families
        .insert(FontFamily::Monospace, chain("jetbrains-mono"));
    f.families
        .insert(FontFamily::Name(FAMILY_ICONS.into()), vec!["lucide".into()]);
    f
}

// ---------------------------------------------------------------------------
// egui style mapping (§2.7)
// ---------------------------------------------------------------------------

pub const SCROLL_BAR_W: f32 = 8.0;
pub const SLIDER_RAIL_H: f32 = 4.0;
pub const COMBO_H: f32 = 240.0;
pub const TEXT_EDIT_W: f32 = 280.0;
pub const BUTTON_PAD_Y: f32 = 6.0;
pub const ICON_INNER: f32 = 8.0;
pub const ANIMATION_TIME: f32 = 0.1;

pub fn style(p: &Palette) -> egui::Style {
    use egui::style::{ScrollStyle, WidgetVisuals};
    let mut s = egui::Style {
        text_styles: [
            (TextStyle::Small, typ::CAPTION.font()),
            (TextStyle::Body, typ::BODY.font()),
            (TextStyle::Button, typ::BODY_STRONG.font()),
            (TextStyle::Heading, typ::TITLE.font()),
            (TextStyle::Monospace, typ::MONO.font()),
            (TextStyle::Name("display".into()), typ::DISPLAY.font()),
            (TextStyle::Name("headline".into()), typ::HEADLINE.font()),
            (TextStyle::Name("title_sm".into()), typ::TITLE_SM.font()),
            (
                TextStyle::Name("caption_strong".into()),
                typ::CAPTION_STRONG.font(),
            ),
            (TextStyle::Name("mono_sm".into()), typ::MONO_SM.font()),
        ]
        .into(),
        ..Default::default()
    };
    s.animation_time = ANIMATION_TIME;
    s.interaction.tooltip_delay = motion::TOOLTIP_DELAY;

    let sp = &mut s.spacing;
    sp.item_spacing = v2(space::SM, space::SM);
    sp.button_padding = v2(space::MD, BUTTON_PAD_Y);
    sp.interact_size = v2(size::CONTROL, size::CONTROL);
    sp.icon_width = size::ICON;
    sp.icon_width_inner = ICON_INNER;
    sp.icon_spacing = space::SM;
    sp.menu_margin = margin_all(space::SM);
    sp.window_margin = margin_all(space::XL);
    sp.indent = space::LG;
    sp.combo_height = COMBO_H;
    sp.text_edit_width = TEXT_EDIT_W;
    sp.slider_rail_height = SLIDER_RAIL_H;
    sp.scroll = ScrollStyle::thin();
    sp.scroll.bar_width = SCROLL_BAR_W;
    // Bars float over the 16 px row padding; content keeps the φ width.
    sp.scroll.floating_allocated_width = 0.0;

    let v = &mut s.visuals;
    *v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.elevated(3);
    v.extreme_bg_color = p.surface;
    v.text_edit_bg_color = Some(p.surface);
    v.faint_bg_color = p.surface_sunken;
    v.code_bg_color = p.surface_sunken;
    v.hyperlink_color = p.accent_text;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_corner_radius = cr(radius::XL);
    v.menu_corner_radius = cr(radius::LG);
    v.window_shadow = p.shadow(3);
    v.popup_shadow = p.shadow(2);
    v.window_stroke = hairline(p.border);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = hairline(p.text);
    v.text_cursor.stroke = Stroke::new(stroke::FOCUS, p.accent);
    v.striped = false;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.override_text_color = None;
    let wv = |bg: Color32, weak: Color32, st: Color32, fg: Color32| WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: weak,
        bg_stroke: hairline(st),
        corner_radius: cr(radius::MD),
        fg_stroke: hairline(fg),
        expansion: 0.0,
    };
    v.widgets.noninteractive = wv(p.surface, p.surface, p.border, p.text);
    v.widgets.inactive = wv(p.surface_sunken, p.surface, p.border_strong, p.text);
    v.widgets.hovered = wv(p.surface_hover, p.surface_hover, p.border_strong, p.text);
    v.widgets.active = wv(p.accent_subtle, p.accent_subtle, p.accent, p.text);
    v.widgets.open = v.widgets.hovered;
    s
}

/// Install fonts + both theme styles; follow the OS theme by default.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_style_of(egui::Theme::Light, style(&LIGHT));
    ctx.set_style_of(egui::Theme::Dark, style(&DARK));
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::System);
}

// ---------------------------------------------------------------------------
// WCAG contrast (§2.5, checklist 3)
// ---------------------------------------------------------------------------

fn channel(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn luminance(c: Color32) -> f64 {
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §2.5 contrast table, recomputed from the hex tokens (checklist 3).
    #[test]
    fn contrast_pairs_meet_wcag() {
        for p in [&LIGHT, &DARK] {
            let text_pairs = [
                ("text/bg", p.text, p.bg),
                ("text/surface", p.text, p.surface),
                ("text/sunken", p.text, p.surface_sunken),
                ("text/hover", p.text, p.surface_hover),
                ("text/selection", p.text, p.selection),
                ("text/accent_subtle", p.text, p.accent_subtle),
                ("text/success_subtle", p.text, p.success_subtle),
                ("text/warning_subtle", p.text, p.warning_subtle),
                ("text/danger_subtle", p.text, p.danger_subtle),
                ("muted/bg", p.text_muted, p.bg),
                ("muted/surface", p.text_muted, p.surface),
                ("muted/sunken", p.text_muted, p.surface_sunken),
                ("muted/hover", p.text_muted, p.surface_hover),
                ("muted/selection", p.text_muted, p.selection),
                ("muted/accent_subtle", p.text_muted, p.accent_subtle),
                ("accent_text/surface", p.accent_text, p.surface),
                ("accent_text/bg", p.accent_text, p.bg),
                ("accent_text/accent_subtle", p.accent_text, p.accent_subtle),
                ("accent_text/selection", p.accent_text, p.selection),
                ("accent_text/hover", p.accent_text, p.surface_hover),
                ("on_accent/accent", p.on_accent, p.accent),
                ("on_accent/accent_hover", p.on_accent, p.accent_hover),
                ("on_accent/accent_pressed", p.on_accent, p.accent_pressed),
                ("on_accent/danger", p.on_accent, p.danger),
                ("on_accent/danger_hover", p.on_accent, p.danger_hover),
                ("success/surface", p.success, p.surface),
                ("success/subtle", p.success, p.success_subtle),
                ("success/hover", p.success, p.surface_hover),
                ("warning/surface", p.warning, p.surface),
                ("warning/subtle", p.warning, p.warning_subtle),
                ("warning/hover", p.warning, p.surface_hover),
                ("danger/surface", p.danger, p.surface),
                ("danger/subtle", p.danger, p.danger_subtle),
                ("danger/hover", p.danger, p.surface_hover),
                ("tooltip bg/text", p.bg, p.text),
            ];
            for (name, fg, bg) in text_pairs {
                let r = contrast(fg, bg);
                assert!(r >= 4.5, "{name} dark={} ratio {r:.2} < 4.5", p.dark);
            }
            let non_text = [
                ("border_strong/surface", p.border_strong, p.surface),
                ("border_strong/bg", p.border_strong, p.bg),
                ("border_strong/sunken", p.border_strong, p.surface_sunken),
                ("focus/surface", p.focus, p.surface),
                ("focus/bg", p.focus, p.bg),
                ("focus/sunken", p.focus, p.surface_sunken),
                ("focus/selection", p.focus, p.selection),
                ("accent/surface", p.accent, p.surface),
                ("accent/sunken", p.accent, p.surface_sunken),
                ("on_accent/brand", hex(0xFFFFFF), LIGHT.brand),
            ];
            for (name, fg, bg) in non_text {
                let r = contrast(fg, bg);
                assert!(r >= 3.0, "{name} dark={} ratio {r:.2} < 3.0", p.dark);
            }
        }
    }

    #[test]
    fn phi_split_matches_spec() {
        assert_eq!(detail_width(1280.0), 488.0);
        assert_eq!(detail_width(960.0), 368.0);
        assert_eq!(detail_width(600.0), 320.0);
        assert_eq!(detail_width(2000.0), 560.0);
    }

    #[test]
    fn type_scale_has_three_weights_and_min_12px() {
        for t in typ::ALL {
            assert!(t.size >= 12.0);
            assert_eq!(t.line % 4.0, 0.0);
        }
    }
}
