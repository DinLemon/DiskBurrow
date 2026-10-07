use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../../../src/DiskBurrow.App/Resources/DiskBurrow.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let package = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let resource = out.join("DiskBurrow.rc");
    let compiled = out.join("DiskBurrow.res");
    let quoted = |p: PathBuf| {
        p.canonicalize()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('\\', "\\\\")
    };
    let icon = quoted(package.join("../../../src/DiskBurrow.App/Resources/DiskBurrow.ico"));
    // GPUI already links resource 1/RT_MANIFEST with asInvoker and PerMonitorV2.
    // A second manifest resource would collide with gpui.lib at link time.
    fs::write(
        &resource,
        format!(
            r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION 0,3,0,1
PRODUCTVERSION 0,3,0,1
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x2L
FILEOS 0x40004L
FILETYPE 0x1L
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904b0"
  BEGIN
   VALUE "CompanyName", "DinLemon\0"
   VALUE "FileDescription", "DiskBurrow Rust preview\0"
   VALUE "FileVersion", "0.3.0-alpha.1\0"
   VALUE "InternalName", "DiskBurrow\0"
   VALUE "OriginalFilename", "DiskBurrow.exe\0"
   VALUE "ProductName", "DiskBurrow\0"
   VALUE "ProductVersion", "0.3.0-alpha.1\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"#
        ),
    )
    .unwrap();
    let status = Command::new("rc.exe")
        .arg("/nologo")
        .arg("/fo")
        .arg(&compiled)
        .arg(&resource)
        .status()
        .expect("Windows SDK rc.exe is required; initialize the MSVC build environment");
    assert!(status.success(), "Compile DiskBurrow resources");
    println!("cargo:rustc-link-arg-bin=DiskBurrow={}", compiled.display());
}
