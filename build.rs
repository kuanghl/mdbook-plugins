fn main() {
    // 编译 pikchr C 库（内置在 vendor/ 目录中，无外部依赖）
    let pikchr_path = "vendor/pikchr.c";
    if std::path::Path::new(pikchr_path).exists() {
        cc::Build::new()
            .file(pikchr_path)
            .compile("pikchr");
    } else {
        panic!("pikchr.c not found at '{}'", pikchr_path);
    }

    // 部署二进制到 test/bin/
    // build.rs 在 cargo 编译主 crate 之前运行，此时 target/{profile}/mdbook-plugins
    // 还不存在（首次构建）或为旧版本。所以采用后台等待策略：
    // spawn 一个子进程，等待编译完成（目标二进制出现）后拷贝到 test/bin/。
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set");
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    let src_path = std::path::Path::new(&manifest_dir)
        .join("target")
        .join(&profile)
        .join("mdbook-plugins");
    let dst_path = std::path::Path::new(&manifest_dir)
        .join("test")
        .join("bin")
        .join("mdbook-plugins");

    // 删除旧的残余文件（如果有）
    let _ = std::fs::remove_file(&dst_path);

    // 确保 test/bin/ 目录存在
    if let Some(parent) = dst_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 后台等待编译完成后拷贝
    let src = src_path.to_string_lossy().to_string();
    let dst = dst_path.to_string_lossy().to_string();
    let copy_script = format!(
        r#"#!/bin/bash
# 等待编译完成（目标二进制出现），最多等 300 秒
for i in $(seq 1 300); do
    if [ -f "{}" ]; then
        cp "{}" "{}" && exit 0
    fi
    sleep 1
done
exit 1
"#,
        src, src, dst
    );

    match std::process::Command::new("bash")
        .args(&["-c", &copy_script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => {
            println!(
                "cargo:warning=部署后台进程已启动（等待编译完成，将拷贝到 test/bin/mdbook-plugins）"
            );
        }
        Err(e) => {
            // 无法 spawn 时，直接尝试拷贝当前二进制（可能是旧版本）
            if src_path.exists() {
                if let Err(cp_err) = std::fs::copy(&src_path, &dst_path) {
                    println!(
                        "cargo:warning=部署到 test/bin/ 失败: 后台进程({})和直接拷贝({})均失败",
                        e, cp_err
                    );
                } else {
                    println!(
                        "cargo:warning=已部署 test/bin/mdbook-plugins（直接拷贝，可能为旧版本，建议重新构建后确认）"
                    );
                }
            } else {
                println!(
                    "cargo:warning=部署跳过: 二进制尚未生成（首次构建后自动生效）"
                );
            }
        }
    }
}
