//! One-page test PDFs built with PDFium: a clean text layer plus chosen
//! non-text content, for the visual cap (rebuild spec §Pipeline 1).

use std::path::{Path, PathBuf};

use autorename_core::extract::pdfium;
use pdfium_render::prelude::*;

/// Text lines of the generated invoice page (clean text layer).
pub const INVOICE: &[&str] = &[
    "INVOICE INV-2025-0815",
    "Date: 12.05.2025",
    "Bill to: Spqrk GmbH, Hauptstrasse 1, 1010 Wien",
    "Consulting services May 2025 EUR 1,200.00",
    "Total EUR 1,200.00",
    "Payable within 14 days.",
];

/// Non-text content of the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Extra {
    /// Raster logo (top left, 180 × 40 pt) showing this name in large type.
    RasterLogo(&'static str),
    /// Image of 10 × 10 pt (below 0.1 % of an A4 page).
    TinyImage,
    /// Filled circle of 40 pt diameter (a vector logo, Bézier curves).
    VectorLogo,
    /// Stroke-only circle of 40 pt diameter.
    StrokedCurve,
    /// Table: shaded header cell, hairline rules, a stroked grid line and
    /// one path with three cell rectangles.
    Table,
    /// White rectangle covering the whole page, behind everything.
    Background,
    /// A white rectangle over the whole page hides the text layer.
    HiddenText,
}

/// Render `name` in 28 pt Helvetica on a 180 × 40 pt page, as an image.
fn logo_image(p: &Pdfium, name: &str) -> image::DynamicImage {
    let mut doc = p.create_new_pdf().unwrap();
    let font = doc.fonts_mut().helvetica_bold();
    let mut page = doc
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::Custom(
            PdfPoints::new(180.0),
            PdfPoints::new(40.0),
        ))
        .unwrap();
    page.objects_mut()
        .create_text_object(
            PdfPoints::new(6.0),
            PdfPoints::new(12.0),
            name,
            font,
            PdfPoints::new(20.0),
        )
        .unwrap();
    let cfg = PdfRenderConfig::new()
        .set_target_width(720)
        .set_target_height(160)
        .set_clear_color(PdfColor::WHITE);
    page.render_with_config(&cfg).unwrap().as_image().unwrap()
}

