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

/// 行内按钮触发的动作。
enum RowAction {
    Launch(String),
    Restore(String),
    /// 检测到游戏在跑：强行结束进程并恢复。
    Stop(String),
    Remove(String),
    FrameGen(String),
    Quality(String),
}

/// 标题改名（双击进入）触发的动作。
///
/// 收在这里统一处理，是因为行渲染时只借得到 `view` 与 `rename` 两个字段：
/// 真正写偏好要 `&mut self`，得等循环结束。
enum NameAction {
    Start(crate::rename::RenameState),
    Commit(String),
    Cancel(String),
}

/// 改名输入框的宽度：足够放下常见游戏名，又不把同一行的按钮挤出去。
const RENAME_WIDTH: f32 = 170.0;

/// 主窗口标题：窗口标题、托盘悬停与"再次运行程序"的窗口查找共用这一份。
pub const WINDOW_TITLE: &str = "DLSSG 启动器";

/// 恢复类动作最多等多久：超时按失败弹窗，不把界面挂成"进行中"。
const RESTORE_TIMEOUT_SECONDS: u64 = 180;
/// 启动交出去后先等这么久：引擎在规划/预检阶段失败会立刻写报告，界面要能看到原因。
const RUN_GRACE_SECONDS: u64 = 6;
/// 行状态轮询间隔（游戏进程、部署记录、会话锁都可能被引擎或用户改动）。
const REFRESH_INTERVAL_MS: u64 = 900;
/// 等提权子进程回报告的上限：超时就不再多等，提示重试。
const GPU_TIMEOUT_SECONDS: u64 = 60;

/// 正在等结果的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Run { game: String, started: Instant },
    Restore { game: String, started: Instant },
    Stop { game: String, started: Instant },
}

impl Pending {
    fn game(&self) -> &str {
        match self {
            Self::Run { game, .. } | Self::Restore { game, .. } | Self::Stop { game, .. } => game,
        }
    }

    fn report_name(&self) -> &'static str {
        match self {
            Self::Run { .. } => "ui-run.json",
            Self::Restore { .. } => "ui-restore.json",
            Self::Stop { .. } => "ui-stop.json",
        }
    }

    fn success_title(&self) -> &'static str {
        match self {
            Self::Run { .. } => "启动失败",
            Self::Restore { .. } => "恢复原状完成",
            Self::Stop { .. } => "已结束并恢复",
        }
    }

    fn failure_title(&self) -> &'static str {
        match self {
            Self::Run { .. } => "启动失败",
            Self::Restore { .. } => "恢复原状失败",
            Self::Stop { .. } => "结束并恢复失败",
        }
    }

    fn timeout(&self) -> std::time::Duration {
        match self {
            // 启动只看开头几秒：引擎接管后的会话可能持续很久，不该等着弹窗。
            Self::Run { .. } => std::time::Duration::from_secs(RUN_GRACE_SECONDS),
            _ => std::time::Duration::from_secs(RESTORE_TIMEOUT_SECONDS),
        }
    }

    fn started(&self) -> Instant {
        match self {
            Self::Run { started, .. }
            | Self::Restore { started, .. }
            | Self::Stop { started, .. } => *started,
        }
    }
}

