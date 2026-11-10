use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn file_version(version: &str) -> String {
    let base = version.split('-').next().unwrap_or(version);
    let mut parts = base.split('.').map(|part| part.parse::<u16>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    let build = version
        .rsplit('.')
        .next()
        .and_then(|part| part.parse::<u16>().ok())
        .unwrap_or(0);
    format!("{major},{minor},{patch},{build}")
}

fn find_rc() -> Option<PathBuf> {
    if let Ok(path) = env::var("RC") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    if let Ok(output) = Command::new("where").arg("rc.exe").output() {
        if output.status.success() {
            if let Some(line) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
            {
                let path = PathBuf::from(line);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    let kits = PathBuf::from(env::var_os("ProgramFiles(x86)")?)
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions: Vec<PathBuf> = fs::read_dir(kits)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    versions.reverse();
    versions
        .into_iter()
        .map(|version| version.join("x64").join("rc.exe"))
        .find(|path| path.is_file())
}

fn embed_version_resource() {
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe was not found; skipping the PE version resource");
        return;
    };
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let numeric = file_version(&version);
    let product = "Aurelia";
    let description = "Aurelia Illustration and Animation Studio";
    let company = "LeTrollologist";

    let source = format!(
        r#"1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "{company}"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "aurelia"
      VALUE "OriginalFilename", "aurelia.exe"
      VALUE "ProductName", "{product}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    );

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let rc_path = out_dir.join("aurelia-version.rc");
    let res_path = out_dir.join("aurelia-version.res");
    fs::write(&rc_path, source).expect("write version resource");
    let status = Command::new(rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .status()
        .expect("run rc.exe");
    if !status.success() {
        panic!("rc.exe failed to compile the PE version resource");
    }
    println!("cargo:rustc-link-arg={}", res_path.display());
}

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rustc-link-lib=shell32");
        println!("cargo:rustc-link-lib=ole32");
    }

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        embed_version_resource();
    }
    println!("cargo:rerun-if-changed=build.rs");
}
