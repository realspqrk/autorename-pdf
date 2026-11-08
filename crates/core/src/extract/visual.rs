//! Significant non-text visuals on a page (pictures, drawings): the input
//! of the visual cap in the per-page score (rebuild spec §Pipeline 1).
//!
//! A clean text layer says nothing about what is printed only as a picture
//! (a logo with the issuer's name, a stamp, a scan behind the text). A page
//! with such a visual therefore scores at most [`VISUAL_CAP`]; a readability
//! limit above it reads the page with OCR.
//!
//! All sizes are displayed bounds in page space, clipped to the crop box
//! (area `A`):
//! - an image object (also inside form XObjects) counts when its area is
//!   ≥ 0.001·A and both sides are ≥ 2 pt. Full-page images count (scans).
//! - filled paths count together: the summed area of qualifying paths must
//!   reach 0.001·A. A path qualifies when it is filled, not fully
//!   transparent, its minor extent is > 1 pt, and it has a Bézier segment
//!   or ≥ 6 non-MoveTo segments, unless all its subpaths are axis-aligned
//!   rectangles (table borders, cell shading, backgrounds). Stroke-only
//!   paths never count.
//! - shading and unsupported objects count when their area is ≥ 0.001·A.
//!
//! Form XObjects are walked recursively (depth ≤ 32, ≤ 100 000 objects plus
//! path segments per page). Exhausting a limit or a PDFium error counts as
//! a visual, with a warning: the safe side is reading the page again.

use pdfium_render::prelude::*;

/// Highest text-layer score of a page with a significant visual.
pub const VISUAL_CAP: f64 = 0.75;

/// Minimum share of the page area a visual must cover.
const MIN_AREA_SHARE: f32 = 0.001;
/// Minimum side of an image, in points.
const MIN_IMAGE_SIDE: f32 = 2.0;
/// A filled path must be wider than this in both directions (hairlines).
const MIN_PATH_EXTENT: f32 = 1.0;
/// Straight-line paths with fewer segments are simple shapes (rules, boxes).
const MIN_PLAIN_SEGMENTS: usize = 6;
const MAX_DEPTH: usize = 32;
const MAX_ITEMS: usize = 100_000;
/// Tolerance for "horizontal or vertical", in points.
const AXIS_EPS: f32 = 0.01;

/// Text-layer score of a page: readability `r`, capped when `visual`.
pub fn capped(r: f64, visual: bool) -> f64 {
    if visual { r.min(VISUAL_CAP) } else { r }
}

/// Result of [`scan_page`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// The page has a significant non-text visual.
    pub visual: bool,
    /// Why a limit or PDFium error forced `visual`.
    pub warning: Option<String>,
}

/// Affine map `(x, y) ↦ (a·x + c·y + e, b·x + d·y + f)` (PDF convention).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn of(m: &PdfMatrix) -> Self {
        Affine {
            a: m.a(),
            b: m.b(),
            c: m.c(),
            d: m.d(),
            e: m.e(),
            f: m.f(),
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// `self` first, then `outer`.
    fn then(&self, o: &Affine) -> Affine {
        Affine {
            a: o.a * self.a + o.c * self.b,
            b: o.b * self.a + o.d * self.b,
            c: o.a * self.c + o.c * self.d,
            d: o.b * self.c + o.d * self.d,
            e: o.a * self.e + o.c * self.f + o.e,
            f: o.b * self.e + o.d * self.f + o.f,
        }
    }
}

/// Axis-aligned box `l ≤ r`, `b ≤ t`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Box2 {
    l: f32,
    b: f32,
    r: f32,
    t: f32,
}

impl Box2 {
    fn of_points(pts: impl IntoIterator<Item = (f32, f32)>) -> Self {
        let mut bx = Box2 {
            l: f32::INFINITY,
            b: f32::INFINITY,
            r: f32::NEG_INFINITY,
            t: f32::NEG_INFINITY,
        };
        for (x, y) in pts {
            bx.l = bx.l.min(x);
            bx.r = bx.r.max(x);
            bx.b = bx.b.min(y);
            bx.t = bx.t.max(y);
        }
        bx
    }

    fn of_rect(r: &PdfRect) -> Self {
        Box2::of_points([
            (r.left().value, r.bottom().value),
            (r.right().value, r.top().value),
        ])
    }

    fn clip(&self, o: &Box2) -> Box2 {
        Box2 {
            l: self.l.max(o.l),
            b: self.b.max(o.b),
            r: self.r.min(o.r),
            t: self.t.min(o.t),
        }
    }

