//! 主窗口（M4-01）：只负责画，所有判断在 [`crate::view_model`]。
//!
//! `--ui-selftest <ms>`：开窗后自动关闭，并把 `ui=ok`、`pixels_per_point`、首帧耗时写进报告，
//! 供验收脚本测量内存与冷启动。

use eframe::egui;
use std::path::PathBuf;
use std::time::Instant;

use crate::view_model::{GameStatus, LauncherView};

/// 自检参数。
#[derive(Debug, Clone, Default)]
pub struct SelfTest {
    pub hold_ms: u64,
    pub report: Option<PathBuf>,
}

pub struct LauncherApp {
    view: LauncherView,
    self_test: Option<SelfTest>,
    started: Instant,
    close_at: Option<Instant>,
    reported: bool,
    first_frame_ms: Option<u128>,
    /// 设置页：`None` 表示窗口关闭。
    settings: Option<crate::settings_form::SettingsForm>,
    /// 偏好文件路径，保存时用。
    preferences_path: Option<std::path::PathBuf>,
    /// 底部提示（保存结果等）。
    notice: String,
    /// 托盘图标（常驻，窗口隐藏后仍可操作）。
    tray: Option<crate::tray::Tray>,
    /// 待确认移除的游戏 id。
    remove_pending: Option<String>,
    /// 交接后隐藏窗口（不退出进程）。
    hide_after_handoff: bool,
    /// 恢复进行中：窗口保持可见并轮询结果。
    restore_pending: bool,
    /// 交给引擎恢复的时刻（用于超时提示）。
    restore_started: Option<std::time::Instant>,
    /// 已把工作交给引擎：本帧结束后关闭界面。
    quit_after_frame: bool,
    /// 允许真正关闭（托盘"退出"或自检结束）；否则关闭按钮只隐藏到托盘。
    allow_close: bool,
    /// 自定义 INI 的导入路径与预览文本（M4-04）。
    ini_path: String,
    ini_preview: Option<String>,
    ini_message: String,
    ini_verbatim: bool,
    /// 添加游戏对话框（M4-05）。
    add_game: Option<crate::add_game::AddGameState>,
    /// 关于对话框（M4-06）。
    show_about: bool,
    /// 启动器根目录（日志与记录都在它下面）。
    root: std::path::PathBuf,
}

impl LauncherApp {
    pub fn with_preferences(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.preferences_path = path;
        self
    }

    /// 直接打开"添加游戏"对话框（`--open-add-game` 与测试用）。
    #[allow(dead_code)]
    pub fn open_add_game(&mut self) {
        self.add_game = Some(crate::add_game::AddGameState::new());
    }

    pub fn with_root(mut self, root: std::path::PathBuf) -> Self {
        self.root = root;
        self
    }

    pub fn new(view: LauncherView, self_test: Option<SelfTest>) -> Self {
        let started = Instant::now();
        let close_at = self_test
            .as_ref()
            .map(|test| started + std::time::Duration::from_millis(test.hold_ms.max(50)));
        Self {
            view,
            self_test,
            started,
            close_at,
            reported: false,
            first_frame_ms: None,
            settings: None,
            preferences_path: None,
            notice: String::new(),
            quit_after_frame: false,
            allow_close: false,
            tray: None,
            remove_pending: None,
            hide_after_handoff: false,
            restore_pending: false,
            restore_started: None,
            ini_path: String::new(),
            ini_preview: None,
            ini_message: String::new(),
            ini_verbatim: false,
            add_game: None,
            show_about: false,
            root: std::path::PathBuf::new(),
        }
    }

    /// 打开设置页：取该游戏已存的设置，没有就用默认值。
    fn open_settings(&mut self) {
        let Some(entry) = self.view.selected_game() else {
            return;
        };
        let stored = self
            .preferences_path
            .as_deref()
            .and_then(|path| launcher_core::preferences::load(path).ok())
            .and_then(|preferences| {
                preferences
                    .find(&entry.id)
                    .and_then(|game| game.settings.clone())
            });
        self.settings = Some(crate::settings_form::SettingsForm::new(
            stored.unwrap_or_default(),
        ));
    }

