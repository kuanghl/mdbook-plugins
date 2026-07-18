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

    // 构建完成后自动部署二进制到 test/bin/
    // （每次构建都会执行，确保 test/bin/ 中的二进制为最新版）
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set");
    let src_path = std::path::Path::new(&manifest_dir)
        .join("target")
        .join(&profile)
        .join("mdbook-plugins");
    let dst_path = std::path::Path::new(&manifest_dir)
        .join("test")
        .join("bin")
        .join("mdbook-plugins");

    if src_path.exists() {
        if let Err(e) = std::fs::copy(&src_path, &dst_path) {
            println!(
                "cargo:warning=部署二进制到 test/bin/ 失败: {} ({} -> {})",
                e,
                src_path.display(),
                dst_path.display()
            );
        } else {
            println!("cargo:warning=已部署二进制到 test/bin/");
        }
    } else {
        println!(
            "cargo:warning=首次构建，二进制尚未生成，跳过部署到 test/bin/（后续构建将自动部署）"
        );
    }
}