    fn width(&self) -> f32 {
        (self.r - self.l).max(0.0)
    }

    fn height(&self) -> f32 {
        (self.t - self.b).max(0.0)
    }

    fn area(&self) -> f32 {
        self.width() * self.height()
    }
}

struct Scanner {
    crop: Box2,
    min_area: f32,
    items: usize,
    path_area: f32,
    found: bool,
}

type Step = std::result::Result<(), String>;

fn err(e: PdfiumError) -> String {
    format!("{e:?}")
}

impl Scanner {
    fn count(&mut self, n: usize) -> Step {
        self.items += n;
        if self.items > MAX_ITEMS {
            return Err(format!("more than {MAX_ITEMS} objects"));
        }
        Ok(())
    }

    /// Displayed bounds of `obj` (container space → page via `ctm`), clipped.
    fn shown(&self, obj: &PdfPageObject, ctm: &Affine) -> std::result::Result<Box2, String> {
        let q = obj.bounds().map_err(err)?;
        let corners = [
            (q.x1().value, q.y1().value),
            (q.x2().value, q.y2().value),
            (q.x3().value, q.y3().value),
            (q.x4().value, q.y4().value),
        ];
        Ok(Box2::of_points(corners.map(|(x, y)| ctm.apply(x, y))).clip(&self.crop))
    }

    fn walk(&mut self, obj: &PdfPageObject, ctm: &Affine, depth: usize) -> Step {
        self.count(1)?;
        match obj.object_type() {
            PdfPageObjectType::Text => {}
            PdfPageObjectType::Image => {
                let bx = self.shown(obj, ctm)?;
                if bx.area() >= self.min_area
                    && bx.width() >= MIN_IMAGE_SIDE
                    && bx.height() >= MIN_IMAGE_SIDE
                {
                    self.found = true;
                }
            }
            PdfPageObjectType::Path => {
                if let Some(path) = obj.as_path_object() {
                    self.path(obj, path, ctm)?;
                }
            }
            PdfPageObjectType::XObjectForm => {
                if depth >= MAX_DEPTH {
                    return Err(format!("form XObjects nested deeper than {MAX_DEPTH}"));
                }
                let Some(form) = obj.as_x_object_form_object() else {
                    return Ok(());
                };
                let inner = Affine::of(&obj.matrix().map_err(err)?).then(ctm);
                for i in 0..form.len() {
                    let child = form.get(i).map_err(err)?;
                    self.walk(&child, &inner, depth + 1)?;
                    if self.found {
                        break;
                    }
                }
            }
            PdfPageObjectType::Shading | PdfPageObjectType::Unsupported => {
                if self.shown(obj, ctm)?.area() >= self.min_area {
                    self.found = true;
                }
            }
        }
        Ok(())
    }

    fn path(&mut self, obj: &PdfPageObject, path: &PdfPagePathObject, ctm: &Affine) -> Step {
        if path.fill_mode().map_err(err)? == PdfPathFillMode::None {
            return Ok(());
        }
        // A fill color PDFium cannot report (patterns) counts as visible.
        if obj.fill_color().is_ok_and(|c| c.alpha() == 0) {
            return Ok(());
        }
        let bx = self.shown(obj, ctm)?;
        if bx.width().min(bx.height()) <= MIN_PATH_EXTENT || bx.area() == 0.0 {
            return Ok(());
        }
        let segs = path.segments();
        let n = segs.len() as usize;
        self.count(n)?;
        let full = Affine::of(&path.matrix().map_err(err)?).then(ctm);
        let mut bezier = false;
        let mut plain = 0usize;
        let mut subpaths: Vec<Vec<(f32, f32)>> = Vec::new();
        for i in 0..n {
            let s = segs.get(i as PdfPathSegmentIndex).map_err(err)?;
            let (x, y) = s.point();
            let pt = full.apply(x.value, y.value);
            match s.segment_type() {
                PdfPathSegmentType::MoveTo => subpaths.push(vec![pt]),
                PdfPathSegmentType::BezierTo => bezier = true,
                _ => {
                    plain += 1;
                    match subpaths.last_mut() {
                        Some(sp) => sp.push(pt),
                        None => subpaths.push(vec![pt]),
                    }
                }
            }
        }
        let complex = bezier || plain >= MIN_PLAIN_SEGMENTS;
        if !complex || (!bezier && all_rectangles(&subpaths)) {
            return Ok(());
        }
        self.path_area += bx.area();
        if self.path_area >= self.min_area {
            self.found = true;
        }
        Ok(())
    }
}