    /// 保存设置到偏好文件。
    fn save_settings(&mut self) {
        let Some(form) = self.settings.as_mut() else {
            return;
        };
        let Some(settings) = form.take_for_save() else {
            self.notice = form
                .error
                .clone()
                .unwrap_or_else(|| "校验未通过".to_owned());
            return;
        };
        let Some(path) = self.preferences_path.clone() else {
            self.notice = "偏好路径不可用，无法保存".to_owned();
            return;
        };
        let Some(entry) = self.view.selected_game().cloned() else {
            return;
        };
        let mut preferences = launcher_core::preferences::load(&path).unwrap_or_default();
        match preferences.find(&entry.id) {
            Some(existing) => {
                let mut updated = existing.clone();
                updated.settings = Some(settings);
                preferences.upsert(updated);
            }
            None => {
                let mut profile = launcher_core::preferences::GameProfile::new(
                    &entry.id,
                    &entry.name,
                    entry.directory.clone(),
                );
                profile.engine_host = entry.engine_host.clone();
                profile.settings = Some(settings);
                preferences.upsert(profile);
            }
        }
        self.notice = match launcher_core::preferences::save(&path, &preferences) {
            Ok(()) => "设置已保存".to_owned(),
            Err(error) => format!("保存失败：{error}"),
        };
    }

    fn write_report(&mut self, pixels_per_point: f32) {
        let Some(test) = self.self_test.as_ref() else {
            return;
        };
        let Some(path) = test.report.as_ref() else {
            return;
        };
        if self.reported {
            return;
        }
        self.reported = true;
        let text = format!(
            concat!(
                "{{\n",
                "  \"ui\": \"ok\",\n",
                "  \"pixels_per_point\": {ppp},\n",
                "  \"games\": {games},",
                "  \"tray\": {tray}",
                "}}\n"
            ),
            ppp = pixels_per_point,
            games = self.view.games.len(),
            tray = self.tray.is_some()
        );
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("headline").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("DLSSG 启动器");
                ui.add_space(12.0);
                ui.label(self.view.headline());
            });
        });

        egui::Panel::bottom("actions").show(ui, |ui| {
            ui.horizontal(|ui| {
                let launch = ui.add_enabled(self.view.can_launch(), egui::Button::new("启动"));
                let restore_text = if self.last_restore_outcome() == Some(false) {
                    "重试恢复"
                } else {
                    "恢复原状"
                };
                let restore =
                    ui.add_enabled(self.view.can_restore(), egui::Button::new(restore_text));

                if launch.clicked() {
                    self.hand_off(crate::handoff::Command::Run, "ui-run.json");
                }
                if restore.clicked() {
                    self.hand_off(crate::handoff::Command::Restore, "ui-restore.json");
                }
                if let Some(entry) = self.view.selected_game() {
                    let mut text = format!("状态：{}", entry.status.text());
                    if self.restore_pending {
                        text.push_str(" · 恢复中…");
                    }
                    match self.last_restore_outcome() {
                        Some(true) => text.push_str(" · 上次恢复成功"),
                        Some(false) => text.push_str(" · 上次恢复失败（可重试）"),
                        None => {}
                    }
                    ui.label(egui::RichText::new(text).small().weak());
                }
                ui.separator();
                let can_remove = self.view.selected_game().is_some();
                if ui
                    .add_enabled(can_remove, egui::Button::new("移除游戏"))
                    .clicked()
                {
                    self.remove_pending = self.view.selected_game().map(|entry| entry.id.clone());
                }
                if ui.button("添加游戏").clicked() {
                    self.add_game = Some(crate::add_game::AddGameState::new());
                }
                let mut open_settings = false;
                if ui.button("日志").clicked() {
                    let logs = crate::about::logs_directory(&self.root);
                    let _ = std::fs::create_dir_all(&logs);
                    self.notice = match launcher_win::open_path(&logs) {
                        Ok(()) => format!("已打开日志目录：{}", logs.display()),
                        Err(error) => format!("打开日志目录失败：{error}"),
                    };
                }
                if ui.button("关于").clicked() {
                    self.show_about = true;
                }
                if ui
                    .add_enabled(
                        self.view.selected_game().is_some(),
                        egui::Button::new("设置"),
                    )
                    .clicked()
                {
                    open_settings = true;
                }
                if open_settings {
                    self.open_settings();
                }
                if !self.view.notice.is_empty() {
                    ui.separator();
                    ui.label(&self.view.notice);
                }
                // 按钮回调留给 M4-05/M4-06 接引擎；这里先保证布局与可用性正确。
                let _ = (launch, restore);
            });
        });

        self.draw_about(ui);
        self.draw_add_game(ui);
        self.draw_settings(ui);

        egui::CentralPanel::default().show(ui, |ui| {
            if self.view.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label("还没有游戏。点击下方「添加游戏」开始。");
                });
                return;
            }
            let mut select = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, entry) in self.view.games.iter().enumerate() {
                    let selected = self.view.selected == Some(index);
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(selected, &entry.name)
                            .on_hover_text(entry.summary())
                            .clicked()
                        {
                            select = Some(index);
                        }
                        status_pill(ui, entry.status);
                        if !entry.engine_host.is_empty() && entry.engine_host != "none" {
                            ui.label(egui::RichText::new(&entry.engine_host).small().weak());
                        }
                    });
                    ui.label(entry.directory.display().to_string());
                    ui.separator();
                }
            });
            if let Some(index) = select {
                self.view.select(index);
            }
        });
    }
}

