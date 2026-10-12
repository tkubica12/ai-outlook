mod brand {
    include!("src/brand.rs");
}
fn main() {
    println!("cargo:rerun-if-changed=src/brand.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let icon = out.join("tomlook.ico");
    std::fs::write(&icon, brand::ico()).expect("Write generated Tomlook icon");
    let rc = out.join("tomlook.rc");
    let version = std::env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let numeric = ["MAJOR", "MINOR", "PATCH"].map(|part| {
        std::env::var(format!("CARGO_PKG_VERSION_{part}"))
            .expect("Cargo numeric version")
            .parse::<u16>()
            .expect("Windows version component must fit u16")
    });
    std::fs::write(
        &rc,
        format!(
            r#"1 ICON "{icon}"
1 VERSIONINFO
 FILEVERSION {major},{minor},{patch},0
 PRODUCTVERSION {major},{minor},{patch},0
 FILEFLAGSMASK 0x3fL
 FILEFLAGS 0
 FILEOS 0x40004L
 FILETYPE 0x1L
 FILESUBTYPE 0
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "FileDescription", "Tomlook native Windows calendar\0"
   VALUE "FileVersion", "{version}\0"
   VALUE "ProductName", "Tomlook\0"
   VALUE "ProductVersion", "{version}\0"
   VALUE "OriginalFilename", "Tomlook.exe\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x0409, 1200
 END
END
"#,
            icon = icon.display().to_string().replace('\\', "\\\\"),
            major = numeric[0],
            minor = numeric[1],
            patch = numeric[2],
        ),
    )
    .expect("Write icon resource");
    let res = out.join("tomlook.res");
    let status = std::process::Command::new("rc.exe")
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&rc)
        .status()
        .expect("Windows SDK resource compiler is required");
    assert!(status.success(), "Compile Tomlook icon resource");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}
