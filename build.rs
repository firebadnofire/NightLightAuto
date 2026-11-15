// SPDX-License-Identifier: MPL-2.0
fn main() {
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    println!("cargo:rerun-if-changed=resources/app.ico");
    embed_resource::compile("resources/app.rc", embed_resource::NONE)
        .manifest_required()
        .expect("Windows SDK resource compiler is required");
}