impl LauncherApp {
    /// 关于对话框（M4-06）。
    fn draw_about(&mut self, root: &mut egui::Ui) {
        if !self.show_about {
            return;
        }
        let info = crate::about::AboutInfo::collect(&self.root, self.preferences_path.clone());
        let mut open = true;
        egui::Window::new("关于")
            .open(&mut open)
            .default_width(520.0)
            .resizable(false)
            .show(root.ctx(), |ui| {
                egui::Grid::new("about-grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (key, value) in info.lines() {
                            ui.label(key);
                            ui.label(egui::RichText::new(value).monospace());
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.label(
                    egui::RichText::new("提示只在窗口内显示，不弹系统通知，因此不会抢焦点。")
                        .small()
                        .weak(),
                );
            });
        if !open {
            self.show_about = false;
        }
    }

    /// 把一份游戏配置写进偏好并刷新列表。
    fn persist_profile(&mut self, profile: launcher_core::preferences::GameProfile) {
        let Some(path) = self.preferences_path.clone() else {
            self.notice = "偏好路径不可用，无法保存".to_owned();
            return;
        };
        let mut preferences = launcher_core::preferences::load(&path).unwrap_or_default();
        preferences.upsert(profile.clone());
        self.notice = match launcher_core::preferences::save(&path, &preferences) {
            Ok(()) => format!("已添加：{}", profile.name),
            Err(error) => format!("保存失败：{error}"),
        };
        let status = crate::view_model::derive_status(
            profile.directory.is_dir(),
            false,
            launcher_win::session_active(),
        );
        self.view.upsert(crate::view_model::GameEntry {
            id: profile.id.clone(),
            name: profile.name.clone(),
            directory: profile.directory.clone(),
            status,
            engine_host: profile.engine_host.clone(),
        });
        if let Some(index) = self
            .view
            .games
            .iter()
            .position(|item| item.id == profile.id)
        {
            self.view.select(index);
        }
    }

    /// 移除游戏前的确认框：只从偏好里删记录，不动磁盘。
    fn draw_remove_confirm(&mut self, root: &mut egui::Ui) {
        let Some(id) = self.remove_pending.clone() else {
            return;
        };
        let name = self
            .view
            .games
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| id.clone());
        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("移除游戏")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(root.ctx(), |ui| {
                ui.label(format!("从列表里移除「{name}」？"));
                ui.label("只删除这条偏好记录，不会动游戏目录或已部署的文件。");
                ui.horizontal(|ui| {
                    if ui.button("移除").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirmed {
            if let Some(path) = self.preferences_path.as_ref() {
                if let Ok(mut preferences) = launcher_core::preferences::load(path) {
                    preferences.games.retain(|game| game.id != id);
                    let _ = launcher_core::preferences::save(path, &preferences);
                }
            }
            self.view.games.retain(|entry| entry.id != id);
            self.notice = format!("已移除 {name}");
            self.remove_pending = None;
        } else if !open || cancelled {
            self.remove_pending = None;
        }
    }
    /// 读最近一次"恢复原状"的报告：`Some(true)` 成功、`Some(false)` 失败、`None` 未跑过。
    fn last_restore_outcome(&self) -> Option<bool> {
        let path = crate::about::logs_directory(&self.root).join("ui-restore.json");
        let text = std::fs::read_to_string(path).ok()?;
        if text.contains("\"code\": \"restored\"") {
            Some(true)
        } else if text.contains("\"error\"") {
            Some(false)
        } else {
            None
        }
    }
    /// 把「启动 / 恢复原状」交给提权后的引擎子进程，成功后本界面退出。
    fn hand_off(&mut self, command: crate::handoff::Command, report_name: &str) {
        let Some(game) = self.view.selected_game().map(|entry| entry.id.clone()) else {
            self.notice = "先选择一个游戏".to_owned();
            return;
        };
        let logs = crate::about::logs_directory(&self.root);
        let _ = std::fs::create_dir_all(&logs);
        let report = logs.join(report_name);
        match crate::handoff::hand_off(command, &game, &self.root, &report) {
            Ok(()) => {
                match command {
                    // 启动：交接成功后隐藏窗口，交给托盘常驻。
                    crate::handoff::Command::Run => self.hide_after_handoff = true,
                    // 恢复：窗口保持可见，显示"恢复中"并轮询结果。
                    crate::handoff::Command::Restore => {
                        self.restore_pending = true;
                        self.restore_started = Some(std::time::Instant::now());
                    }
                }
                self.notice = format!(
                    "已交给引擎（{}），界面将关闭；结果见 {}",
                    command.as_str(),
                    report.display()
                );
            }
            Err(error) => self.notice = format!("交接失败：{error}"),
        }
    }
    /// 添加游戏对话框（M4-05）。
    fn draw_add_game(&mut self, root: &mut egui::Ui) {
        use crate::add_game::{AddGameState, HostChoice};

        let Some(mut state) = self.add_game.take() else {
            return;
        };
        let mut open = true;
        let mut close_requested = false;
        let mut confirmed = None;
        egui::Window::new("添加游戏")
            .open(&mut open)
            .default_width(560.0)
            .resizable(true)
            .show(root.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("游戏目录");
                    // 显式 id + 固定宽度：避免与其它输入框抢焦点，也方便键盘输入。
                    ui.add(
                        egui::TextEdit::singleline(&mut state.directory)
                            .id_salt("add-game-directory")
                            .desired_width(360.0)
                            .hint_text(r"D:\Games\BlackMythWukong"),
                    );
                    if ui.button("浏览…").clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            state.directory = folder.to_string_lossy().into_owned();
                            state.message = "已选择目录，点击「扫描」查找渲染程序".to_owned();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("扫描").clicked() {
                        let directory = std::path::PathBuf::from(state.directory.trim());
                        match launcher_core::discovery::renderer_candidates(
                            &directory,
                            None,
                            launcher_core::discovery::DiscoveryLimits::default(),
                            &|| false,
                        ) {
                            Ok(outcome) => state.apply_scan(outcome),
                            Err(error) => state.message = format!("扫描失败：{error}"),
                        }
                    }
                    ui.label(if state.directory_ok() {
                        "目录可用"
                    } else {
                        "目录不可用"
                    });
                });
                if !state.message.is_empty() {
                    ui.label(&state.message);
                }
                if !state.candidates.is_empty() {
                    ui.separator();
                    let rows: Vec<(usize, String, bool)> = state
                        .candidates
                        .iter()
                        .enumerate()
                        .map(|(index, candidate)| {
                            (
                                index,
                                format!(
                                    "{}  ·  {} 分  ·  {}",
                                    candidate.relative,
                                    candidate.score,
                                    candidate.reasons.join("；")
                                ),
                                state.selected == Some(index),
                            )
                        })
                        .collect();
                    let mut pick = None;
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for (index, text, selected) in rows {
                                if ui.selectable_label(selected, text).clicked() {
                                    pick = Some(index);
                                }
                            }
                        });
                    if let Some(index) = pick {
                        state.select(index);
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("引擎宿主");
                    for host in HostChoice::all() {
                        ui.selectable_value(&mut state.host, host, host.as_text());
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(state.confirm().is_some(), egui::Button::new("确定"))
                        .clicked()
                    {
                        confirmed = state.confirm();
                    }
                    if ui.button("取消").clicked() {
                        close_requested = true;
                    }
                });
            });

        if let Some(profile) = confirmed {
            self.persist_profile(profile);
            self.add_game = None;
        } else if open && !close_requested {
            self.add_game = Some(state);
        }
        let _ = AddGameState::new;
    }