/// Every subpath with an edge is an axis-aligned rectangle (4 corners,
/// edges horizontal or vertical, the closing point may repeat the first).
fn all_rectangles(subpaths: &[Vec<(f32, f32)>]) -> bool {
    let close =
        |p: (f32, f32), q: (f32, f32)| (p.0 - q.0).abs() < AXIS_EPS && (p.1 - q.1).abs() < AXIS_EPS;
    let mut any = false;
    for sp in subpaths.iter().filter(|sp| sp.len() > 1) {
        let mut pts = sp.as_slice();
        if pts.len() > 1 && close(pts[0], pts[pts.len() - 1]) {
            pts = &pts[..pts.len() - 1];
        }
        if pts.len() != 4 {
            return false;
        }
        let axis = (0..4).all(|i| {
            let (p, q) = (pts[i], pts[(i + 1) % 4]);
            (p.0 - q.0).abs() < AXIS_EPS || (p.1 - q.1).abs() < AXIS_EPS
        });
        if !axis {
            return false;
        }
        any = true;
    }
    any
}

/// Crop box of `page` (media box, then the page size as fallbacks).
fn crop_box(page: &PdfPage) -> Box2 {
    let b = page.boundaries();
    match b.crop().or_else(|_| b.media()) {
        Ok(bb) => Box2::of_rect(&bb.bounds),
        Err(_) => Box2 {
            l: 0.0,
            b: 0.0,
            r: page.width().value,
            t: page.height().value,
        },
    }
}

/// Does `page` show a significant non-text visual? Call inside
/// [`super::pdfium::with_pdfium`].
pub fn scan_page(page: &PdfPage) -> Scan {
    let crop = crop_box(page);
    let mut s = Scanner {
        crop,
        min_area: MIN_AREA_SHARE * crop.area(),
        items: 0,
        path_area: 0.0,
        found: false,
    };
    let objects = page.objects();
    let mut result = Ok(());
    for i in 0..objects.len() {
        result = objects
            .get(i)
            .map_err(err)
            .and_then(|o| s.walk(&o, &Affine::IDENTITY, 0));
        if result.is_err() || s.found {
            break;
        }
    }
    match result {
        Ok(()) => Scan {
            visual: s.found,
            warning: None,
        },
        Err(e) => Scan {
            visual: true,
            warning: Some(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_applies_only_to_visual_pages() {
        assert_eq!(capped(1.0, true), VISUAL_CAP);
        assert_eq!(capped(0.4, true), 0.4);
        assert_eq!(capped(1.0, false), 1.0);
    }

    #[test]
    fn affine_composition_applies_inner_first() {
        let scale = Affine {
            a: 2.0,
            d: 2.0,
            ..Affine::IDENTITY
        };
        let shift = Affine {
            e: 10.0,
            f: 5.0,
            ..Affine::IDENTITY
        };
        assert_eq!(scale.then(&shift).apply(1.0, 1.0), (12.0, 7.0));
        assert_eq!(shift.then(&scale).apply(1.0, 1.0), (22.0, 12.0));
    }

    #[test]
    fn rectangle_subpaths() {
        let rect = |x: f32, y: f32| {
            vec![
                (x, y),
                (x + 5.0, y),
                (x + 5.0, y + 3.0),
                (x, y + 3.0),
                (x, y),
            ]
        };
        assert!(all_rectangles(&[rect(0.0, 0.0), rect(10.0, 0.0)]));
        // Implicitly closed rectangle (4 corners).
        assert!(all_rectangles(&[vec![
            (0.0, 0.0),
            (5.0, 0.0),
            (5.0, 3.0),
            (0.0, 3.0)
        ]]));
        // A diamond and a hexagon are drawings.
        assert!(!all_rectangles(&[vec![
            (0.0, 1.0),
            (1.0, 0.0),
            (2.0, 1.0),
            (1.0, 2.0)
        ]]));
        let hex = vec![
            (0.0, 0.0),
            (2.0, 0.0),
            (3.0, 1.0),
            (2.0, 2.0),
            (0.0, 2.0),
            (-1.0, 1.0),
        ];
        assert!(!all_rectangles(&[rect(0.0, 0.0), hex]));
        assert!(!all_rectangles(&[]));
    }
}
