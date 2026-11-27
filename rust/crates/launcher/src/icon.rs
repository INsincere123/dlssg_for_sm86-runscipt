pub fn app_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/launcher.png"))
        .expect("启动器图标必须是有效的 PNG")
}