    /// 设置窗口（M4-03）：常用 5 项 + 高级 7 项，联动灰化与校验都在 `settings_form`。
    fn draw_settings(&mut self, root: &mut egui::Ui) {
        use crate::settings_form::{ADVANCED_KEYS, COMMON_KEYS};
        use launcher_core::settings::{
            EngineKind, FlowResolution, RuntimeMode, SuperResolutionMode, UseHudless,
        };

        let Some(mut form) = self.settings.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let states = form.field_states();
        egui::Window::new("设置")
            .open(&mut open)
            .default_width(520.0)
            .resizable(true)
            .show(root.ctx(), |ui| {
                ui.label(format!("常用（{} 项）", COMMON_KEYS.len()));
                ui.add_enabled_ui(true, |ui| {
                    ui.checkbox(&mut form.settings.enabled, "Enabled · 启用组件");
                });
                ui.add_enabled_ui(form.settings.enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Engine · 超分引擎");
                        ui.selectable_value(&mut form.settings.engine, EngineKind::Dlss, "DLSS");
                        let xe = ui.selectable_value(
                            &mut form.settings.engine,
                            EngineKind::XeSs,
                            "XeSS",
                        );
                        if xe.clicked() {
                            form.set_engine(EngineKind::XeSs);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("MaxGeneratedFrames · 生成帧数");
                        // 一律列出 1–5；引擎是 XeSS 时由内部夹紧逻辑收敛到 3。
                        for value in 1..=5_u8 {
                            if ui
                                .selectable_value(
                                    &mut form.settings.max_generated_frames,
                                    value,
                                    value.to_string(),
                                )
                                .clicked()
                            {
                                form.dirty = true;
                            }
                        }
                        ui.label(format!("上限 {}", states.max_generated_frames));
                    });
                    ui.horizontal(|ui| {
                        ui.label("SuperResolution.Mode");
                        for (mode, text) in [
                            (SuperResolutionMode::Off, "Off"),
                            (SuperResolutionMode::MInt8, "M-Int8"),
                            (SuperResolutionMode::LInt8, "L-Int8"),
                            (SuperResolutionMode::MFp16, "M-Fp16"),
                            (SuperResolutionMode::LFp16, "L-Fp16"),
                        ] {
                            if ui
                                .selectable_value(&mut form.settings.super_resolution, mode, text)
                                .clicked()
                            {
                                form.set_super_resolution(mode);
                            }
                        }
                    });
                    ui.add_enabled_ui(states.advanced_enabled, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Dlssg.Preset");
                            for (preset, text) in [
                                (launcher_core::settings::Preset::A, "A"),
                                (launcher_core::settings::Preset::B, "B"),
                                (launcher_core::settings::Preset::C, "C"),
                            ] {
                                if ui
                                    .selectable_value(&mut form.settings.preset, preset, text)
                                    .clicked()
                                {
                                    form.dirty = true;
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Level");
                            for (value, text) in [
                                (0_u8, "0 关闭"),
                                (1, "1 错误"),
                                (2, "2 配置与能力"),
                                (3, "3 详细追踪"),
                            ] {
                                if ui
                                    .selectable_value(&mut form.settings.level, value, text)
                                    .clicked()
                                {
                                    form.dirty = true;
                                }
                            }
                        });
                    });
                });

