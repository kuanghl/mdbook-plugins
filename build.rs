use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn main() {
    // 编译 pikchr C 源码
    let pikchr_path = "vendor/pikchr.c";
    if std::path::Path::new(pikchr_path).exists() {
        cc::Build::new().file(pikchr_path).compile("pikchr");
    } else {
        panic!("pikchr.c not found at '{}'", pikchr_path);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());

    let src: PathBuf = [&manifest_dir, "target", &profile, "mdbook-plugins"]
        .iter()
        .collect();
    let dst: PathBuf = [&manifest_dir, "test", "bin", "mdbook-plugins"]
        .iter()
        .collect();

    // 确保目标目录存在
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 若二进制已存在（例如二次构建），直接拷贝
    if src.exists() {
        let _ = std::fs::copy(&src, &dst);
        println!("cargo:warning=Binary already present, copied to {}", dst.display());
        return;
    }

    // 首次构建：生成后台轮询脚本，等待二进制生成后自动拷贝
    let script = format!(
        r#"
while ! [ -f "{}" ]; do
    sleep 0.5
done
cp "{}" "{}"
exit 0
"#,
        src.display(),
        src.display(),
        dst.display()
    );

    let script_path = PathBuf::from(&manifest_dir)
        .join("target")
        .join("copy_after_build.sh");
    std::fs::write(&script_path, &script).unwrap();
    let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755));

    let child = Command::new("bash")
        .arg(&script_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match child {
        Ok(_) => println!(
            "cargo:warning=Deploy watcher started. Binary will be copied to {} once compiled.",
            dst.display()
        ),
        Err(e) => println!(
            "cargo:warning=Failed to start deploy watcher: {}. You may need to copy manually.",
            e
        ),
    }
}