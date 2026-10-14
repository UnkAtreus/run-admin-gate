fn main() {
    println!("cargo:rerun-if-changed=res");
    // Dialogs, manifest and version info only exist for Windows targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("res/manager.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