/// Write `<dir>/<name>.pdf`: one A4 page with `lines` as text plus `extras`.
pub fn page_pdf(dir: &Path, name: &str, lines: &[&str], extras: &[Extra]) -> PathBuf {
    let bytes = pdfium::with_pdfium(|p| {
        let logo = extras.iter().find_map(|e| match e {
            Extra::RasterLogo(n) => Some(logo_image(p, n)),
            _ => None,
        });
        let mut doc = p.create_new_pdf().unwrap();
        let font = doc.fonts_mut().helvetica();
        let mut page = doc
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::a4())
            .unwrap();
        let pt = PdfPoints::new;
        let full = PdfRect::new_from_values(0.0, 0.0, page.height().value, page.width().value);
        if extras.contains(&Extra::Background) {
            page.objects_mut()
                .create_path_object_rect(full, None, None, Some(PdfColor::WHITE))
                .unwrap();
        }
        for (i, line) in lines.iter().enumerate() {
            page.objects_mut()
                .create_text_object(pt(56.0), pt(700.0 - 18.0 * i as f32), *line, font, pt(11.0))
                .unwrap();
        }
        for e in extras {
            match e {
                Extra::RasterLogo(_) => {
                    let img = logo.as_ref().unwrap();
                    page.objects_mut()
                        .create_image_object(
                            pt(56.0),
                            pt(760.0),
                            img,
                            Some(pt(180.0)),
                            Some(pt(40.0)),
                        )
                        .unwrap();
                }
                Extra::TinyImage => {
                    let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                        20,
                        20,
                        image::Rgb([200, 30, 30]),
                    ));
                    page.objects_mut()
                        .create_image_object(
                            pt(500.0),
                            pt(780.0),
                            &img,
                            Some(pt(10.0)),
                            Some(pt(10.0)),
                        )
                        .unwrap();
                }
                Extra::VectorLogo => {
                    page.objects_mut()
                        .create_path_object_circle_at(
                            pt(500.0),
                            pt(780.0),
                            pt(20.0),
                            None,
                            None,
                            Some(PdfColor::new(20, 60, 160, 255)),
                        )
                        .unwrap();
                }
                Extra::StrokedCurve => {
                    page.objects_mut()
                        .create_path_object_circle_at(
                            pt(500.0),
                            pt(780.0),
                            pt(20.0),
                            Some(PdfColor::BLACK),
                            Some(pt(1.0)),
                            None,
                        )
                        .unwrap();
                }
                Extra::Table => {
                    let grey = Some(PdfColor::new(230, 230, 230, 255));
                    let header = PdfRect::new_from_values(560.0, 50.0, 580.0, 545.0);
                    page.objects_mut()
                        .create_path_object_rect(header, None, None, grey)
                        .unwrap();
                    for y in [540.0, 520.0, 500.0] {
                        let rule = PdfRect::new_from_values(y, 50.0, y + 0.5, 545.0);
                        page.objects_mut()
                            .create_path_object_rect(rule, None, None, Some(PdfColor::BLACK))
                            .unwrap();
                    }
                    page.objects_mut()
                        .create_path_object_line(
                            pt(300.0),
                            pt(500.0),
                            pt(300.0),
                            pt(580.0),
                            PdfColor::BLACK,
                            pt(0.5),
                        )
                        .unwrap();
                    // Three cells in one path: 12 straight segments, all rectangles.
                    let mut cells =
                        PdfPagePathObject::new(&doc, pt(50.0), pt(440.0), None, None, grey)
                            .unwrap();
                    for (i, x) in [50.0f32, 200.0, 350.0].into_iter().enumerate() {
                        if i > 0 {
                            cells.move_to(pt(x), pt(440.0)).unwrap();
                        }
                        cells.line_to(pt(x + 140.0), pt(440.0)).unwrap();
                        cells.line_to(pt(x + 140.0), pt(480.0)).unwrap();
                        cells.line_to(pt(x), pt(480.0)).unwrap();
                        cells.close_path().unwrap();
                    }
                    page.objects_mut().add_path_object(cells).unwrap();
                }
                Extra::HiddenText => {
                    page.objects_mut()
                        .create_path_object_rect(full, None, None, Some(PdfColor::WHITE))
                        .unwrap();
                }
                Extra::Background => {}
            }
        }
        drop(page);
        doc.save_to_bytes().unwrap()
    })
    .unwrap();
    let path = dir.join(format!("{name}.pdf"));
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Write `<dir>/<name>.pdf` (lopdf): one A4 page with a line of text and a
/// 40 × 40 pt image inside `depth` nested form XObjects; the outermost form
/// is drawn scaled by `scale`.
pub fn form_image_pdf(dir: &Path, name: &str, scale: f32, depth: usize) -> PathBuf {
    use lopdf::{Document, Object, Stream, dictionary};
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0, 255, 255, 0],
    ));
    // Innermost form draws the image; each outer form draws the next one.
    let mut inner = (
        b"Im0".to_vec(),
        image,
        b"q 40 0 0 40 100 100 cm /Im0 Do Q".to_vec(),
    );
    for _ in 0..depth {
        let (res, id, content) = inner;
        let res_name = String::from_utf8(res.clone()).unwrap();
        let form = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Resources" => dictionary! { "XObject" => dictionary! { res_name.as_str() => id } },
            },
            content,
        ));
        inner = (b"Fm0".to_vec(), form, b"/Fm0 Do".to_vec());
    }
    let (res, id, _) = inner;
    let res_name = String::from_utf8(res).unwrap();
    let content = format!(
        "BT /F1 11 Tf 56 700 Td (Invoice INV-2025-0815 for Spqrk GmbH) Tj ET q {scale} 0 0 {scale} 0 0 cm /{res_name} Do Q"
    );
    let contents = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Contents" => contents,
        "Resources" => dictionary! {
            "Font" => dictionary! { "F1" => font },
            "XObject" => dictionary! { res_name.as_str() => id },
        },
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let path = dir.join(format!("{name}.pdf"));
    doc.save(&path).unwrap();
    path
}
