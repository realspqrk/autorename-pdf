//! Build features no runtime test can observe.

/// `eframe` with `default-features = false` drops `clipboard`: egui then keeps
/// copies in-process. "Copy path" shows its toast, other apps paste nothing,
/// and Ctrl+V into a field (API key) pastes nothing either.
#[test]
fn gui_uses_the_system_clipboard() {
    let manifest = include_str!("../Cargo.toml");
    let eframe = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("eframe "))
        .expect("eframe dependency line");
    assert!(
        eframe.contains("\"clipboard\""),
        "eframe needs the `clipboard` feature: {eframe}"
    );
}
