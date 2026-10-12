fn main() {
    println!("cargo:rerun-if-changed=packaging/volna.rc");
    println!("cargo:rerun-if-changed=assets/app-icon/volna.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile_for("packaging/volna.rc", ["volna"], embed_resource::NONE)
            .manifest_required()
            .expect("compile the Volna application icon");
    }
}
