// SPDX-License-Identifier: MPL-2.0
fn main() {
    use std::{env, fs, path::PathBuf};
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    println!("cargo:rerun-if-changed=resources/app.ico");
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let numbers: Vec<u16> = version
        .split(['.', '-', '+'])
        .take(3)
        .map(|part| {
            part.parse()
                .expect("Version components must fit Windows u16 fields")
        })
        .collect();
    assert_eq!(
        numbers.len(),
        3,
        "Expected a three-component package version"
    );
    let numeric = format!("{},{},{},0", numbers[0], numbers[1], numbers[2]);
    let dotted = numeric.replace(',', ".");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let manifest = fs::read_to_string("resources/app.manifest")
        .expect("Read application manifest")
        .replacen("version=\"0.1.0.0\"", &format!("version=\"{dotted}\""), 1);
    fs::write(out.join("app.manifest"), manifest).expect("Generate versioned manifest");
    let path = |p: PathBuf| p.to_string_lossy().replace('\\', "/");
    let resource = fs::read_to_string("resources/app.rc")
        .expect("Read resource template")
        .replace(
            "\"app.manifest\"",
            &format!("\"{}\"", path(out.join("app.manifest"))),
        )
        .replace(
            "\"app.ico\"",
            &format!("\"{}\"", path(root.join("resources/app.ico"))),
        );
    let version_info = format!(
        r#"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0
FILEOS 0x40004L
FILETYPE 1
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "FileDescription", "Fullscreen Night Light\0"
   VALUE "FileVersion", "{version}\0"
   VALUE "ProductName", "Fullscreen Night Light\0"
   VALUE "ProductVersion", "{version}\0"
   VALUE "OriginalFilename", "FullscreenNightLight.exe\0"
   VALUE "LegalCopyright", "Licensed under MPL-2.0\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x0409, 1200
 END
END
"#
    );
    fs::write(out.join("app.rc"), resource + &version_info).expect("Generate versioned resources");
    embed_resource::compile(out.join("app.rc"), embed_resource::NONE)
        .manifest_required()
        .expect(
            "Windows resource compiler required (Windows SDK or LLVM llvm-rc for cross-builds)",
        );
}
