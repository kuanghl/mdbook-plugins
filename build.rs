fn main() {
    // 编译 pikchr C 源码
    let pikchr_path = "vendor/pikchr.c";
    if std::path::Path::new(pikchr_path).exists() {
        cc::Build::new().file(pikchr_path).compile("pikchr");
    } else {
        panic!("pikchr.c not found at '{}'", pikchr_path);
    }
}
