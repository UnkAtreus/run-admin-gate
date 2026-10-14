fn main() {
    println!("cargo:rerun-if-changed=res");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("res/launcher.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
