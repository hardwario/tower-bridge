//! On Windows, tower-bridge-cli.exe gets a version resource like the app's:
//! the publisher, product and version that Explorer's Details tab, Task
//! Manager and antivirus software read from it.

fn main() -> std::io::Result<()> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    tauri_winres::WindowsResource::new()
        .set("CompanyName", "HARDWARIO a.s.")
        .set("FileDescription", "TOWER Bridge CLI")
        .set("ProductName", "TOWER Bridge")
        .set("LegalCopyright", "© 2026 HARDWARIO a.s.")
        .set("InternalName", "tower-bridge-cli")
        .set("OriginalFilename", "tower-bridge-cli.exe")
        .compile()
}