/// 结果弹窗的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Popup {
    ok: bool,
    title: String,
    detail: String,
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
    /// 单实例"再来一次"的激活事件：收到信号就把窗口叫到最前。
    activation: Option<launcher_win::activate::ActivationEvent>,
    /// 激活等待线程是否已经起来（需要 `egui::Context`，第一帧才有）。
    activation_started: bool,
    /// 等待线程收到过激活信号（在 `ui()` 里消费）。
    activate_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 底部提示（保存结果等）。
    notice: String,
    /// 托盘图标（常驻，窗口隐藏后仍可操作）。
    tray: Option<crate::tray::Tray>,
    /// 待确认移除的游戏 id。
    remove_pending: Option<String>,
    /// 交接后隐藏窗口（不退出进程）。
    hide_after_handoff: bool,
    /// 正在等结果的动作（启动/恢复/结束并恢复）。
    pending: Option<Pending>,
    /// 结果弹窗（有值时显示，用户点「确定」才消失）。
    popup: Option<Popup>,
    /// 上次刷新行状态的时刻。
    last_refresh: Option<Instant>,
    /// 后台识别引擎主版本：大 EXE 要扫几秒，不能卡界面。
    detect_job: Option<(
        String,
        std::thread::JoinHandle<launcher_core::unreal::Detection>,
    )>,
    /// 等着识别的游戏（一次只跑一个，剩下的排队）。
    detect_queue: Vec<(String, std::path::PathBuf, std::path::PathBuf)>,
    /// 后台平台扫描（注册表 + 平台清单，可能要几秒）。
    scan_job: Option<std::thread::JoinHandle<crate::scan::ScanResult>>,
    /// 扫描结果窗口（有值时显示）。
    scan_report: Option<crate::scan::ScanResult>,

    /// 添加游戏对话框（M4-05）。
    add_game: Option<crate::add_game::AddGameState>,
    /// 正在改名的行（双击标题进入；回车保存、Esc 放弃）。
    rename: Option<crate::rename::RenameState>,
    /// 切换显卡名称窗口（有值就显示）。
    gpu: Option<crate::gpu::Panel>,
    /// 本次运行里成功改过显卡名称（切换或还原）：窗口里据此显示红字重启按钮。
    ///
    /// 又改回本次运行开始时的名字就不显示——净改动为零，重启没有意义。
    gpu_change: Option<crate::gpu::Change>,
    /// 本次运行第一次成功改动之前的名字（判断"是否已经改回原样"）。
    gpu_changes: crate::gpu::SessionChanges,
    /// 等提权子进程的起始时刻（超时后不再傻等）。
    gpu_busy_since: Option<Instant>,
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

    /// 单实例"再来一次"的激活事件（`main` 拿到界面锁之后创建）。
    pub fn with_activation(
        mut self,
        event: Option<launcher_win::activate::ActivationEvent>,
    ) -> Self {
        self.activation = event;
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

    /// 调试用：开窗后直接摆出某个界面，便于截图核对（`--ui-preview`）。
    pub fn with_preview(mut self, preview: crate::Preview) -> Self {
        if let Some(outcome) = preview.restore_outcome() {
            let ok = outcome.is_ok();
            self.popup = Some(Popup {
                ok,
                title: outcome.title().to_owned(),
                detail: outcome.detail(),
            });
        }
        if let Some(view) = preview.settings_view() {
            self.open_settings(view);
        }
        if let Some(report) = preview.scan_sample() {
            self.scan_report = Some(report);
        }
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
            activation: None,
            activation_started: false,
            activate_pending: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            notice: String::new(),

            tray: None,
            remove_pending: None,
            hide_after_handoff: false,
            pending: None,
            popup: None,
            last_refresh: None,
            detect_job: None,
            detect_queue: Vec::new(),
            scan_job: None,
            scan_report: None,
            add_game: None,
            rename: None,
            gpu: None,
            gpu_change: None,
            gpu_changes: crate::gpu::SessionChanges::default(),
            gpu_busy_since: None,
            show_about: false,
            root: std::path::PathBuf::new(),
        }
    }

    /// 打开某个视图（帧生成 / 画质）：按偏好里的记录填表，缺记录就用默认值。
    fn open_settings(&mut self, view: crate::settings_form::SettingsView) {
        let Some(entry) = self.view.selected_game().cloned() else {
            return;
        };
        let stored = self
            .preferences_path
            .as_deref()
            .and_then(|path| launcher_core::preferences::load(path).ok())
            .and_then(|preferences| preferences.find(&entry.id).cloned());
        let profile = stored.unwrap_or_else(|| {
            let mut profile = launcher_core::preferences::GameProfile::new(
                &entry.id,
                &entry.name,
                entry.directory.clone(),
            );
            profile.engine_host = entry.engine_host.clone();
            profile
        });
        // 有未清理的部署时不允许换代理：旧记录里的代理必须先按原样恢复。
        let pending = launcher_core::journal_store::load_for_game(&self.root, &entry.id)
            .ok()
            .flatten()
            .is_some();
        self.settings = Some(crate::settings_form::SettingsForm::from_profile(
            view, &profile, pending,
        ));
    }

    /// 保存设置到偏好文件；返回是否真的写成功（失败时表单保持打开）。
    fn save_settings(&mut self) -> bool {
        let Some(path) = self.preferences_path.clone() else {
            self.notice = "偏好路径不可用，无法保存".to_owned();
            return false;
        };
        let Some(entry) = self.view.selected_game().cloned() else {
            return false;
        };
        let Some(form) = self.settings.as_mut() else {
            return false;
        };
        let Some(settings) = form.take_for_save() else {
            self.notice = form
                .error
                .clone()
                .unwrap_or_else(|| "校验未通过".to_owned());
            return false;
        };
        let mut preferences = launcher_core::preferences::load(&path).unwrap_or_default();
        let mut profile = preferences.find(&entry.id).cloned().unwrap_or_else(|| {
            let mut profile = launcher_core::preferences::GameProfile::new(
                &entry.id,
                &entry.name,
                entry.directory.clone(),
            );
            profile.engine_host = entry.engine_host.clone();
            profile
        });
        profile.settings = Some(settings);
        if let Err(message) = form.apply_to_profile(&mut profile, &self.root) {
            self.notice = message;
            return false;
        }
        preferences.upsert(profile);
        match launcher_core::preferences::save(&path, &preferences) {
            Ok(()) => {
                self.notice = "设置已保存".to_owned();
                true
            }
            Err(error) => {
                self.notice = format!("保存失败：{error}");
                false
            }
        }
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
                // 启动/恢复/移除已移到每一行；顶栏只保留全局入口。
                if let Some(entry) = self.view.selected_game() {
                    match self.last_restore_outcome() {
                        Some(true) => {
                            ui.label(egui::RichText::new("上次恢复成功").small().weak());
                        }
                        Some(false) => {
                            ui.label(
                                egui::RichText::new("上次恢复失败，可在该行点「恢复原状」重试")
                                    .small()
                                    .weak(),
                            );
                        }
                        None => {}
                    }
                    let _ = entry;
                }
                // 扫描放第一个（用户要求）：平台记录里能认出来的直接加，少让用户点。
                let scanning = self.scan_job.is_some();
                let scan_label = if scanning { "扫描中…" } else { "扫描游戏" };
                if ui
                    .add_enabled(!scanning, egui::Button::new(scan_label))
                    .on_hover_text(
                        "扫 Steam / GOG / Epic 的平台记录与注册表，自动定位鸣潮、异环；\n只查平台，不翻全盘",
                    )
                    .clicked()
                {
                    self.start_scan();
                }
                ui.separator();
                if ui.button("添加游戏").clicked() {
                    self.add_game = Some(crate::add_game::AddGameState::new());
                }
                if ui
                    .button("切换显卡")
                    .on_hover_text(
                        "改显卡在系统里的显示名称（只支持单张 NVIDIA 显卡）；\n改名与还原都要重启电脑才生效",
                    )
                    .clicked()
                {
                    self.open_gpu();
                }
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
                if !self.view.notice.is_empty() {
                    ui.separator();
                    // 底部空间有限，长提示会截断：悬停看全文。
                    let notice = self.view.notice.clone();
                    ui.label(egui::RichText::new(&notice).small())
                        .on_hover_text(&notice);
                }
                // 按钮回调留给 M4-05/M4-06 接引擎；这里先保证布局与可用性正确。
            });
        });

        self.draw_about(ui);
        self.draw_add_game(ui);
        self.draw_settings(ui);
        self.draw_gpu(ui);

        egui::CentralPanel::default().show(ui, |ui| {
            if self.view.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label("还没有游戏。点击下方「添加游戏」开始。");
                });
                return;
            }
            let mut select = None;
            let mut action: Option<RowAction> = None;
            let mut name_actions: Vec<NameAction> = Vec::new();
            // 引擎正跑着哪个游戏：那一行的「恢复原状」要置灰（手动恢复会和引擎抢记录）。
            let engine_owner = self.engine_owner();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, entry) in self.view.games.iter().enumerate() {
                    let selected = self.view.selected == Some(index);
                    let id = entry.id.clone();
                    let engine_is_ue = !entry.engine_host.is_empty() && entry.engine_host != "none";
                    let engine_busy_here = engine_owner.as_deref() == Some(id.as_str());
                    ui.horizontal(|ui| {
                        // 行内操作：启动/结束并恢复在最前，恢复原状只在有遗留部署时出现，移除在最后。
                        if self.view.row_running(index) {
                            // 检测到游戏进程：按钮变成"结束并恢复"，即使游戏是手动启动的。
                            if ui
                                .button(egui::RichText::new("结束并恢复").strong())
                                .on_hover_text(
                                    "游戏正在运行：强行结束它的进程（含官方启动器），然后恢复原状",
                                )
                                .clicked()
                            {
                                action = Some(RowAction::Stop(id.clone()));
                            }
                        } else {
                            let can_launch =
                                entry.status.can_launch() && !self.view.session_running();
                            if ui
                                .add_enabled(can_launch, egui::Button::new("启动"))
                                .on_hover_text("部署资源并启动（二游会先拉起官方启动器）")
                                .clicked()
                            {
                                action = Some(RowAction::Launch(id.clone()));
                            }
                        }
                        // 标题：双击进入改名；编辑时回车保存、Esc 放弃（点别处也算保存）。
                        if let Some(rename) = self.rename.as_mut().filter(|state| state.id == id) {
                            let widget = egui::Id::unique(("game-title", id.as_str()));
                            let fresh = rename.fresh;
                            rename.fresh = false;
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut rename.text)
                                    .id(widget)
                                    .desired_width(RENAME_WIDTH)
                                    .hint_text("游戏名称"),
                            );
                            if fresh {
                                response.request_focus();
                            }
                            if response.gained_focus() {
                                // 全选旧名字：直接输入就是覆盖，不用先删。
                                if let Some(mut state) =
                                    egui::TextEdit::load_state(ui.ctx(), widget)
                                {
                                    let end = egui::text::CCursor::new(rename.text.chars().count());
                                    state.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::two(
                                            egui::text::CCursor::new(0),
                                            end,
                                        ),
                                    ));
                                    state.store(ui.ctx(), widget);
                                }
                            }
                            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                                name_actions.push(NameAction::Cancel(id.clone()));
                            } else if response.lost_focus() {
                                name_actions.push(NameAction::Commit(id.clone()));
                            }
                        } else {
                            let response = ui
                                .selectable_label(selected, &entry.name)
                                .on_hover_text(format!(
                                    "{}\n双击标题可改名（回车保存，Esc 放弃）",
                                    entry.summary()
                                ));
                            if response.double_clicked() {
                                name_actions.push(NameAction::Start(
                                    crate::rename::RenameState::start(&id, &entry.name),
                                ));
                            } else if response.clicked() {
                                select = Some(index);
                            }
                        }
                        if ui
                            .button("帧生成")
                            .on_hover_text("帧生成参数、日志与代理文件")
                            .clicked()
                        {
                            action = Some(RowAction::FrameGen(id.clone()));
                        }
                        if ui
                            .add_enabled(engine_is_ue, egui::Button::new("画质"))
                            .on_hover_text(if engine_is_ue {
                                "画质档位与自定义 INI"
                            } else {
                                "未识别为虚幻引擎，画质档位不适用"
                            })
                            .clicked()
                        {
                            action = Some(RowAction::Quality(id.clone()));
                        }
                        status_pill(ui, entry.status);
                        if engine_is_ue {
                            ui.label(egui::RichText::new(&entry.engine_host).small().weak());
                        }
                        // 游戏在跑时不给「恢复原状」（该走「结束并恢复」）；
                        // 引擎正在跑这个游戏时置灰：手动恢复会和引擎抢同一份部署记录。
                        if entry.status.can_restore() && !self.view.row_running(index) {
                            let clicked = if engine_busy_here {
                                ui.add_enabled(false, egui::Button::new("恢复原状"))
                                    .on_disabled_hover_text(
                                        "引擎正在跑这个游戏；要中止请点「结束并恢复」",
                                    )
                                    .clicked()
                            } else {
                                ui.button("恢复原状")
                                    .on_hover_text("有未清理的部署：按记录恢复游戏目录与配置")
                                    .clicked()
                            };
                            if clicked {
                                action = Some(RowAction::Restore(id.clone()));
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button("移除")
                                .on_hover_text("从列表移除这条记录（不动磁盘）")
                                .clicked()
                            {
                                action = Some(RowAction::Remove(id.clone()));
                            }
                        });
                    });
                    ui.label(entry.directory.display().to_string());
                    ui.separator();
                }
            });
            if let Some(index) = select {
                self.view.select(index);
            }
            for pending in name_actions {
                match pending {
                    NameAction::Start(state) => self.begin_rename(state),
                    NameAction::Commit(id) => self.commit_rename(&id),
                    NameAction::Cancel(id) => self.cancel_rename(&id),
                }
            }
            if let Some(pending) = action {
                self.run_row_action(pending);
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
            running: false,
        });
        if let Some(index) = self
            .view
            .games
            .iter()
            .position(|item| item.id == profile.id)
        {
            self.view.select(index);
        }
        // 添加后立刻后台识别引擎版本：识别得出来就有画质档位，识别不出来等用户手动校准。
        if profile.engine_host == "none" {
            if let Some(relative) = profile.render_executable.as_deref() {
                let relative = relative.trim();
                if !relative.is_empty() {
                    let executable = std::path::PathBuf::from(relative);
                    let executable = if executable.is_absolute() {
                        executable
                    } else {
                        profile.directory.join(executable)
                    };
                    if executable.is_file() {
                        self.start_detection(&profile.id, profile.directory.clone(), executable);
                    }
                }
            }
        }
    }

    /// 处理行内按钮：按动作分派到启动/结束并恢复/恢复/移除/配置。
    fn run_row_action(&mut self, action: RowAction) {
        match action {
            RowAction::Launch(id) => {
                self.view.select_by_id(&id);
                self.hand_off(crate::handoff::Command::Run);
            }
            RowAction::Stop(id) => {
                self.view.select_by_id(&id);
                self.hand_off(crate::handoff::Command::Stop);
            }
            RowAction::Restore(id) => {
                self.view.select_by_id(&id);
                self.hand_off(crate::handoff::Command::Restore);
            }
            RowAction::Remove(id) => self.remove_pending = Some(id),
            RowAction::FrameGen(id) => {
                self.view.select_by_id(&id);
                self.open_settings(crate::settings_form::SettingsView::FrameGen);
            }
            RowAction::Quality(id) => {
                self.view.select_by_id(&id);
                self.open_settings(crate::settings_form::SettingsView::Quality);
            }
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
    /// 结果弹窗（启动失败 / 恢复 / 结束并恢复共用）：点「确定」才消失。
    fn draw_popup(&mut self, root: &mut egui::Ui) {
        let Some(popup) = self.popup.clone() else {
            return;
        };
        let ok = popup.ok;
        let mut close = false;
        let mut open_logs = false;
        // 用 Area + 居中卡片，而不是 Window::anchor：后者按首帧尺寸猜位置，会偏到角落。
        egui::Area::new(egui::Id::unique("restore-result-card"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(root.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(420.0);
                        let color = if ok {
                            egui::Color32::from_rgb(0x3D, 0x9A, 0x5F)
                        } else {
                            egui::Color32::from_rgb(0xD0, 0x4A, 0x4A)
                        };
                        ui.label(
                            egui::RichText::new(&popup.title)
                                .color(color)
                                .size(18.0)
                                .strong(),
                        );
                        ui.separator();
                        ui.label(&popup.detail);
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("确定").clicked() {
                                close = true;
                            }
                            if !ok && ui.button("打开日志").clicked() {
                                open_logs = true;
                            }
                        });
                    });
            });
        if open_logs {
            let logs = crate::about::logs_directory(&self.root);
            let _ = std::fs::create_dir_all(&logs);
            let _ = launcher_win::open_path(&logs);
        }
        if close {
            self.popup = None;
        }
    }
    /// 切换显卡名称窗口：只改注册表里的显示名，写入交给提权子进程。
    ///
    /// 重启提示与"当前是否伪装"无关，只跟**本次运行里成功改过**有关：
    /// 切换和还原都会提示重启，重启电脑或重开启动器后提示自然消失（不检测重启状态）。
    fn draw_gpu(&mut self, root: &mut egui::Ui) {
        if self.gpu.is_none() {
            return;
        }
        // 引擎在用（部署/恢复/游戏在跑）时不许改名、不许重启：改名要重启，重启会打断恢复。
        let engine_busy = self.pending.is_some() || launcher_win::session_active();
        let mut open = true;
        let mut close = false;
        let mut action: Option<crate::gpu::Action> = None;
        let mut refresh = false;
        let mut restart = false;
        egui::Window::new("切换显卡名称")
            .open(&mut open)
            .default_width(560.0)
            .resizable(false)
            .show(root.ctx(), |ui| {
                let Some(panel) = self.gpu.as_mut() else {
                    return;
                };
                let device = panel.situation.as_ref().ok().map(|situation| {
                    (
                        situation.device.display_name.clone(),
                        situation.device.instance_id.clone(),
                    )
                });
                match device {
                    Some((name, instance)) => {
                        ui.label(format!("设备：{name}"));
                        ui.label(egui::RichText::new(instance).small().weak());
                    }
                    None => {
                        ui.label(egui::RichText::new("设备：未找到").small().weak());
                    }
                }
                let red = egui::Color32::from_rgb(0xD0, 0x4A, 0x4A);
                let status = panel.status_text();
                let failed = panel.situation.is_err();
                let text_color = ui.visuals().text_color();
                ui.label(egui::RichText::new(status).color(if failed { red } else { text_color }));
                if !panel.error.is_empty() {
                    ui.label(egui::RichText::new(&panel.error).color(red));
                }
                // 本次运行改过 → 红字提示 + 重启按钮（切换/还原同一套，文案不同）
                if let Some(change) = self.gpu_change {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(change.text()).color(red));
                    });
                    if panel.restart_sent {
                        ui.label(
                            egui::RichText::new("已计划 10 秒后重启；要取消就运行 shutdown /a")
                                .color(red),
                        );
                    } else if panel.confirm_restart {
                        ui.label("确定现在重启？所有程序会被结束，未保存的改动会丢。");
                        ui.label("10 秒后重启，运行 shutdown /a 可取消。");
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    !engine_busy && panel.can_restart(),
                                    egui::Button::new("重启"),
                                )
                                .on_disabled_hover_text("引擎正在跑，先结束游戏并恢复")
                                .clicked()
                            {
                                restart = true;
                            }
                            if ui.button("取消").clicked() {
                                panel.confirm_restart = false;
                            }
                        });
                    } else if ui
                        .add_enabled(
                            !engine_busy && panel.can_restart(),
                            egui::Button::new(egui::RichText::new("立即重启电脑").color(red)),
                        )
                        .on_disabled_hover_text("引擎正在跑，先结束游戏并恢复")
                        .clicked()
                    {
                        panel.confirm_restart = true;
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("目标：");
                    ui.add_enabled(
                        !panel.busy,
                        egui::TextEdit::singleline(&mut panel.target)
                            .desired_width(300.0)
                            .hint_text("NVIDIA GeForce RTX 5070"),
                    );
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let apply_label = if panel.busy {
                        "正在写入…".to_owned()
                    } else {
                        format!(
                            "切换为 {}",
                            launcher_core::gpu::validate_target_name(&panel.target)
                                .unwrap_or_else(|_| "（名称无效）".to_owned())
                        )
                    };
                    if ui
                        .add_enabled(
                            panel.can_apply() && !engine_busy,
                            egui::Button::new(apply_label),
                        )
                        .on_hover_text("先备份原始名称再写注册表；需要重启才生效")
                        .clicked()
                    {
                        action = Some(crate::gpu::Action::Apply);
                    }
                    if ui
                        .add_enabled(
                            panel.can_restore() && !engine_busy,
                            egui::Button::new("恢复原名称"),
                        )
                        .on_hover_text("按状态备份还原；没有备份时按驱动的 DriverDesc 还原")
                        .clicked()
                    {
                        action = Some(crate::gpu::Action::Restore);
                    }
                    if ui
                        .add_enabled(
                            panel.can_reset() && !engine_busy,
                            egui::Button::new("清除旧备份"),
                        )
                        .on_hover_text("驱动更新已恢复真名时用：清掉旧备份，之后可重新切换")
                        .clicked()
                    {
                        action = Some(crate::gpu::Action::ResetState);
                    }
                    if ui
                        .add_enabled(!panel.busy, egui::Button::new("刷新"))
                        .clicked()
                    {
                        refresh = true;
                    }
                    if ui.button("关闭").clicked() {
                        close = true;
                    }
                });
                ui.label(
                    egui::RichText::new(
                        "只改注册表里的显示名称，需重启电脑才生效；不会自动重启、不动驱动，也不改变显卡真实能力。",
                    )
                    .small()
                    .weak(),
                );
                if engine_busy {
                    ui.label(
                        egui::RichText::new("引擎正在跑：改名和重启先放一放，结束游戏并恢复后再来。")
                            .small()
                            .weak(),
                    );
                }
            });
        if refresh {
            self.refresh_gpu_panel();
        }
        if let Some(action) = action {
            self.start_gpu_action(action);
        }
        if restart {
            self.gpu_restart();
        }
        if close {
            open = false;
        }
        // 写入途中不许关窗：报告没人收的话，改没改成、要不要重启都会丢。
        let busy = self.gpu.as_ref().map(|panel| panel.busy).unwrap_or(false);
        if busy {
            open = true;
        }
        if !open {
            self.gpu = None;
            self.gpu_busy_since = None;
        }
    }

    /// 打开显卡窗口：目标名优先用现役目标，其次本机记住的上次名字，最后默认 5070。
    fn open_gpu(&mut self) {
        // 已经开着就只刷新：别把正在进行的写入或重启确认状态清掉。
        if self.gpu.is_some() {
            self.refresh_gpu_panel();
            return;
        }
        let remembered = self
            .preferences_path
            .as_deref()
            .and_then(|path| launcher_core::preferences::load(path).ok())
            .and_then(|preferences| preferences.gpu)
            .map(|gpu| gpu.target_name)
            .unwrap_or_default();
        self.gpu = Some(crate::gpu::Panel::load(&remembered));
    }

    /// 重新读设备事实（动作结束、点「刷新」时）。
    fn refresh_gpu_panel(&mut self) {
        if let Some(panel) = self.gpu.as_mut() {
            panel.refresh();
        }
    }

    /// 记住这次成功切换用的名字：下次打开窗口默认就是它。
    fn remember_gpu_target(&mut self) {
        let Some(target) = self
            .gpu
            .as_ref()
            .map(|panel| panel.target.trim().to_owned())
            .filter(|name| !name.is_empty())
        else {
            return;
        };
        let Some(path) = self.preferences_path.clone() else {
            return;
        };
        let Ok(mut preferences) = launcher_core::preferences::load(&path) else {
            return;
        };
        preferences.gpu = Some(launcher_core::preferences::GpuPreference {
            target_name: target,
        });
        let _ = launcher_core::preferences::save(&path, &preferences);
    }

    /// 把一次显卡名称动作交给提权子进程（界面本身没有写权限）。
    fn start_gpu_action(&mut self, action: crate::gpu::Action) {
        let Some(target) = self.gpu.as_ref().map(|panel| panel.target.clone()) else {
            return;
        };
        if self.gpu.as_ref().map(|panel| panel.busy).unwrap_or(false) {
            return;
        }
        let name = if action == crate::gpu::Action::Apply {
            match launcher_core::gpu::validate_target_name(&target) {
                Ok(value) => Some(value),
                Err(error) => {
                    if let Some(panel) = self.gpu.as_mut() {
                        panel.error = error.to_string();
                    }
                    return;
                }
            }
        } else {
            None
        };
        let report = self.report_path(action.report_name());
        let _ = std::fs::remove_file(&report);
        let executable = match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                if let Some(panel) = self.gpu.as_mut() {
                    panel.error = format!("取自身路径失败：{error}");
                }
                return;
            }
        };
        let arguments = crate::gpu::arguments(action, name.as_deref(), &report);
        // 记下"改之前"的名字：还原成功后再比一次，改回原样就不提示重启。
        if matches!(
            action,
            crate::gpu::Action::Apply | crate::gpu::Action::Restore
        ) {
            if let Some(current) = self
                .gpu
                .as_ref()
                .and_then(|panel| panel.situation.as_ref().ok())
                .map(|situation| situation.current_name.clone())
            {
                self.gpu_changes.remember_baseline(&current);
            }
        }
        if let Some(panel) = self.gpu.as_mut() {
            panel.error.clear();
            panel.confirm_restart = false;
        }
        match launcher_win::elevate::spawn_elevated(&executable, &arguments) {
            Ok(()) => {
                if let Some(panel) = self.gpu.as_mut() {
                    panel.busy = true;
                }
                self.gpu_busy_since = Some(Instant::now());
                self.notice = "已请求管理员权限…".to_owned();
            }
            Err(error) => {
                if let Some(panel) = self.gpu.as_mut() {
                    panel.error = error.to_string();
                }
            }
        }
    }

    /// 轮询提权子进程报告：成功就刷新状态并记下"本次改过什么"。
    fn poll_gpu(&mut self) {
        let busy = self.gpu.as_ref().map(|panel| panel.busy).unwrap_or(false);
        if !busy {
            return;
        }
        // 写入不是原子的：解析失败就等下一帧，别把半截文件当错误。
        let report =
            std::fs::read_to_string(self.report_path(crate::gpu::Action::Status.report_name()))
                .ok()
                .and_then(|text| crate::gpu::Report::parse(&text));
        match report {
            Some(report) => {
                if let Some(panel) = self.gpu.as_mut() {
                    panel.busy = false;
                }
                self.gpu_busy_since = None;
                let action = crate::gpu::Action::parse(&report.action);
                if report.ok {
                    self.notice = report.message.clone();
                    self.refresh_gpu_panel();
                    // 只有"和本次运行开始时的名字不同"才提示重启；又改回去就不提示。
                    if let Some(
                        action @ (crate::gpu::Action::Apply | crate::gpu::Action::Restore),
                    ) = action
                    {
                        let current = self
                            .gpu
                            .as_ref()
                            .and_then(|panel| panel.situation.as_ref().ok())
                            .map(|situation| situation.current_name.clone())
                            .unwrap_or_default();
                        self.gpu_change = self.gpu_changes.settle(&current, action);
                    }
                    if action == Some(crate::gpu::Action::Apply) {
                        self.remember_gpu_target();
                    }
                } else {
                    if let Some(panel) = self.gpu.as_mut() {
                        panel.error = report.error.clone();
                    }
                    self.notice = report.error;
                }
            }
            None => {
                // 子进程可能被拒绝或崩了：等太久就报出来，不能让窗口永远"正在写入"。
                let timed_out = self
                    .gpu_busy_since
                    .map(|started| {
                        started.elapsed() >= std::time::Duration::from_secs(GPU_TIMEOUT_SECONDS)
                    })
                    .unwrap_or(false);
                if timed_out {
                    self.gpu_busy_since = None;
                    if let Some(panel) = self.gpu.as_mut() {
                        panel.busy = false;
                        panel.error =
                            "提权子进程没有回报告（可能被取消或失败），可以再试一次".to_owned();
                    }
                }
            }
        }
    }

    /// 重启电脑（10 秒倒计时；不做任何重启状态检测）。
    fn gpu_restart(&mut self) {
        if self.pending.is_some() || launcher_win::session_active() {
            self.notice = "引擎正在跑，先结束游戏并恢复再重启".to_owned();
            return;
        }
        match crate::gpu::restart(10) {
            Ok(message) => {
                self.notice = message;
                if let Some(panel) = self.gpu.as_mut() {
                    panel.confirm_restart = false;
                    panel.restart_sent = true;
                    panel.error.clear();
                }
            }
            Err(error) => {
                if let Some(panel) = self.gpu.as_mut() {
                    panel.error = error;
                }
            }
        }
    }

    /// 起一次后台平台扫描（注册表 + 平台清单 + 每个游戏的渲染 EXE 确认）。
    fn start_scan(&mut self) {
        if self.scan_job.is_some() {
            self.notice = "扫描已经在进行".to_owned();
            return;
        }
        let root = self.root.clone();
        let existing = self.existing_games();
        self.notice = "正在扫描平台记录…".to_owned();
        self.scan_job = Some(std::thread::spawn(move || {
            crate::scan::scan(&root, &existing)
        }));
    }

    /// 已经在列表里的游戏（ID + 目录），扫描时跳过。
    fn existing_games(&self) -> Vec<(String, std::path::PathBuf)> {
        let mut existing: Vec<(String, std::path::PathBuf)> = self
            .view
            .games
            .iter()
            .map(|entry| (entry.id.clone(), entry.directory.clone()))
            .collect();
        if let Some(path) = self.preferences_path.as_deref() {
            if let Ok(preferences) = launcher_core::preferences::load(path) {
                for profile in &preferences.games {
                    if !existing.iter().any(|(id, _)| id == &profile.id) {
                        existing.push((profile.id.clone(), profile.directory.clone()));
                    }
                }
            }
        }
        existing
    }

    /// 扫描结束后：能确定的直接加，剩下的放进结果窗口等用户确认。
    fn poll_scan(&mut self) {
        if self
            .scan_job
            .as_ref()
            .map(|handle| !handle.is_finished())
            .unwrap_or(false)
        {
            return;
        }
        let Some(handle) = self.scan_job.take() else {
            return;
        };
        let result = match handle.join() {
            Ok(result) => result,
            Err(_) => {
                self.notice = "扫描线程异常结束".to_owned();
                return;
            }
        };
        let mut added = Vec::new();
        for ready in &result.ready {
            self.persist_profile(ready.profile.clone());
            added.push(ready.profile.name.clone());
        }
        if !added.is_empty() {
            self.notice = format!("扫描添加 {} 个：{}", added.len(), added.join("、"));
        } else {
            self.notice = result.summary();
        }
        if !result.ready.is_empty() || !result.ambiguous.is_empty() || !result.notes.is_empty() {
            self.scan_report = Some(result);
        }
    }

    /// 扫描结果窗口：可添加的直接写进列表，需要确认的给一个「选择渲染程序」入口。
    fn draw_scan_report(&mut self, root: &mut egui::Ui) {
        let Some(report) = self.scan_report.clone() else {
            return;
        };
        let mut open = true;
        let mut pick: Option<std::path::PathBuf> = None;
        egui::Window::new("扫描结果")
            .open(&mut open)
            .default_width(620.0)
            .resizable(true)
            .show(root.ctx(), |ui| {
                if report.ready.is_empty() && report.ambiguous.is_empty() {
                    ui.label("没有可添加的新游戏。");
                }
                if !report.ready.is_empty() {
                    ui.label(format!("已自动添加 {} 个：", report.ready.len()));
                    for ready in &report.ready {
                        // 分两行：长路径在 horizontal 里不会换行，会被窗口裁掉。
                        ui.label(format!("· {}", ready.profile.name));
                        ui.label(
                            egui::RichText::new(format!(
                                "  {}（{}）",
                                ready.profile.directory.display(),
                                ready.source
                            ))
                            .small()
                            .weak(),
                        );
                    }
                }
                if !report.ambiguous.is_empty() {
                    ui.separator();
                    ui.label(format!(
                        "需要确认 {} 个（没找到唯一可信的渲染程序）：",
                        report.ambiguous.len()
                    ));
                    for item in &report.ambiguous {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .button("选择渲染程序")
                                    .on_hover_text("打开「添加游戏」，目录已填好")
                                    .clicked()
                                {
                                    pick = Some(item.directory.clone());
                                }
                                ui.label(egui::RichText::new(&item.name).strong());
                            });
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}（{}；{}）",
                                    item.directory.display(),
                                    item.source,
                                    item.reason
                                ))
                                .small()
                                .weak(),
                            );
                        });
                    }
                }
                if !report.notes.is_empty() {
                    ui.separator();
                    for note in &report.notes {
                        ui.label(egui::RichText::new(note).small().weak());
                    }
                }
            });
        if let Some(directory) = pick {
            self.add_game = Some(crate::add_game::AddGameState::prefilled(&directory));
            self.scan_report = None;
        } else if !open {
            self.scan_report = None;
        }
    }

    /// 渲染 EXE 的绝对路径（偏好里存的是相对游戏目录的路径）。
    fn render_executable_of(&self, id: &str) -> Option<std::path::PathBuf> {
        let path = self.preferences_path.as_deref()?;
        let preferences = launcher_core::preferences::load(path).ok()?;
        let profile = preferences.find(id)?;
        let relative = profile.render_executable.as_deref()?;
        if relative.trim().is_empty() {
            return None;
        }
        let candidate = std::path::PathBuf::from(relative);
        Some(if candidate.is_absolute() {
            candidate
        } else {
            profile.directory.join(candidate)
        })
    }

    /// 起一次后台引擎识别（同一时间只跑一个，避免抢磁盘）。
    fn start_detection(
        &mut self,
        game_id: &str,
        directory: std::path::PathBuf,
        executable: std::path::PathBuf,
    ) {
        if self.detect_job.is_some() {
            // 排队：扫描可能一次加好几个游戏，等前面的跑完接着来。
            self.detect_queue
                .push((game_id.to_owned(), directory, executable));
            return;
        }
        let name = self
            .view
            .games
            .iter()
            .find(|entry| entry.id == game_id)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| game_id.to_owned());
        let handle =
            std::thread::spawn(move || launcher_core::unreal::detect(&directory, &executable));
        self.notice = format!("正在识别「{name}」的引擎版本（要扫渲染 EXE，可能需要几秒）…");
        self.detect_job = Some((game_id.to_owned(), handle));
    }

    /// 每帧看一眼后台识别是否结束。
    fn poll_detection(&mut self) {
        if self
            .detect_job
            .as_ref()
            .map(|(_, handle)| !handle.is_finished())
            .unwrap_or(false)
        {
            return;
        }
        let Some((id, handle)) = self.detect_job.take() else {
            // 空闲时把排队的识别接上。
            if let Some((id, directory, executable)) = self.detect_queue.first().cloned() {
                self.detect_queue.remove(0);
                self.start_detection(&id, directory, executable);
            }
            return;
        };
        match handle.join() {
            Ok(detection) => self.apply_detection(&id, &detection),
            Err(_) => self.notice = "引擎识别线程异常结束，请重试".to_owned(),
        }
        if let Some((id, directory, executable)) = self.detect_queue.first().cloned() {
            self.detect_queue.remove(0);
            self.start_detection(&id, directory, executable);
        }
    }

    /// 识别结果写回偏好与界面；只有唯一主版本才写，冲突/未知一律保留原值。
    fn apply_detection(&mut self, id: &str, detection: &launcher_core::unreal::Detection) {
        let host = match detection.major {
            Some(4) => "UE4",
            Some(5) => "UE5",
            _ => {
                self.notice = format!(
                    "引擎识别：{}（{}）；可手动校准",
                    detection.text(),
                    detection.source
                );
                return;
            }
        };
        self.write_engine_host(id, host);
        if let Some(form) = self.settings.as_mut() {
            form.engine_host = host.to_owned();
            form.engine_host_is_ue = true;
            form.dirty = true;
        }
        self.notice = format!("引擎识别：{host}（{}）", detection.source);
    }

    /// 双击标题：开始编辑这一行（同一时间只编辑一行，先落盘上一行的改动）。
    fn begin_rename(&mut self, state: crate::rename::RenameState) {
        if let Some(previous) = self.rename.as_ref().map(|current| current.id.clone()) {
            self.commit_rename(&previous);
        }
        self.rename = Some(state);
    }

    /// 回车（或点别处）：名字有实质变化就写进偏好，否则什么都不做。
    ///
    /// 改的只是 `name`：id、目录、部署记录都不动，所以引擎在跑、有未完成的部署时改名也安全。
    fn commit_rename(&mut self, id: &str) {
        if self.rename.as_ref().map(|state| state.id.as_str()) != Some(id) {
            return;
        }
        let Some(state) = self.rename.take() else {
            return;
        };
        let name = match state.normalized() {
            Ok(name) => name,
            Err(message) => {
                self.notice = message;
                // 名字不合法：输入框留着让用户接着改，别把他刚打的字丢掉。
                self.rename = Some(crate::rename::RenameState {
                    fresh: true,
                    ..state
                });
                return;
            }
        };
        if name == state.original {
            return;
        }
        let Some(path) = self.preferences_path.clone() else {
            self.notice = "偏好路径不可用，改名没有保存".to_owned();
            return;
        };
        let mut preferences = match launcher_core::preferences::load(&path) {
            Ok(preferences) => preferences,
            Err(error) => {
                self.notice = format!("偏好读不出来，改名没有保存：{error}");
                return;
            }
        };
        let Some(profile) = preferences.find(id).cloned() else {
            self.notice = format!("偏好里没有这条记录，改名没有保存：{id}");
            return;
        };
        let mut updated = profile;
        updated.name = name.clone();
        preferences.upsert(updated);
        match launcher_core::preferences::save(&path, &preferences) {
            Ok(()) => {
                if let Some(entry) = self.view.games.iter_mut().find(|entry| entry.id == id) {
                    entry.name = name.clone();
                }
                self.notice = format!("已重命名为「{name}」");
            }
            Err(error) => self.notice = format!("改名保存失败：{error}"),
        }
    }

    /// Esc：放弃这一行的编辑，原名不动。
    fn cancel_rename(&mut self, id: &str) {
        if self.rename.as_ref().map(|state| state.id.as_str()) == Some(id) {
            self.rename = None;
        }
    }

    /// 把引擎宿主写进偏好并刷新该行（识别与手动校准共用）。
    fn write_engine_host(&mut self, id: &str, host: &str) {
        if let Some(entry) = self.view.games.iter_mut().find(|entry| entry.id == id) {
            entry.engine_host = host.to_owned();
        }
        let Some(path) = self.preferences_path.clone() else {
            return;
        };
        let Ok(mut preferences) = launcher_core::preferences::load(&path) else {
            return;
        };
        let Some(profile) = preferences.find(id).cloned() else {
            return;
        };
        let mut updated = profile;
        updated.engine_host = host.to_owned();
        preferences.upsert(updated);
        let _ = launcher_core::preferences::save(&path, &preferences);
    }

    /// 读最近一次"恢复原状"的报告：`Some(true)` 成功、`Some(false)` 失败、`None` 未跑过。
    fn last_restore_outcome(&self) -> Option<bool> {
        let text = std::fs::read_to_string(self.report_path("ui-restore.json")).ok()?;
        if text.contains("\"code\": \"restored\"") {
            Some(true)
        } else if text.contains("\"error\"") {
            Some(false)
        } else {
            None
        }
    }

    /// 引擎报告路径（引擎子进程写、界面读）。
    fn report_path(&self, name: &str) -> std::path::PathBuf {
        crate::about::logs_directory(&self.root).join(name)
    }

    /// 负载里有没有超分内核（轻量版没有）：`<负载根>\dlssg_sm86\sr\sr_kernels.dll`。
    fn super_resolution_available(&self) -> bool {
        crate::engine::resource_root_of(&self.root)
            .join(r"dlssg_sm86\sr\sr_kernels.dll")
            .is_file()
    }

    /// 引擎当前占着会话、并且在跑哪个游戏（读 `logs\status.json`，并核对它的 pid 还活着）。    ///
    /// 用来把「恢复原状」在"引擎正跑着这个游戏"时置灰：这时手动恢复会和引擎抢同一份
    /// 部署记录，既容易失败，也会把游戏留在半个部署状态。该走「结束并恢复」。
    fn engine_owner(&self) -> Option<String> {
        if !launcher_win::session_active() {
            return None;
        }
        let text = std::fs::read_to_string(self.report_path("status.json")).ok()?;
        let game_id = json_text(&text, "game_id")?;
        let pid: u32 = json_text(&text, "pid")?.parse().ok()?;
        launcher_win::process_running(pid).then_some(game_id)
    }

    /// 按当前磁盘事实重算一行状态。
    fn refresh_status(&mut self, id: &str) {
        let pending = launcher_core::journal_store::load_for_game(&self.root, id)
            .ok()
            .flatten()
            .is_some();
        let (exists, running) = self
            .view
            .games
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| (entry.directory.is_dir(), entry.running))
            .unwrap_or((false, false));
        let status = crate::view_model::derive_status(exists, pending, running);
        if let Some(entry) = self.view.games.iter_mut().find(|entry| entry.id == id) {
            entry.status = status;
        }
    }

    /// 周期性刷新所有行：状态（部署记录 + **本行**游戏进程）与全局会话占用。
    ///
    /// 以前把"全局会话在跑"写进每一行的状态，结果一款游戏在跑会让所有行都显示"运行中"，
    /// 真正在跑的那一行反而显示"已部署"（2026-11-23 真机反馈）。现在状态只看本行事实，
    /// 全局会话只用来禁用其它行的「启动」。
    fn refresh_rows(&mut self) {
        let due = self
            .last_refresh
            .map(|last| last.elapsed().as_millis() >= REFRESH_INTERVAL_MS as u128)
            .unwrap_or(true);
        if !due {
            return;
        }
        self.last_refresh = Some(Instant::now());
        self.view.session_taken = launcher_win::session_active();
        let profiles = self
            .preferences_path
            .as_deref()
            .and_then(|path| launcher_core::preferences::load(path).ok())
            .map(|preferences| preferences.games)
            .unwrap_or_default();
        for index in 0..self.view.games.len() {
            let id = self.view.games[index].id.clone();
            let directory = self.view.games[index].directory.clone();
            let pending = launcher_core::journal_store::load_for_game(&self.root, &id)
                .ok()
                .flatten()
                .is_some();
            let running = match profiles.iter().find(|profile| profile.id == id) {
                Some(profile) => crate::engine::is_game_running(profile),
                None => false,
            };
            let entry = &mut self.view.games[index];
            entry.status = crate::view_model::derive_status(directory.is_dir(), pending, running);
            entry.running = running;
        }
    }

    /// 把「启动 / 恢复原状 / 结束并恢复」交给提权后的引擎子进程。
    fn hand_off(&mut self, command: crate::handoff::Command) {
        let Some(game) = self.view.selected_game().map(|entry| entry.id.clone()) else {
            self.notice = "先选择一个游戏".to_owned();
            return;
        };
        let logs = crate::about::logs_directory(&self.root);
        let _ = std::fs::create_dir_all(&logs);
        let report = logs.join(command.report_name());
        // 上一轮的报告会让界面误判成本轮结果，先清掉。
        let _ = std::fs::remove_file(&report);
        match crate::handoff::hand_off(command, &game, &self.root, &report) {
            Ok(()) => {
                let started = Instant::now();
                self.pending = Some(match command {
                    crate::handoff::Command::Run => Pending::Run {
                        game: game.clone(),
                        started,
                    },
                    crate::handoff::Command::Restore => Pending::Restore {
                        game: game.clone(),
                        started,
                    },
                    crate::handoff::Command::Stop => Pending::Stop {
                        game: game.clone(),
                        started,
                    },
                });
                self.notice = match command {
                    crate::handoff::Command::Run => "已交给引擎，正在等它接管…".to_owned(),
                    crate::handoff::Command::Restore => "正在恢复原状…".to_owned(),
                    crate::handoff::Command::Stop => "正在结束进程并恢复…".to_owned(),
                };
            }
            Err(error) => self.notice = format!("交接失败：{error}"),
        }
    }

    /// 每帧看一眼正在等的动作有没有结果。
    fn poll_pending(&mut self) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let report = self.report_path(pending.report_name());
        match crate::restore_result::read(&report) {
            Some(outcome) => {
                self.pending = None;
                // 启动成功的报告要等整局结束才写，这里通常只在失败时命中。
                if pending.report_name() == "ui-run.json" && outcome.is_ok() {
                    return;
                }
                self.popup = Some(Popup {
                    ok: outcome.is_ok(),
                    title: if outcome.is_ok() {
                        pending.success_title().to_owned()
                    } else {
                        pending.failure_title().to_owned()
                    },
                    detail: outcome.detail(),
                });
                self.refresh_status(pending.game());
            }
            None => {
                if pending.started().elapsed() >= pending.timeout() {
                    self.pending = None;
                    match pending {
                        // 启动：引擎接管后就该把界面藏起来；失败的话上面已经弹窗了。
                        Pending::Run { .. } => {
                            self.hide_after_handoff = true;
                            self.notice = "引擎已接管，界面已隐藏（托盘图标可恢复）".to_owned();
                        }
                        _ => {
                            self.popup = Some(Popup {
                                ok: false,
                                title: pending.failure_title().to_owned(),
                                detail: format!(
                                    "等待 {} 秒仍没有结果。\n引擎可能还在结束游戏进程，或提权被拒绝。请查看日志后再试。",
                                    pending.timeout().as_secs()
                                ),
                            });
                        }
                    }
                }
            }
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
                                // 置信度/分数只在内部用于排序与自动预选，不在界面上显示。
                                format!(
                                    "{}  ·  {}",
                                    candidate.relative,
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

    /// 设置窗口：帧生成（12 键 + 代理文件）与画质（档位 + 自定义 INI）两个视图。
    fn draw_settings(&mut self, root: &mut egui::Ui) {
        let Some(mut form) = self.settings.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        egui::Window::new(form.view.title())
            .open(&mut open)
            .default_width(560.0)
            .resizable(true)
            .show(root.ctx(), |ui| {
                match form.view {
                    crate::settings_form::SettingsView::FrameGen => {
                        self.draw_frame_gen(ui, &mut form)
                    }
                    crate::settings_form::SettingsView::Quality => self.draw_quality(ui, &mut form),
                }
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
            if self.save_settings() {
                self.settings = None;
            }
        } else if open {
            self.settings = Some(form);
        }
    }

    /// 帧生成视图：常用 5 项 + 高级 7 项 + 自定义 INI + 代理文件。
    fn draw_frame_gen(&mut self, ui: &mut egui::Ui, form: &mut crate::settings_form::SettingsForm) {
        use crate::settings_form::{ADVANCED_KEYS, COMMON_KEYS};
        use launcher_core::settings::{
            EngineKind, FlowResolution, RuntimeMode, SuperResolutionMode, UseHudless,
        };
        let states = form.field_states();
        if !self.super_resolution_available() {
            ui.colored_label(
                egui::Color32::from_rgb(0xD0, 0x8A, 0x2E),
                "轻量版：没带超分内核（dlssg_sm86\\sr），SuperResolution.Mode 只能用 Off；要用超分请换全量包。",
            );
        }
        ui.label(format!("常用（{} 项）", COMMON_KEYS.len()));
        ui.add_enabled_ui(true, |ui| {
            ui.checkbox(&mut form.settings.enabled, "Enabled · 启用组件")
                .on_hover_text(crate::settings_form::key_hint("Enabled"));
        });
        ui.add_enabled_ui(form.settings.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Engine · 超分引擎")
                    .on_hover_text(crate::settings_form::key_hint("Engine"));
                ui.selectable_value(&mut form.settings.engine, EngineKind::Dlss, "DLSS");
                let xe = ui.selectable_value(&mut form.settings.engine, EngineKind::XeSs, "XeSS");
                if xe.clicked() {
                    form.set_engine(EngineKind::XeSs);
                }
            });
            ui.horizontal(|ui| {
                ui.label("MaxGeneratedFrames · 生成帧数")
                    .on_hover_text(crate::settings_form::key_hint("MaxGeneratedFrames"));
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
                let sr_available = self.super_resolution_available();
                ui.label("SuperResolution.Mode")
                    .on_hover_text(crate::settings_form::key_hint("SuperResolution.Mode"));
                for (mode, text) in [
                    (SuperResolutionMode::Off, "Off"),
                    (SuperResolutionMode::MInt8, "M-Int8"),
                    (SuperResolutionMode::LInt8, "L-Int8"),
                    (SuperResolutionMode::MFp16, "M-Fp16"),
                    (SuperResolutionMode::LFp16, "L-Fp16"),
                ] {
                    // 轻量版没有 sr 内核：除 Off 之外的档位置灰，别让用户选了再失败。
                    let enabled = sr_available || mode == SuperResolutionMode::Off;
                    let response = ui.add_enabled_ui(enabled, |ui| {
                        ui.selectable_value(&mut form.settings.super_resolution, mode, text)
                    });
                    let response = if enabled {
                        response.inner
                    } else {
                        response.inner.on_disabled_hover_text(
                            "当前是轻量版，不含超分内核（sr_kernels.dll / nvngx_dlss.dll）",
                        )
                    };
                    if response.clicked() {
                        form.set_super_resolution(mode);
                    }
                }
            });
            ui.add_enabled_ui(states.advanced_enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Dlssg.Preset")
                        .on_hover_text(crate::settings_form::key_hint("Dlssg.Preset"));
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
                    ui.label("Level")
                        .on_hover_text(crate::settings_form::key_hint("Level"));
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
                        ui.label("Optimized")
                            .on_hover_text(crate::settings_form::key_hint("Optimized"));
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
                        ui.label("Logging.Directory")
                            .on_hover_text(crate::settings_form::key_hint("Logging.Directory"));
                        if ui
                            .text_edit_singleline(&mut form.settings.directory)
                            .changed()
                        {
                            form.dirty = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("FlowResolution")
                            .on_hover_text(crate::settings_form::key_hint("FlowResolution"));
                        for (mode, text) in [
                            (FlowResolution::Coarse, "Coarse"),
                            (FlowResolution::Middle, "Middle"),
                            (FlowResolution::Fine, "Fine"),
                            (FlowResolution::CoarseAt4K, "Coarse@4K"),
                        ] {
                            if ui
                                .selectable_value(&mut form.settings.flow_resolution, mode, text)
                                .clicked()
                            {
                                form.dirty = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("UseHudless")
                            .on_hover_text(crate::settings_form::key_hint("UseHudless"));
                        for (mode, text) in [
                            (UseHudless::Auto, "Auto"),
                            (UseHudless::Zero, "0"),
                            (UseHudless::One, "1"),
                        ] {
                            if ui
                                .selectable_value(&mut form.settings.use_hudless, mode, text)
                                .clicked()
                            {
                                form.dirty = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Runtime.Mode")
                            .on_hover_text(crate::settings_form::key_hint("Runtime.Mode"));
                        for (mode, text) in [
                            (RuntimeMode::Bundled, "Bundled"),
                            (RuntimeMode::External, "External"),
                            (RuntimeMode::Pinned, "Pinned"),
                        ] {
                            if ui
                                .selectable_value(&mut form.settings.runtime_mode, mode, text)
                                .clicked()
                            {
                                form.set_runtime_mode(mode);
                            }
                        }
                    });
                    ui.add_enabled_ui(states.runtime_path, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Runtime.Path")
                                .on_hover_text(crate::settings_form::key_hint("Runtime.Path"));
                            let mut text = form.settings.runtime_path.clone().unwrap_or_default();
                            if ui.text_edit_singleline(&mut text).changed() {
                                form.settings.runtime_path =
                                    (!text.trim().is_empty()).then_some(text);
                                form.dirty = true;
                            }
                        });
                    });
                });
            });

        // 「自定义 INI」只属于画质视图（05-自定义 里选文件），帧生成这边不再重复给一份。
        // 旧偏好里可能还留着"组件配置全文"：给一个清掉它的出口，否则预设会一直灰着。
        if form.settings.custom_ini.is_some() {
            ui.horizontal(|ui| {
                ui.colored_label(
                    egui::Color32::from_rgb(0xD0, 0x8A, 0x2E),
                    "这条记录用的是「组件配置全文」，12 键预设不生效。",
                );
                if ui
                    .button("清除全文，改回 12 键")
                    .on_hover_text("删除记录里保存的整份 INI，改由上面的 12 个键生成")
                    .clicked()
                {
                    form.settings.custom_ini = None;
                    form.dirty = true;
                }
            });
        }
        ui.separator();
        self.draw_proxy_section(ui, form);
    }

    /// 代理文件选择：编号、名称、来源与建议；有遗留部署时整块灰掉。
    fn draw_proxy_section(
        &mut self,
        ui: &mut egui::Ui,
        form: &mut crate::settings_form::SettingsForm,
    ) {
        use crate::settings_form::SettingsForm;
        egui::CollapsingHeader::new("代理文件")
            .default_open(true)
            .show(ui, |ui| {
                let states = form.field_states();
                let game = self.view.selected_game().cloned();
                // 渲染目录里已有的普通文件 = 别人的注入器占着这个位置，本程序不覆盖。
                let occupied = game
                    .as_ref()
                    .and_then(|entry| self.render_executable_of(&entry.id))
                    .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
                    .map(|directory| {
                        SettingsForm::proxy_options()
                            .into_iter()
                            .filter_map(|option| {
                                let path = directory.join(option.name);
                                let metadata = std::fs::symlink_metadata(&path).ok()?;
                                // 链接是我们自己部署的，不算被占用。
                                (!metadata.file_type().is_symlink()).then_some(option.name)
                            })
                            .collect::<Vec<&'static str>>()
                    })
                    .unwrap_or_default();
                ui.add_enabled_ui(states.proxy_choice, |ui| {
                    for option in SettingsForm::proxy_options() {
                        let selected = form.proxy_index == option.index;
                        let blocked = occupied.contains(&option.name);
                        let text = format!(
                            "{}  {}（{}）{}",
                            option.index,
                            option.name,
                            option.location,
                            if blocked { "  已被占用" } else { "" }
                        );
                        let hover = if blocked {
                            "该位置已有别的注入器/模组加载器的文件，本程序不会覆盖它；换一个代理"
                        } else {
                            option.note
                        };
                        let label = if blocked {
                            egui::RichText::new(text)
                                .color(egui::Color32::from_rgb(0xD0, 0x8A, 0x2E))
                        } else {
                            egui::RichText::new(text)
                        };
                        if ui
                            .selectable_label(selected, label)
                            .on_hover_text(hover)
                            .clicked()
                        {
                            form.set_proxy(option.index);
                        }
                    }
                });
                if !occupied.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xD0, 0x8A, 0x2E),
                        format!("已被占用：{}（属于别的工具，不动它）", occupied.join("、")),
                    );
                }
                ui.label(
                    egui::RichText::new(format!("当前：{}", form.proxy_name()))
                        .small()
                        .weak(),
                );
                if !states.proxy_choice {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xD0, 0x8A, 0x2E),
                        "该游戏还有未清理的部署：先点该行的「恢复原状」，才能换代理。",
                    );
                }
                if let Some(game) = game {
                    if let Some(hint) = form.proxy_hint(&game.id, &game.directory) {
                        ui.colored_label(egui::Color32::from_rgb(0xD0, 0x8A, 0x2E), hint);
                    }
                }
                ui.label(
                    egui::RichText::new("只部署一个代理；换代理要重新部署，旧的会按记录先清掉。")
                        .small()
                        .weak(),
                );
            });
    }

    /// 画质视图：引擎版本 + 4 档画质 + `05-自定义` + 配置源结论。
    fn draw_quality(&mut self, ui: &mut egui::Ui, form: &mut crate::settings_form::SettingsForm) {
        use crate::settings_form::SettingsForm;

        ui.horizontal(|ui| {
            ui.label("引擎版本");
            ui.label(
                egui::RichText::new(if form.engine_host_is_ue {
                    form.engine_host.clone()
                } else {
                    "尚未确定".to_owned()
                })
                .strong(),
            );
            let busy = self.detect_job.is_some();
            let label = if busy { "识别中…" } else { "重新识别" };
            if ui
                .add_enabled(!busy, egui::Button::new(label))
                .on_hover_text("读 Engine\\Build\\Build.version、渲染 EXE 的标记与版本信息；冲突或缺失不猜")
                .clicked()
            {
                let game = self.view.selected_game().cloned();
                if let Some(game) = game {
                    match self.render_executable_of(&game.id) {
                        Some(executable) if executable.is_file() => {
                            self.start_detection(&game.id, game.directory.clone(), executable);
                        }
                        _ => {
                            self.notice =
                                "这条记录没有可用的渲染程序，识别不了；先在「添加游戏」里扫描确认目录"
                                    .to_owned();
                        }
                    }
                }
            }
            ui.label(egui::RichText::new("手动校准：").small().weak());
            for host in ["UE4", "UE5"] {
                let selected = form.engine_host == host;
                if ui.selectable_label(selected, host).clicked() {
                    form.engine_host = host.to_owned();
                    form.engine_host_is_ue = true;
                    form.dirty = true;
                    if let Some(game) = self.view.selected_game().cloned() {
                        // 手动校准立即生效，不用等保存按钮（与 PS 的"手动校准"一致）。
                        self.write_engine_host(&game.id, host);
                    }
                }
            }
        });
        if !form.engine_host_is_ue {
            ui.colored_label(
                egui::Color32::from_rgb(0xD0, 0x8A, 0x2E),
                "未识别为虚幻引擎，画质档位不适用；确认识别结果后再来选档位。",
            );
        }

        ui.separator();
        ui.label("画质档位");
        ui.add_enabled_ui(form.engine_host_is_ue, |ui| {
            for tier in SettingsForm::quality_options() {
                let selected = form.quality_tier == tier;
                if ui
                    .selectable_label(selected, tier.folder())
                    .on_hover_text(match tier {
                        launcher_core::quality::QualityTier::Tier5 => {
                            "用你自己填的 Engine.ini；不做合并，原样部署"
                        }
                        _ => "仓库里的通用分类档位（源文件保持可编辑）",
                    })
                    .clicked()
                {
                    form.set_quality(tier);
                }
            }
        });

        if form.quality_tier == launcher_core::quality::QualityTier::Tier5 {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("自定义 INI");
                ui.add_enabled_ui(form.engine_host_is_ue, |ui| {
                    ui.text_edit_singleline(&mut form.custom_ini_path);
                });
                if ui.button("浏览…").clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("Engine.ini", &["ini"])
                        .pick_file()
                    {
                        form.custom_ini_path = file.to_string_lossy().into_owned();
                        form.dirty = true;
                    }
                }
            });
        }

        ui.separator();
        let summary = form.quality_summary(&self.root);
        ui.label(summary);
        ui.label(
            egui::RichText::new(
                "部署的是带标记的临时副本（只读），原件按字节备份后恢复；源文件不会被改动。",
            )
            .small()
            .weak(),
        );
    }
}

