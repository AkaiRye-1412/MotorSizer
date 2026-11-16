//! 把图标与版本信息写入 Windows 可执行文件资源。
//!
//! 仅对 Windows 目标生效；构建时需要 PATH 中有 `windres`（由 build.cmd 提供）。

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_owned());
    let packed = pack_version(&version);

    let mut resource = winres::WindowsResource::new();
    resource
        .set_icon("assets/app.ico")
        .set("ProductName", "MotionSizer Studio")
        .set(
            "FileDescription",
            "MotionSizer Studio — 电机选型与运动曲线计算",
        )
        .set("OriginalFilename", "motor-selection.exe")
        .set("CompanyName", "Akribis")
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set_version_info(winres::VersionInfo::FILEVERSION, packed)
        .set_version_info(winres::VersionInfo::PRODUCTVERSION, packed);

    resource
        .compile()
        .unwrap_or_else(|error| panic!("无法编译 Windows 资源：{error}"));

    // winres 只把资源打进 libresource.a；GNU 链接器会丢弃没有符号引用的静态库成员，
    // 资源段因此进不了 exe。直接把编译好的 .o 作为链接参数传给目标文件。
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        let object = std::path::Path::new(&out_dir).join("resource.o");
        if object.exists() {
            println!("cargo:rustc-link-arg={}", object.display());
        }
    }
}

/// 把 "2.0.3" 打包成资源版本号要求的 4 段 16 位整数（2.0.3.0）。
fn pack_version(version: &str) -> u64 {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    let a = parts.next().unwrap_or(0) & 0xffff;
    let b = parts.next().unwrap_or(0) & 0xffff;
    let c = parts.next().unwrap_or(0) & 0xffff;
    let d = parts.next().unwrap_or(0) & 0xffff;
    (a << 48) | (b << 32) | (c << 16) | d
}