                ui.separator();
                egui::CollapsingHeader::new(format!("高级（{} 项）", ADVANCED_KEYS.len()))
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_enabled_ui(states.advanced_enabled, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Optimized");
                                for (value, text) in [
                                    (0_u8, "0 原厂"),
                                    (1, "1 逐位一致"),
                                    (2, "2 快（有损）"),
                                    (3, "3 最快（有损）"),
                                ] {
                                    if ui
                                        .selectable_value(&mut form.settings.optimized, value, text)
                                        .clicked()
                                    {
                                        form.dirty = true;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Logging.Directory");
                                if ui
                                    .text_edit_singleline(&mut form.settings.directory)
                                    .changed()
                                {
                                    form.dirty = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("FlowResolution");
                                for (mode, text) in [
                                    (FlowResolution::Coarse, "Coarse"),
                                    (FlowResolution::Middle, "Middle"),
                                    (FlowResolution::Fine, "Fine"),
                                    (FlowResolution::CoarseAt4K, "Coarse@4K"),
                                ] {
                                    if ui
                                        .selectable_value(
                                            &mut form.settings.flow_resolution,
                                            mode,
                                            text,
                                        )
                                        .clicked()
                                    {
                                        form.dirty = true;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("UseHudless");
                                for (mode, text) in [
                                    (UseHudless::Auto, "Auto"),
                                    (UseHudless::Zero, "0"),
                                    (UseHudless::One, "1"),
                                ] {
                                    if ui
                                        .selectable_value(
                                            &mut form.settings.use_hudless,
                                            mode,
                                            text,
                                        )
                                        .clicked()
                                    {
                                        form.dirty = true;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Runtime.Mode");
                                for (mode, text) in [
                                    (RuntimeMode::Bundled, "Bundled"),
                                    (RuntimeMode::External, "External"),
                                    (RuntimeMode::Pinned, "Pinned"),
                                ] {
                                    if ui
                                        .selectable_value(
                                            &mut form.settings.runtime_mode,
                                            mode,
                                            text,
                                        )
                                        .clicked()
                                    {
                                        form.set_runtime_mode(mode);
                                    }
                                }
                            });
                            ui.add_enabled_ui(states.runtime_path, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Runtime.Path");
                                    let mut text =
                                        form.settings.runtime_path.clone().unwrap_or_default();
                                    if ui.text_edit_singleline(&mut text).changed() {
                                        form.settings.runtime_path =
                                            (!text.trim().is_empty()).then_some(text);
                                        form.dirty = true;
                                    }
                                });
                            });
                        });
                    });

                ui.separator();
                egui::CollapsingHeader::new("自定义 INI")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("文件");
                            ui.text_edit_singleline(&mut self.ini_path);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("导入").clicked() {
                                match std::fs::read_to_string(self.ini_path.trim()) {
                                    Ok(text) => {
                                        if self.ini_verbatim {
                                            form.import_verbatim(&text);
                                            self.ini_message = "已按自定义全文导入".to_owned();
                                        } else {
                                            match form.import_text(&text) {
                                                Ok(outcome) => {
                                                    self.ini_message = format!(
                                                        "已应用 {} 项，未知 {} 项，提示 {} 条",
                                                        outcome.applied.len(),
                                                        outcome.unknown.len(),
                                                        outcome.hints.len()
                                                    );
                                                }
                                                Err(message) => self.ini_message = message,
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        self.ini_message = format!("读取失败：{error}");
                                    }
                                }
                            }
                            ui.checkbox(&mut self.ini_verbatim, "整份作为自定义 INI");
                            if form.is_verbatim() {
                                ui.label(egui::RichText::new("当前使用自定义全文").small().weak());
                            }
                            if ui.button("预览").clicked() {
                                self.ini_preview = Some(match form.preview() {
                                    Ok(text) => text,
                                    Err(message) => message,
                                });
                            }
                            if ui.button("恢复默认").clicked() {
                                form.restore_defaults();
                                self.ini_verbatim = false;
                                self.ini_message = "已恢复默认".to_owned();
                                self.ini_preview = None;
                            }
                        });
                        if !self.ini_message.is_empty() {
                            ui.label(&self.ini_message);
                        }
                        if let Some(preview) = self.ini_preview.clone() {
                            egui::ScrollArea::vertical()
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut preview.as_str())
                                            .desired_width(f32::INFINITY)
                                            .interactive(false),
                                    );
                                });
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("保存").clicked() {
                        save = true;
                    }
                    if ui.button("校验").clicked() {
                        let _ = form.validate();
                    }
                    if let Some(error) = form.error.as_deref() {
                        ui.colored_label(egui::Color32::from_rgb(0xD0, 0x4A, 0x4A), error);
                    }
                });
            });

        if save {
            self.settings = Some(form);
            self.save_settings();
            self.settings = None;
        } else if open {
            self.settings = Some(form);
        }
    }
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.tray.is_none() {
            self.tray = crate::tray::Tray::new("DLSSG 启动器");
        }
        if let Some(action) = self.tray.as_ref().and_then(|tray| tray.poll()) {
            match action {
                crate::tray::Action::Show => {
                    self.hide_after_handoff = false;
                    root.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    root.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                crate::tray::Action::Logs => {
                    let logs = crate::about::logs_directory(&self.root);
                    let _ = std::fs::create_dir_all(&logs);
                    let _ = launcher_win::open_path(&logs);
                }
                crate::tray::Action::Quit => {
                    self.allow_close = true;
                    root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        // 关闭按钮 = 最小化到托盘：取消关闭并隐藏窗口，只有托盘"退出"才结束进程。
        if !self.allow_close && root.ctx().input(|input| input.viewport().close_requested()) {
            root.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            root.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hide_after_handoff = true;
        }
        // 恢复进行中：保持窗口可见，显示进度并在结果出来后更新提示。
        if self.restore_pending {
            match self.last_restore_outcome() {
                Some(true) => {
                    self.restore_pending = false;
                    self.notice = "恢复完成".to_owned();
                }
                Some(false) => {
                    self.restore_pending = false;
                    self.notice = "恢复失败，可点「重试恢复」".to_owned();
                }
                None => {
                    let elapsed = self
                        .restore_started
                        .map(|start| start.elapsed().as_secs())
                        .unwrap_or(0);
                    if elapsed >= 60 {
                        self.restore_pending = false;
                        self.notice = "恢复仍未完成，请查看日志".to_owned();
                    }
                }
            }
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(300));
        }
        if let Some(entry) = self.view.selected_game() {
            if let Some(tray) = self.tray.as_ref() {
                tray.set_tooltip(&format!(
                    "DLSSG 启动器 · {} · {}",
                    entry.name,
                    entry.status.text()
                ));
            }
        }
        if self.hide_after_handoff {
            // 交接后隐藏窗口，进程留在托盘里；状态用悬停提示表达。
            root.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if let Some(tray) = self.tray.as_ref() {
                tray.set_tooltip("DLSSG 启动器：引擎已接管（双击托盘图标恢复窗口）");
            }
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.draw_remove_confirm(root);
        if self.quit_after_frame {
            root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.first_frame_ms.is_none() {
            self.first_frame_ms = Some(self.started.elapsed().as_millis());
        }
        self.draw(root);
        if self.self_test.is_some() {
            // 自检必须保证还会被再次调度：窗口被遮挡或失焦时 egui 可能停止重绘，
            // 那样 close_at 永远不会被执行（2026-11-22 实测挂过一次）。
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            let pixels_per_point = root.ctx().pixels_per_point();
            self.write_report(pixels_per_point);
            if self
                .close_at
                .map(|deadline| Instant::now() >= deadline)
                .unwrap_or(false)
            {
                self.allow_close = true;
                root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

/// 载入系统字体（微软雅黑），离开时用内置字体。
pub fn font_bytes() -> Option<Vec<u8>> {
    let windir = std::env::var_os("WINDIR")?;
    let path = PathBuf::from(windir).join("Fonts").join("msyh.ttc");
    std::fs::read(path).ok()
}

/// 构建窗口选项：PerMonitorV2 由 eframe 默认处理，150% 缩放下不做额外缩放。
pub fn native_options(title: &str) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 580.0])
            .with_min_inner_size([720.0, 460.0])
            .with_title(title),
        ..Default::default()
    }
}

/// 状态胶囊颜色（供列表与 M4-02 复用）。
pub fn status_color(status: GameStatus) -> egui::Color32 {
    match status {
        GameStatus::Ready => egui::Color32::from_rgb(0x3D, 0x9A, 0x5F),
        GameStatus::NeedsRestore => egui::Color32::from_rgb(0xD0, 0x8A, 0x2E),
        GameStatus::SessionActive => egui::Color32::from_rgb(0x4A, 0x7C, 0xD0),
        GameStatus::Missing => egui::Color32::from_rgb(0x8A, 0x8A, 0x8A),
    }
}

/// 状态胶囊：圆角底色 + 白字，用于列表行。
pub fn status_pill(ui: &mut egui::Ui, status: GameStatus) {
    let color = status_color(status);
    egui::Frame::new()
        .fill(color)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(status.text())
                    .color(egui::Color32::WHITE)
                    .small(),
            );
        });
}
