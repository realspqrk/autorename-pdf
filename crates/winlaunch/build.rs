include!("../../packaging/windows/winres.rs");

fn main() {
    embed_windows_resources("autorename-pdf-gui", "AutoRename-PDF");
}
