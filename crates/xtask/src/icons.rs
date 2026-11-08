//! `cargo xtask icons [--check]`: render the app icon set from the SVG
//! masters in `assets/icons/` into `assets/icons/generated/` (checked in).
//!
//! Sources (design spec §6): 16/20/24/32 px use the hand-hinted small
//! masters `app-<n>.svg`; 22 px is the 20 px master with a 1 px margin
//! (hicolor 22); everything ≥ 40 px uses `app.svg`; the macOS `.icns`
//! uses `app-macos.svg` from 64 px up. Output is byte-deterministic, so
//! `--check` (CI) fails when the checked-in files are stale.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use resvg::{tiny_skia, usvg};

/// PNG sizes in `generated/png/`.
const PNG_SIZES: &[u32] = &[16, 20, 22, 24, 32, 40, 48, 64, 128, 256, 512, 1024];
/// Windows app icon (Explorer picks 16/20/24/32 at 100/125/150/200 %).
const ICO_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48, 64, 256];
/// Context-menu verb icon (small masters only).
const MENU_ICO_SIZES: &[u32] = &[16, 20, 24, 32];
/// freedesktop hicolor `apps/autorename-pdf.png` sizes.
const HICOLOR_SIZES: &[u32] = &[16, 22, 24, 32, 48, 64, 128, 256, 512];

const ICON_NAME: &str = "autorename-pdf";

struct Masters {
    small: BTreeMap<u32, usvg::Tree>,
    app: usvg::Tree,
    app_svg: Vec<u8>,
    macos: usvg::Tree,
}

fn load(path: &Path) -> Result<(usvg::Tree, Vec<u8>), String> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((tree, data))
}

fn masters(dir: &Path) -> Result<Masters, String> {
    let mut small = BTreeMap::new();
    for n in [16, 20, 24, 32] {
        small.insert(n, load(&dir.join(format!("app-{n}.svg")))?.0);
    }
    let (app, app_svg) = load(&dir.join("app.svg"))?;
    let macos = load(&dir.join("app-macos.svg"))?.0;
    Ok(Masters {
        small,
        app,
        app_svg,
        macos,
    })
}

/// Render `tree` scaled to `inner` px, placed at (`pad`, `pad`) on a
/// `size` px transparent canvas. Returns the pixmap (premultiplied).
fn render(tree: &usvg::Tree, size: u32, inner: u32, pad: u32) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("non-zero icon size");
    let s = inner as f32 / tree.size().width();
    let t = tiny_skia::Transform::from_scale(s, s).post_translate(pad as f32, pad as f32);
    resvg::render(tree, t, &mut pixmap.as_mut());
    pixmap
}

fn app_icon(m: &Masters, size: u32) -> tiny_skia::Pixmap {
    match size {
        22 => render(&m.small[&20], 22, 20, 1),
        n if m.small.contains_key(&n) => render(&m.small[&n], n, n, 0),
        n => render(&m.app, n, n, 0),
    }
}

fn mac_icon(m: &Masters, size: u32) -> tiny_skia::Pixmap {
    if size < 64 {
        app_icon(m, size)
    } else {
        render(&m.macos, size, size, 0)
    }
}

/// Straight (non-premultiplied) RGBA bytes.
fn rgba(p: &tiny_skia::Pixmap) -> Vec<u8> {
    p.pixels()
        .iter()
        .flat_map(|c| {
            let c = c.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

fn png(p: &tiny_skia::Pixmap) -> Result<Vec<u8>, String> {
    p.encode_png().map_err(|e| e.to_string())
}

fn ico(m: &Masters, sizes: &[u32]) -> Result<Vec<u8>, String> {
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for &n in sizes {
        let img = ico::IconImage::from_rgba_data(n, n, rgba(&app_icon(m, n)));
        // PNG-compressed 256 (Vista+), BMP below (fastest for the shell).
        let entry = if n >= 256 {
            ico::IconDirEntry::encode_as_png(&img)
        } else {
            ico::IconDirEntry::encode_as_bmp(&img)
        }
        .map_err(|e| e.to_string())?;
        dir.add_entry(entry);
    }
    let mut out = Vec::new();
    dir.write(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn icns(m: &Masters) -> Result<Vec<u8>, String> {
    use icns::IconType as T;
    let types = [
        (T::RGBA32_16x16, 16),
        (T::RGBA32_16x16_2x, 32),
        (T::RGBA32_32x32, 32),
        (T::RGBA32_32x32_2x, 64),
        (T::RGBA32_128x128, 128),
        (T::RGBA32_128x128_2x, 256),
        (T::RGBA32_256x256, 256),
        (T::RGBA32_256x256_2x, 512),
        (T::RGBA32_512x512, 512),
        (T::RGBA32_512x512_2x, 1024),
    ];
    let mut fam = icns::IconFamily::new();
    for (t, n) in types {
        let img = icns::Image::from_data(icns::PixelFormat::RGBA, n, n, rgba(&mac_icon(m, n)))
            .map_err(|e| e.to_string())?;
        fam.add_icon_with_type(&img, t).map_err(|e| e.to_string())?;
    }
    let mut out = Vec::new();
    fam.write(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Every generated file: path relative to `generated/` → bytes.
fn outputs(m: &Masters) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let mut out = BTreeMap::new();
    for &n in PNG_SIZES {
        out.insert(
            PathBuf::from(format!("png/{ICON_NAME}-{n}.png")),
            png(&app_icon(m, n))?,
        );
    }
    for &n in HICOLOR_SIZES {
        out.insert(
            PathBuf::from(format!("hicolor/{n}x{n}/apps/{ICON_NAME}.png")),
            png(&app_icon(m, n))?,
        );
    }
    out.insert(
        PathBuf::from(format!("hicolor/scalable/apps/{ICON_NAME}.svg")),
        m.app_svg.clone(),
    );
    out.insert(
        PathBuf::from(format!("{ICON_NAME}.ico")),
        ico(m, ICO_SIZES)?,
    );
    out.insert(PathBuf::from("context-menu.ico"), ico(m, MENU_ICO_SIZES)?);
    for &n in MENU_ICO_SIZES {
        out.insert(
            PathBuf::from(format!("context-menu/{ICON_NAME}-{n}.png")),
            png(&app_icon(m, n))?,
        );
    }
    out.insert(PathBuf::from(format!("{ICON_NAME}.icns")), icns(m)?);
    Ok(out)
}

pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let src = root.join("assets/icons");
    let gen_dir = src.join("generated");
    let files = outputs(&masters(&src)?)?;
    let mut stale = Vec::new();
    for (rel, bytes) in &files {
        let path = gen_dir.join(rel);
        if fs::read(&path).is_ok_and(|b| &b == bytes) {
            continue;
        }
        if check {
            stale.push(rel.display().to_string());
            continue;
        }
        if let Some(d) = path.parent() {
            fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    if !stale.is_empty() {
        return Err(format!(
            "stale icons (run `cargo xtask icons`): {}",
            stale.join(", ")
        ));
    }
    println!(
        "{} icon files up to date in {}",
        files.len(),
        gen_dir.display()
    );
    Ok(())
}
