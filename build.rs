fn main() {
    println!("cargo:rerun-if-changed=assets/opendesk-icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_DESKTOP").is_some()
    {
        winresource::WindowsResource::new()
            .set_icon("assets/opendesk-icon.ico")
            .set("ProductName", "Opendesk")
            .set("FileDescription", "Opendesk Remote Desktop")
            .set("LegalCopyright", "Opendesk contributors")
            .compile()
            .expect("Compile the Opendesk application icon and version resource");
    }
}
