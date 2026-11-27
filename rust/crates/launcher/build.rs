fn main() {
    println!("cargo:rerun-if-changed=launcher.rc");
    println!("cargo:rerun-if-changed=assets/launcher.ico");

    let version = env!("CARGO_PKG_VERSION");
    let file_version = format!("FILE_VERSION={},0", version.replace('.', ","));
    let product_version = format!("PRODUCT_VERSION=\"{version}\"");
    embed_resource::compile_for(
        "launcher.rc",
        ["DlssgLauncher"],
        [file_version, product_version],
    )
    .manifest_required()
    .expect("无法嵌入 Windows 图标和版本资源");
}