impl LauncherApp {
    /// 第一帧才起等待线程：线程里要 `egui::Context` 来唤醒重绘。
    fn start_activation_waiter(&mut self, ctx: &egui::Context) {
        if self.activation_started {
            return;
        }
        self.activation_started = true;
        let Some(event) = self.activation.take() else {
            return;
        };
        let pending = self.activate_pending.clone();
        let ctx = ctx.clone();
        launcher_win::activate::spawn_waiter(event, move || {
            pending.store(true, std::sync::atomic::Ordering::SeqCst);
            // 窗口隐藏时 Windows 不发 RedrawRequested；eframe 会给不可见窗口直接跑帧，
            // 这里主动要一帧，让 `Visible(true)` 尽快生效。
            ctx.request_repaint();
        });
    }

    /// 把窗口叫回前台（托盘「显示窗口」与"再次运行程序"共用这一条路径）。
    ///
    /// 三件事缺一不可：清掉"交接后隐藏"（否则下一帧又会把自己藏回去）、`Visible(true)`
    /// （从托盘恢复）、`Minimized(false)` + `Focus`（最小化或压在别的窗口后面时回到最前）。
    fn show_main_window(&mut self, ctx: &egui::Context) {
        self.hide_after_handoff = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.tray.is_none() {
            self.tray = crate::tray::Tray::new(WINDOW_TITLE);
        }
        // 再次运行程序：等待线程已经收到信号 → 走和托盘「显示窗口」同一条路径。
        self.start_activation_waiter(root.ctx());
        if self
            .activate_pending
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.show_main_window(root.ctx());
        }
        if let Some(action) = self.tray.as_ref().and_then(|tray| tray.poll()) {
            match action {
                crate::tray::Action::Show => self.show_main_window(root.ctx()),
                crate::tray::Action::Logs => {
                    let logs = crate::about::logs_directory(&self.root);
                    let _ = std::fs::create_dir_all(&logs);
                    let _ = launcher_win::open_path(&logs);
                }
                crate::tray::Action::Quit => {
                    // 直接结束进程。eframe 0.37 的 `ViewportCommand::Close` 只是往 viewport
                    // 里塞一个 `ViewportEvent::Close`，没有任何人处理它——窗口不会关、
                    // 进程也不会退（真机 2026-11-23：托盘"退出"点了没反应）。
                    // 界面没有未落盘的状态（偏好改完即写），直接退出最省事也最可靠。
                    std::process::exit(0);
                }
            }
        }
        // 关闭按钮 = 收进托盘（进程继续活着，托盘左键能再打开）；真退出走托盘「退出」。
        if root.ctx().input(|input| input.viewport().close_requested()) {
            root.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            root.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hide_after_handoff = true;
        }
        // 背景扫描：跑着就持续重绘，结束后把能确定的直接加进来。
        self.poll_scan();
        if self.scan_job.is_some() {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        // 显卡名称切换：等提权子进程的报告。
        self.poll_gpu();
        if self.gpu.as_ref().map(|panel| panel.busy).unwrap_or(false) {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.draw_scan_report(root);
        // 后台识别引擎主版本：跑着就持续要重绘，结束后写回偏好。
        self.poll_detection();
        if self.detect_job.is_some() {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        // 行状态与"游戏是否在跑"定期重算：引擎部署、游戏启动/退出都不需要重开界面。
        self.refresh_rows();
        root.ctx()
            .request_repaint_after(std::time::Duration::from_millis(REFRESH_INTERVAL_MS));
        // 正在等结果的动作（启动/恢复/结束并恢复）。
        self.poll_pending();
        if self.pending.is_some() {
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
        self.draw_popup(root);

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
                // 与托盘"退出"同理：程序化关窗在 eframe 0.37 里不生效，直接结束进程。
                std::process::exit(0);
            }
        }
    }
}

/// 载入系统字体（微软雅黑），离开时用内置字体。
///
/// 优先只读内存映射：`msyh.ttc` 有 18.8 MB，整块读进堆会变成常驻专用内存（epaint 的
/// `FontData` 一直持有这份字节）；映射成文件页后只有用到的字形被换入，内存吃紧时系统能直接
/// 回收。映射不可用（字体缺失、权限不足、空文件）时回退整读，行为与旧版完全一致。
pub fn font_data() -> Option<egui::FontData> {
    let windir = std::env::var_os("WINDIR")?;
    let path = PathBuf::from(windir).join("Fonts").join("msyh.ttc");
    if let Some(blob) = launcher_win::fontmap::font_blob(&path) {
        return Some(egui::FontData::from_blob(blob, 0));
    }
    std::fs::read(&path).ok().map(egui::FontData::from_owned)
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

/// 从一段紧凑 JSON 里取一个字符串/数字字段的原文（`{"game_id":"x","pid":1}`）。
fn json_text(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    if let Some(after) = rest.strip_prefix('"') {
        let end = after.find('"')?;
        return Some(after[..end].to_owned());
    }
    let end = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_status_json_is_parsed() {
        // 与 `ipc::StatusSnapshot::to_json` 的真实形状一致
        let text = r#"{"game_id":"direct-790c6651f94e333f0469a56d9c5faab5","phase":"starting","pid":14672,"updated_at":1795414816,"message":"引擎已接管"}"#;
        assert_eq!(
            json_text(text, "game_id").as_deref(),
            Some("direct-790c6651f94e333f0469a56d9c5faab5")
        );
        assert_eq!(json_text(text, "pid").as_deref(), Some("14672"));
        assert_eq!(json_text(text, "message").as_deref(), Some("引擎已接管"));
        assert!(json_text(text, "nope").is_none());
        assert!(json_text("not json", "game_id").is_none());
    }
}
