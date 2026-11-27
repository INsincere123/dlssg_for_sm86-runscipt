//! 主窗口（M4-01）：只负责画，所有判断在 [`crate::view_model`]。
//!
//! `--ui-selftest <ms>`：开窗后自动关闭，并把 `ui=ok`、`pixels_per_point`、首帧耗时写进报告，
//! 供验收脚本测量内存与冷启动。

use eframe::egui;
use launcher_core::preferences::{GameProfile, Preferences};
use launcher_core::quality::EngineHost;
use std::path::PathBuf;
use std::time::Instant;

use crate::handoff::{self, Command, Event, Outcome, Pending, Request};
use crate::settings_form::{SettingsForm, SettingsView};
use crate::tray::{ExitAction, GuiCommand, GuiExit, GuiMessage};
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
    Engine(String),
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

/// 行状态轮询间隔（游戏进程、部署记录、会话锁都可能被引擎或用户改动）。
const REFRESH_INTERVAL_MS: u64 = 900;
/// 等提权子进程回报告的上限：超时就不再多等，提示重试。
const GPU_TIMEOUT_SECONDS: u64 = 60;

/// 结果弹窗的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Popup {
    ok: bool,
    title: String,
    detail: String,
}

struct DetectionTarget {
    id: String,
    directory: PathBuf,
    executable: PathBuf,
    engine_host: String,
}

impl DetectionTarget {
    fn from_profile(profile: &GameProfile) -> Result<Self, String> {
        let executable = crate::engine::render_path(profile)
            .filter(|path| path.is_file())
            .ok_or_else(|| "该游戏没有可用于识别的渲染程序。".to_owned())?;
        Ok(Self {
            id: profile.id.clone(),
            directory: profile.directory.clone(),
            executable,
            engine_host: profile.engine_host.clone(),
        })
    }

    fn still_matches(&self, profile: &GameProfile) -> bool {
        self.id == profile.id
            && self.directory == profile.directory
            && crate::engine::render_path(profile).as_ref() == Some(&self.executable)
            && self.engine_host == profile.engine_host
    }
}

struct DetectionJob {
    target: Option<DetectionTarget>,
    handle: std::thread::JoinHandle<launcher_core::unreal::Detection>,
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
    /// 等待线程收到过激活信号（在 `logic()` 里消费）。
    activate_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 原生托盘宿主与它送来的界面命令。
    tray_host: Option<u32>,
    tray_commands: Option<std::sync::mpsc::Receiver<GuiMessage>>,
    tray_handled: u64,
    /// 待确认移除的游戏 id。
    remove_pending: Option<String>,
    /// 正在等结果的动作（启动/恢复/结束并恢复）。
    pending: Vec<Pending>,
    handoff_popups: std::collections::VecDeque<Popup>,
    /// 结果弹窗（有值时显示，用户点「确定」才消失）。
    popup: Option<Popup>,
    /// 上次刷新行状态的时刻。
    last_refresh: Option<Instant>,
    /// 后台识别引擎主版本：大 EXE 要扫几秒，不能卡界面。
    detect_job: Option<DetectionJob>,
    /// 等着识别的游戏（一次只跑一个，剩下的排队）。
    detect_queue: std::collections::VecDeque<DetectionTarget>,
    engine_dialog: Option<String>,
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
    pub fn with_tray_host(mut self, pid: Option<u32>) -> Self {
        self.tray_host = pid;
        self
    }

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
        if let Some((ok, title, detail)) = preview.restore_outcome() {
            self.popup = Some(Popup {
                ok,
                title: title.to_owned(),
                detail: detail.to_owned(),
            });
        }
        if let Some(view) = preview.settings_view() {
            if let Some(id) = self.view.selected_game().map(|entry| entry.id.clone()) {
                self.open_settings(&id, view);
            }
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

            tray_host: None,
            tray_commands: None,
            tray_handled: 0,
            remove_pending: None,
            pending: Vec::new(),
            handoff_popups: std::collections::VecDeque::new(),
            popup: None,
            last_refresh: None,
            detect_job: None,
            detect_queue: std::collections::VecDeque::new(),
            engine_dialog: None,
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

    fn preference_path(&self) -> Result<&std::path::Path, String> {
        self.preferences_path
            .as_deref()
            .ok_or_else(|| "本机偏好路径不可用。".to_owned())
    }

    fn preferences(&self) -> Result<Preferences, String> {
        launcher_core::preferences::load(self.preference_path()?)
            .map_err(|error| format!("读取本机偏好失败：{error}"))
    }

    fn update_preferences<T>(
        &self,
        edit: impl FnOnce(&mut Preferences) -> Result<T, String>,
    ) -> Result<T, String> {
        let path = self.preference_path()?;
        let mut preferences = self.preferences()?;
        let result = edit(&mut preferences)?;
        launcher_core::preferences::save(path, &preferences)
            .map_err(|error| format!("保存本机偏好失败：{error}"))?;
        Ok(result)
    }

    fn load_profile(&self, id: &str) -> Result<GameProfile, String> {
        self.preferences()?
            .find(id)
            .cloned()
            .ok_or_else(|| format!("本机偏好中找不到游戏档案：{id}"))
    }

    fn proxy_lock(&self, game: &str) -> Option<String> {
        if self
            .pending
            .iter()
            .any(|request| request.request.game == game)
        {
            return Some("该游戏正在处理启动或恢复请求，请等请求结束后再切换代理。".to_owned());
        }
        match launcher_core::journal_store::load_for_game(&self.root, game) {
            Ok(Some(_)) => Some("该游戏还有未清理的部署，请先恢复原状再切换代理。".to_owned()),
            Ok(None) => None,
            Err(error) => Some(format!("无法核验该游戏的部署记录，代理已锁定：{error}")),
        }
    }

    fn open_settings(&mut self, id: &str, view: SettingsView) {
        let profile = match self.load_profile(id) {
            Ok(profile) => profile,
            Err(error) => {
                self.view.notice = error;
                return;
            }
        };
        if view == SettingsView::Quality && EngineHost::confirmed(&profile.engine_host).is_none() {
            self.view.notice =
                "「画质」仅适用于已确认的 UE4/UE5。请通过「引擎」识别或校准。".to_owned();
            return;
        }
        self.settings = Some(SettingsForm::from_profile(
            view,
            &profile,
            self.proxy_lock(id),
        ));
    }

    fn save_settings(&mut self, form: &mut SettingsForm) -> bool {
        form.proxy_lock = self.proxy_lock(&form.game_id);
        let result = self.update_preferences(|preferences| {
            let mut profile = preferences
                .find(&form.game_id)
                .cloned()
                .ok_or_else(|| format!("无法保存，游戏档案已移除：{}", form.game_id))?;
            form.apply_to_profile(&mut profile, &self.root)?;
            preferences.upsert(profile);
            Ok(())
        });
        match result {
            Ok(()) => {
                form.error = None;
                form.dirty = false;
                self.view.notice = format!(
                    "「{}」的{}已保存",
                    self.game_name(&form.game_id),
                    form.view.title()
                );
                true
            }
            Err(error) => {
                self.view.notice = error.clone();
                form.error = Some(error);
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
                "  \"games\": {games},\n",
                "  \"first_frame_ms\": {first_frame},\n",
                "  \"tray\": {tray}",
                "}}\n"
            ),
            ppp = pixels_per_point,
            games = self.view.games.len(),
            first_frame = self.first_frame_ms.unwrap_or_default(),
            tray = self.tray_host.is_some()
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
                    self.view.notice = match launcher_win::open_path(&logs) {
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
        self.draw_engine_dialog(ui);
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
                    let engine_host = EngineHost::confirmed(&entry.engine_host);
                    let engine_busy_here =
                        engine_owner.as_ref().is_some_and(|(owner, _)| owner == &id);
                    let allow_stop = self.can_dispatch(Command::Stop, &id, engine_owner.as_ref());
                    let allow_run = self.can_dispatch(Command::Run, &id, engine_owner.as_ref());
                    let allow_restore =
                        self.can_dispatch(Command::Restore, &id, engine_owner.as_ref());
                    ui.horizontal(|ui| {
                        // 行内操作：启动/结束并恢复在最前，恢复原状只在有遗留部署时出现，移除在最后。
                        let running = self.view.row_running(index);
                        let can_cancel = engine_busy_here;
                        if running || can_cancel {
                            // 检测到游戏进程：按钮变成"结束并恢复"，即使游戏是手动启动的。
                            if ui
                                .add_enabled(
                                    allow_stop,
                                    egui::Button::new(
                                        egui::RichText::new(if running {
                                            "结束并恢复"
                                        } else {
                                            "中止并恢复"
                                        })
                                        .strong(),
                                    ),
                                )
                                .on_hover_text(if running {
                                    "结束已核验的游戏和官方启动器进程，然后恢复原状"
                                } else {
                                    "中止当前启动等待并恢复部署"
                                })
                                .on_disabled_hover_text("正在处理前一次引擎请求")
                                .clicked()
                            {
                                action = Some(RowAction::Stop(id.clone()));
                            }
                        } else {
                            let can_launch = entry.status.can_launch()
                                && !self.view.session_running()
                                && allow_run;
                            if ui
                                .add_enabled(can_launch, egui::Button::new("启动"))
                                .on_hover_text("部署资源并启动（二游会先拉起官方启动器）")
                                .on_disabled_hover_text(if !self.pending.is_empty() {
                                    "正在处理前一次引擎请求"
                                } else if self.view.session_running() {
                                    "已有游戏会话正在运行"
                                } else {
                                    entry.status.text()
                                })
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
                            .add_enabled(engine_host.is_some(), egui::Button::new("画质"))
                            .on_hover_text("画质档位与自定义 INI")
                            .on_disabled_hover_text(
                                "仅已确认的 UE4/UE5 可调画质，请用「引擎」识别或校准",
                            )
                            .clicked()
                        {
                            action = Some(RowAction::Quality(id.clone()));
                        }
                        if ui
                            .button("引擎")
                            .on_hover_text("识别引擎或手动校准虚幻主版本")
                            .clicked()
                        {
                            action = Some(RowAction::Engine(id.clone()));
                        }
                        status_pill(ui, entry.status);
                        if let Some(host) = engine_host {
                            ui.label(egui::RichText::new(host.as_text()).small().weak());
                        }
                        // 只在引擎和游戏都已退出时提供手动恢复。
                        if entry.status.can_restore()
                            && !running
                            && !can_cancel
                            && ui
                                .add_enabled(allow_restore, egui::Button::new("恢复原状"))
                                .on_disabled_hover_text("正在处理前一次引擎请求")
                                .on_hover_text("有未清理的部署：按记录恢复游戏目录与配置")
                                .clicked()
                        {
                            action = Some(RowAction::Restore(id.clone()));
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

    /// 先保存偏好，成功后再把条目加到界面。
    fn persist_profile(&mut self, profile: GameProfile) -> Result<(), String> {
        self.update_preferences(|preferences| {
            if preferences.find(&profile.id).is_some() {
                return Err(format!("「{}」已有游戏档案。", profile.name));
            }
            preferences.upsert(profile.clone());
            Ok(())
        })?;
        use launcher_core::ops::ProcessProbe;
        let processes = launcher_win::WinProcessProbe.list().unwrap_or_default();
        let journal = launcher_core::journal_store::load_for_game(&self.root, &profile.id)
            .ok()
            .flatten();
        let running = crate::engine::is_game_running(
            Some(&profile),
            journal.as_ref().map(|(_, journal)| journal),
            &processes,
        );
        let status = crate::view_model::derive_status(
            profile.directory.is_dir(),
            journal.is_some(),
            running,
        );
        self.view.upsert(crate::view_model::GameEntry {
            id: profile.id.clone(),
            name: profile.name.clone(),
            directory: profile.directory.clone(),
            status,
            engine_host: profile.engine_host.clone(),
            running,
        });
        self.view.select_by_id(&profile.id);
        if EngineHost::confirmed(&profile.engine_host).is_none() {
            if let Ok(target) = DetectionTarget::from_profile(&profile) {
                self.start_detection(target);
            }
        }
        Ok(())
    }
    /// 处理行内按钮：按动作分派到启动/结束并恢复/恢复/移除/配置。
    fn run_row_action(&mut self, action: RowAction) {
        match action {
            RowAction::Launch(id) => {
                self.view.select_by_id(&id);
                self.hand_off(Command::Run, &id);
            }
            RowAction::Stop(id) => {
                self.view.select_by_id(&id);
                self.hand_off(Command::Stop, &id);
            }
            RowAction::Restore(id) => {
                self.view.select_by_id(&id);
                self.hand_off(Command::Restore, &id);
            }
            RowAction::Remove(id) => self.remove_pending = Some(id),
            RowAction::FrameGen(id) => {
                self.view.select_by_id(&id);
                self.open_settings(&id, SettingsView::FrameGen);
            }
            RowAction::Quality(id) => {
                self.view.select_by_id(&id);
                self.open_settings(&id, SettingsView::Quality);
            }
            RowAction::Engine(id) => self.engine_dialog = Some(id),
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
            match self.remove_profile(&id) {
                Ok(()) => {
                    self.view.notice = format!("已移除 {name}");
                    self.remove_pending = None;
                }
                Err(error) => self.view.notice = error,
            }
        } else if !open || cancelled {
            self.remove_pending = None;
        }
    }

    fn remove_profile(&mut self, id: &str) -> Result<(), String> {
        self.update_preferences(|preferences| {
            preferences.remove(id);
            Ok(())
        })?;
        let selection = self.view.selected_game().map(|entry| entry.id.clone());
        self.view.games.retain(|entry| entry.id != id);
        self.view.selected = selection.and_then(|selected| {
            self.view
                .games
                .iter()
                .position(|entry| entry.id == selected)
        });
        if self
            .settings
            .as_ref()
            .is_some_and(|form| form.game_id == id)
        {
            self.settings = None;
        }
        if self.engine_dialog.as_deref() == Some(id) {
            self.engine_dialog = None;
        }
        self.detect_queue.retain(|target| target.id != id);
        if let Some(job) = self
            .detect_job
            .as_mut()
            .filter(|job| job.target.as_ref().is_some_and(|target| target.id == id))
        {
            job.target = None;
        }
        Ok(())
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
        let engine_busy = !self.pending.is_empty() || launcher_win::session_active();
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
        if let Err(error) = self.update_preferences(|preferences| {
            preferences.gpu = Some(launcher_core::preferences::GpuPreference {
                target_name: target,
            });
            Ok(())
        }) {
            self.view.notice = format!("显卡名称已切换，未能保存名称偏好：{error}");
        }
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
                self.view.notice = "已请求管理员权限…".to_owned();
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
                    self.view.notice = report.message.clone();
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
                    self.view.notice = report.error;
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
        if !self.pending.is_empty() || launcher_win::session_active() {
            self.view.notice = "引擎正在跑，先结束游戏并恢复再重启".to_owned();
            return;
        }
        match crate::gpu::restart(10) {
            Ok(message) => {
                self.view.notice = message;
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
            self.view.notice = "扫描已经在进行".to_owned();
            return;
        }
        let root = self.root.clone();
        let existing = self.existing_games();
        self.view.notice = "正在扫描平台记录…".to_owned();
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
                self.view.notice = "扫描线程异常结束".to_owned();
                return;
            }
        };
        let mut result = result;
        let candidates = std::mem::take(&mut result.ready);
        for ready in candidates {
            match self.persist_profile(ready.profile.clone()) {
                Ok(()) => result.ready.push(ready),
                Err(error) => result
                    .notes
                    .push(format!("「{}」添加失败：{error}", ready.profile.name)),
            }
        }
        self.view.notice = result.summary();
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
        crate::engine::render_path(&self.load_profile(id).ok()?)
    }

    fn start_detection(&mut self, target: DetectionTarget) {
        if self.detect_job.is_some() {
            if !self.detection_pending(&target.id) {
                self.detect_queue.push_back(target);
            }
            return;
        }
        let directory = target.directory.clone();
        let executable = target.executable.clone();
        match std::thread::Builder::new()
            .name("engine-detection".to_owned())
            .spawn(move || launcher_core::unreal::detect(&directory, &executable))
        {
            Ok(handle) => {
                self.detect_job = Some(DetectionJob {
                    target: Some(target),
                    handle,
                })
            }
            Err(error) => self.view.notice = format!("无法启动引擎识别：{error}"),
        }
    }

    fn detection_pending(&self, game: &str) -> bool {
        self.detect_job
            .as_ref()
            .and_then(|job| job.target.as_ref())
            .is_some_and(|target| target.id == game)
            || self.detect_queue.iter().any(|target| target.id == game)
    }

    fn poll_detection(&mut self) {
        if self
            .detect_job
            .as_ref()
            .is_some_and(|job| !job.handle.is_finished())
        {
            return;
        }
        if let Some(job) = self.detect_job.take() {
            if let Some(target) = job.target {
                match job.handle.join() {
                    Ok(detection) => self.apply_detection(&target, &detection),
                    Err(_) => {
                        self.view.notice =
                            format!("「{}」的引擎识别线程异常结束", self.game_name(&target.id))
                    }
                }
            }
        }
        if let Some(target) = self.detect_queue.pop_front() {
            self.start_detection(target);
        }
    }

    /// 只有唯一主版本才写入，且结果仍需匹配开始识别时的档案。
    fn apply_detection(
        &mut self,
        target: &DetectionTarget,
        detection: &launcher_core::unreal::Detection,
    ) {
        let host = match detection.major {
            Some(4) => EngineHost::Ue4,
            Some(5) => EngineHost::Ue5,
            _ => {
                self.view.notice = format!(
                    "「{}」引擎识别：{}（{}）。可在「引擎」里校准。",
                    self.game_name(&target.id),
                    detection.text(),
                    detection.source,
                );
                return;
            }
        };
        self.view.notice = match self.write_engine_host(&target.id, host, Some(target)) {
            Ok(()) => format!(
                "「{}」已识别为 {}（{}）",
                self.game_name(&target.id),
                host.as_text(),
                detection.source
            ),
            Err(error) => error,
        };
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
                self.view.notice = message;
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
        match self.update_preferences(|preferences| {
            let mut profile = preferences
                .find(id)
                .cloned()
                .ok_or_else(|| format!("本机偏好中找不到游戏档案：{id}"))?;
            profile.name = name.clone();
            preferences.upsert(profile);
            Ok(())
        }) {
            Ok(()) => {
                if let Some(entry) = self.view.games.iter_mut().find(|entry| entry.id == id) {
                    entry.name = name.clone();
                }
                self.view.notice = format!("已重命名为「{name}」");
            }
            Err(error) => {
                self.view.notice = format!("改名保存失败：{error}");
                self.rename = Some(crate::rename::RenameState {
                    fresh: true,
                    ..state
                });
            }
        }
    }

    /// Esc：放弃这一行的编辑，原名不动。
    fn cancel_rename(&mut self, id: &str) {
        if self.rename.as_ref().map(|state| state.id.as_str()) == Some(id) {
            self.rename = None;
        }
    }

    /// 先保存档案，再更新对应游戏行和打开的表单。
    fn write_engine_host(
        &mut self,
        id: &str,
        host: EngineHost,
        detection: Option<&DetectionTarget>,
    ) -> Result<(), String> {
        self.update_preferences(|preferences| {
            let mut profile = preferences
                .find(id)
                .cloned()
                .ok_or_else(|| format!("引擎档案已移除：{id}"))?;
            if detection.is_some_and(|target| !target.still_matches(&profile)) {
                return Err("识别期间游戏路径或引擎声明已变化，已丢弃这次识别。".to_owned());
            }
            profile.engine_host = host.as_text().to_owned();
            preferences.upsert(profile);
            Ok(())
        })?;
        if detection.is_none() {
            self.detect_queue.retain(|target| target.id != id);
            if let Some(job) = self.detect_job.as_mut() {
                if job.target.as_ref().is_some_and(|target| target.id == id) {
                    job.target = None;
                }
            }
        }
        if let Some(entry) = self.view.games.iter_mut().find(|entry| entry.id == id) {
            entry.engine_host = host.as_text().to_owned();
        }
        if let Some(form) = self.settings.as_mut().filter(|form| form.game_id == id) {
            form.engine_host = host.as_text().to_owned();
        }
        Ok(())
    }
    /// 引擎报告路径（引擎子进程写、界面读）。
    fn report_path(&self, name: &str) -> std::path::PathBuf {
        crate::about::logs_directory(&self.root).join(name)
    }

    /// 核对持有活动部署记录的引擎。状态文件只用于重新打开窗口后的会话提示。
    fn engine_owner(&self) -> Option<(String, u32)> {
        let owns_record = |game, pid| {
            launcher_core::journal_store::load_for_game(&self.root, game)
                .ok()
                .flatten()
                .is_some_and(|(_, record)| record.script_pid == pid)
        };
        for pending in &self.pending {
            if pending.request.command != Command::Run {
                continue;
            }
            if let Some(pid) = pending
                .child_pid()
                .filter(|pid| owns_record(&pending.request.game, *pid))
            {
                return Some((pending.request.game.clone(), pid));
            }
        }
        if !launcher_win::session_active() {
            return None;
        }
        let status = launcher_win::ipc::read_status(&self.report_path("status.json"))?;
        (launcher_win::process_running(status.pid) && owns_record(&status.game_id, status.pid))
            .then_some((status.game_id, status.pid))
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
        if let Some(form) = self
            .settings
            .as_ref()
            .filter(|form| form.view == SettingsView::FrameGen)
        {
            let lock = self.proxy_lock(&form.game_id);
            if let Some(form) = self.settings.as_mut() {
                form.proxy_lock = lock;
            }
        }
        self.view.session_taken = launcher_win::session_active();
        use launcher_core::ops::ProcessProbe;
        let processes = launcher_win::WinProcessProbe.list().unwrap_or_default();
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
                .flatten();
            let profile = profiles.iter().find(|profile| profile.id == id);
            let running = crate::engine::is_game_running(
                profile,
                pending.as_ref().map(|(_, journal)| journal),
                &processes,
            );
            let entry = &mut self.view.games[index];
            if let Some(profile) = profile {
                entry.engine_host.clone_from(&profile.engine_host);
            }
            entry.status =
                crate::view_model::derive_status(directory.is_dir(), pending.is_some(), running);
            entry.running = running;
        }
    }

    /// 只有当前游戏对应的引擎可以与一个中止请求并行。
    fn can_dispatch(&self, command: Command, game: &str, owner: Option<&(String, u32)>) -> bool {
        handoff::can_start(
            self.pending.iter().map(|pending| {
                let same_engine = pending.request.game == game
                    && owner
                        .is_some_and(|(id, pid)| id == game && pending.child_pid() == Some(*pid));
                (pending.request.command, same_engine)
            }),
            command,
        )
    }

    fn hand_off(&mut self, command: Command, game: &str) {
        let engine_owner = self.engine_owner();
        if !self.can_dispatch(command, game, engine_owner.as_ref()) {
            self.view.notice = "正在处理前一次引擎请求，请等它结束。".to_owned();
            return;
        }

        let request = Request::new(game, command);
        let logs = crate::about::logs_directory(&self.root);
        match Pending::start(request, &self.root, &logs) {
            Ok(pending) => {
                if command == Command::Stop {
                    for request in &mut self.pending {
                        request.keep_visible();
                    }
                }
                self.pending.push(pending);
                self.view.notice = format!(
                    "等待管理员授权以{}「{}」…",
                    command.label(),
                    self.game_name(game)
                );
            }
            Err(error) => self.view.notice = format!("无法发起{}请求：{error}", command.label()),
        }
    }

    fn game_name<'a>(&'a self, game: &'a str) -> &'a str {
        self.view
            .games
            .iter()
            .find(|entry| entry.id == game)
            .map_or(game, |entry| &entry.name)
    }

    fn poll_pending(&mut self) -> bool {
        let mut auto_park = false;
        let mut events = Vec::new();
        self.pending.retain_mut(|pending| {
            let Some(event) = pending.poll() else {
                return true;
            };
            let terminal = matches!(event, Event::Cancelled | Event::Finished(_));
            events.push((pending.request.clone(), event));
            !terminal
        });
        for (request, event) in events {
            let game = self.game_name(&request.game).to_owned();
            let action = request.command.label();
            match event {
                Event::ChildCreated => {
                    self.view.notice = format!("「{game}」的提权子进程已创建，正在等待{action}。");
                }
                Event::Accepted { auto_hide } => {
                    self.view.notice = format!("引擎已接管「{game}」。");
                    auto_park |= auto_hide;
                }
                Event::Overdue => {
                    let waiting_for = if request.command == Command::Run {
                        "接管确认"
                    } else {
                        "操作结果"
                    };
                    self.view.notice = format!("「{game}」等待{waiting_for}超时，子进程仍在运行。");
                }
                Event::OverdueWithReason(reason) => {
                    self.view.notice =
                        format!("无法核验「{game}」子进程状态：{reason}。仍跟踪本次请求。");
                }
                Event::Cancelled => self.view.notice = format!("已取消「{game}」的{action}授权。"),
                Event::Finished(outcome) => {
                    let (ok, title, detail) = match outcome {
                        Outcome::Completed { message } => {
                            let title = if request.command == Command::Run {
                                "启动会话结束".to_owned()
                            } else {
                                format!("{action}完成")
                            };
                            (true, title, message)
                        }
                        Outcome::SessionBusy { message } => {
                            (false, "引擎会话占用".to_owned(), message)
                        }
                        Outcome::Failed { message } => (false, format!("{action}失败"), message),
                        Outcome::Accepted => unreachable!("接管确认由独立事件处理"),
                    };
                    self.view.notice = format!("「{game}」{title}。{detail}");
                    self.last_refresh = None;
                    if !ok || request.command != Command::Run {
                        self.handoff_popups.push_back(Popup {
                            ok,
                            title: format!("「{game}」{title}"),
                            detail,
                        });
                    }
                }
            }
        }
        if self.popup.is_none() {
            self.popup = self.handoff_popups.pop_front();
        }
        auto_park
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
            let name = profile.name.clone();
            match self.persist_profile(profile) {
                Ok(()) => self.view.notice = format!("已添加「{name}」"),
                Err(error) => {
                    state.message = error.clone();
                    self.view.notice = error;
                    self.add_game = Some(state);
                }
            }
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
        let title = format!("{} · {}", form.view.title(), self.game_name(&form.game_id));
        egui::Window::new(title)
            .id(egui::Id::unique((form.view.title(), &form.game_id)))
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
                    let blocked = if form.view == SettingsView::Quality {
                        if !form.field_states().quality_choice {
                            Some("引擎尚未确认为 UE4/UE5。".to_owned())
                        } else {
                            form.quality_directory().err()
                        }
                    } else {
                        None
                    };
                    if ui
                        .add_enabled(blocked.is_none(), egui::Button::new("保存"))
                        .on_disabled_hover_text(blocked.as_deref().unwrap_or_default())
                        .clicked()
                    {
                        save = true;
                    }
                    if ui.button("校验").clicked() {
                        let _ = form.validate(&self.root);
                    }
                    if let Some(error) = form.error.as_deref() {
                        ui.colored_label(egui::Color32::from_rgb(0xD0, 0x4A, 0x4A), error);
                    }
                });
            });

        if open && (!save || !self.save_settings(&mut form)) {
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
        let resource_root = crate::engine::resource_root_of(&self.root);
        let missing = launcher_core::resources::missing_super_resolution_files(&resource_root);
        let sr_unavailable = (!missing.is_empty()).then(|| {
            let names: Vec<_> = missing
                .iter()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy())
                .collect();
            format!(
                "缺少超分资源：{}。补齐后可选超分档位，当前可用 Off。",
                names.join("、")
            )
        });
        if let Some(reason) = &sr_unavailable {
            ui.colored_label(egui::Color32::from_rgb(0xD0, 0x8A, 0x2E), reason);
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
                ui.label("SuperResolution.Mode")
                    .on_hover_text(crate::settings_form::key_hint("SuperResolution.Mode"));
                for (mode, text) in [
                    (SuperResolutionMode::Off, "Off"),
                    (SuperResolutionMode::MInt8, "M-Int8"),
                    (SuperResolutionMode::LInt8, "L-Int8"),
                    (SuperResolutionMode::MFp16, "M-Fp16"),
                    (SuperResolutionMode::LFp16, "L-Fp16"),
                ] {
                    let enabled = sr_unavailable.is_none() || mode == SuperResolutionMode::Off;
                    let response = ui.add_enabled_ui(enabled, |ui| {
                        ui.selectable_value(&mut form.settings.super_resolution, mode, text)
                    });
                    let response = match &sr_unavailable {
                        Some(reason) if !enabled => response.inner.on_disabled_hover_text(reason),
                        _ => response.inner,
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
                let game = self
                    .view
                    .games
                    .iter()
                    .find(|entry| entry.id == form.game_id);
                // 渲染目录里已有的普通文件 = 别人的注入器占着这个位置，本程序不覆盖。
                let occupied = game
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
                if let Some(reason) = &form.proxy_lock {
                    ui.colored_label(egui::Color32::from_rgb(0xD0, 0x8A, 0x2E), reason);
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

    /// 引擎识别与校准独立于画质入口，始终绑定打开该窗口的游戏。
    fn draw_engine_dialog(&mut self, root: &mut egui::Ui) {
        let Some(id) = self.engine_dialog.clone() else {
            return;
        };
        let Some(entry) = self.view.games.iter().find(|entry| entry.id == id) else {
            self.engine_dialog = None;
            return;
        };
        let name = entry.name.clone();
        let host = EngineHost::confirmed(&entry.engine_host);
        let busy = self.detection_pending(&id);
        let mut open = true;
        let mut detect = false;
        let mut manual = None;
        egui::Window::new(format!("引擎 · {name}"))
            .id(egui::Id::unique(("game-engine", &id)))
            .open(&mut open)
            .show(root.ctx(), |ui| {
                ui.label(format!(
                    "当前引擎：{}",
                    host.map_or("未确认", EngineHost::as_text)
                ));
                detect = ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(if busy {
                            "等待识别…"
                        } else {
                            "识别引擎"
                        }),
                    )
                    .on_hover_text("检查引擎版本文件和渲染 EXE，证据不足或冲突时保留原值")
                    .clicked();
                ui.label("仅在已确认游戏使用虚幻引擎时选择 UE4 或 UE5。");
                ui.horizontal(|ui| {
                    for (choice, label) in [
                        (EngineHost::Ue4, "UE4"),
                        (EngineHost::Ue5, "UE5"),
                        (EngineHost::None, "未确认"),
                    ] {
                        let selected = if choice == EngineHost::None {
                            host.is_none()
                        } else {
                            host == Some(choice)
                        };
                        if ui.selectable_label(selected, label).clicked() {
                            manual = Some(choice);
                        }
                    }
                });
            });
        if !open {
            self.engine_dialog = None;
        }
        if detect {
            match self
                .load_profile(&id)
                .and_then(|profile| DetectionTarget::from_profile(&profile))
            {
                Ok(target) => self.start_detection(target),
                Err(error) => self.view.notice = error,
            }
        }
        if let Some(host) = manual {
            self.view.notice = match self.write_engine_host(&id, host, None) {
                Ok(()) if host == EngineHost::None => format!("「{name}」的引擎已设为未确认"),
                Ok(()) => format!("「{name}」的引擎已校准为 {}", host.as_text()),
                Err(error) => error,
            };
        }
    }

    fn draw_quality(&mut self, ui: &mut egui::Ui, form: &mut SettingsForm) {
        let Some(host) = EngineHost::confirmed(&form.engine_host) else {
            ui.label("仅已确认的 UE4/UE5 支持画质调整。请在游戏行的「引擎」里校准。");
            return;
        };
        ui.label(format!("引擎版本：{}", host.as_text()));
        if !form.deploy_engine_ini {
            ui.label("该档案已关闭 Engine.ini 部署，画质设置无法应用。");
            return;
        }
        ui.horizontal(|ui| {
            ui.label("游戏配置目录");
            if ui
                .text_edit_singleline(&mut form.config_directory)
                .changed()
            {
                form.dirty = true;
            }
            if ui
                .button("选择目录…")
                .on_hover_text("请选择该游戏实际读取 Engine.ini 的配置目录")
                .clicked()
            {
                if let Some(directory) = rfd::FileDialog::new().pick_folder() {
                    form.config_directory = directory.display().to_string();
                    form.dirty = true;
                }
            }
        });
        if let Err(error) = form.quality_directory() {
            ui.colored_label(egui::Color32::from_rgb(0xD0, 0x8A, 0x2E), error);
        }
        ui.separator();
        ui.label("画质档位");
        for tier in SettingsForm::quality_options() {
            if ui
                .selectable_label(form.quality_tier == tier, tier.folder())
                .clicked()
            {
                form.set_quality(tier);
            }
        }
        if form.quality_tier == launcher_core::quality::QualityTier::Tier5 {
            ui.horizontal(|ui| {
                ui.label("自定义 INI");
                if ui.text_edit_singleline(&mut form.custom_ini_path).changed() {
                    form.dirty = true;
                }
                if ui.button("浏览…").clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("Engine.ini", &["ini"])
                        .pick_file()
                    {
                        form.custom_ini_path = file.display().to_string();
                        form.dirty = true;
                    }
                }
            });
        }
        ui.separator();
        ui.label(form.quality_summary(&self.root));
        ui.label("部署只读临时副本，游戏原件按字节备份并在退出后恢复。");
    }
}

impl LauncherApp {
    /// 首次进入事件循环时启动等待线程，由系统事件唤醒界面。
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
            ctx.request_repaint();
        });
    }

    fn show_main_window(&mut self, ctx: &egui::Context) {
        self.last_refresh = None;
        for request in &mut self.pending {
            request.keep_visible();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn park_blocker(&self) -> Option<&'static str> {
        match self.pending.as_slice() {
            [] => {}
            [request] if self.tray_host.is_some() && request.can_detach() => {}
            _ => return Some("请等待当前引擎操作完成。"),
        }
        if self.scan_job.is_some()
            || self.detect_job.is_some()
            || self.gpu.as_ref().is_some_and(|panel| panel.busy)
        {
            return Some("请等待正在进行的扫描、识别或显卡操作。");
        }
        if self.settings.as_ref().is_some_and(|form| form.dirty)
            || self.add_game.is_some()
            || self.rename.is_some()
        {
            return Some("请先保存或取消正在编辑的内容。");
        }
        if self.scan_report.is_some() || self.popup.is_some() || !self.handoff_popups.is_empty() {
            return Some("请先关闭当前结果窗口。");
        }
        None
    }

    fn exit_to_tray(&mut self, host: u32, quit: bool) {
        let mut output = std::io::stdout().lock();
        let write = |action, output: &mut _| {
            let event = GuiExit {
                handled: self.tray_handled,
                action,
            };
            crate::tray::write_message(output, &event).map_err(launcher_core::Error::io_plain)
        };
        let result = if quit {
            write(ExitAction::Quit, &mut output)
        } else if let Some(request) = self.pending.first() {
            request.detach_to(host, |engine| {
                write(
                    ExitAction::HandOff {
                        engine,
                        game_name: self.game_name(&request.request.game).to_owned(),
                    },
                    &mut output,
                )
            })
        } else {
            write(ExitAction::Park, &mut output)
        };
        match result {
            // 退出图形进程即释放其窗口、OpenGL 上下文与字体缓存。
            Ok(()) => std::process::exit(0),
            Err(error) => self.view.notice = format!("无法切换到托盘：{error}"),
        }
    }
}

impl eframe::App for LauncherApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.start_activation_waiter(ctx);
        if self
            .activate_pending
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.show_main_window(ctx);
        }
        if self.tray_host.is_some() && self.tray_commands.is_none() {
            let ctx = ctx.clone();
            self.tray_commands = Some(crate::tray::gui_commands(move || ctx.request_repaint()));
        }
        let mut quit = false;
        while let Some(commands) = self.tray_commands.as_ref() {
            let message = match commands.try_recv() {
                Ok(message) => message,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.tray_host = None;
                    self.tray_commands = None;
                    break;
                }
            };
            self.tray_handled = message.sequence;
            match message.command {
                GuiCommand::Show => self.show_main_window(ctx),
                GuiCommand::Quit => quit = true,
                GuiCommand::Failure { title, detail } => {
                    self.handoff_popups.push_back(Popup {
                        ok: false,
                        title,
                        detail,
                    });
                    self.show_main_window(ctx);
                }
            }
        }

        self.poll_scan();
        self.poll_gpu();
        self.poll_detection();
        if self.scan_job.is_some()
            || self.detect_job.is_some()
            || self.gpu.as_ref().is_some_and(|panel| panel.busy)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let auto_park = self.poll_pending();
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        let close = ctx.input(|input| input.viewport().close_requested());
        if quit || close {
            if let Some(reason) = self.park_blocker() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.view.notice = reason.to_owned();
                self.show_main_window(ctx);
            } else if let Some(host) = self.tray_host {
                // 写入失败时仍保留界面和原进程句柄。
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.exit_to_tray(host, quit);
            }
        } else if let Some(host) = self.tray_host
            && auto_park
            && self.park_blocker().is_none()
        {
            self.exit_to_tray(host, false);
        }

        if !ctx.input(|input| input.viewport().minimized == Some(true)) {
            self.refresh_rows();
            ctx.request_repaint_after(std::time::Duration::from_millis(REFRESH_INTERVAL_MS));
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw_scan_report(root);
        self.draw_remove_confirm(root);
        self.draw_popup(root);

        if self.first_frame_ms.is_none() {
            self.first_frame_ms = Some(self.started.elapsed().as_millis());
        }
        self.draw(root);
        if self.self_test.is_some() {
            // 即使窗口被遮挡或失焦，自检也需要再次调度。
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            self.write_report(root.ctx().pixels_per_point());
            if self
                .close_at
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
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
            .with_icon(crate::icon::app_icon())
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(case: &str) -> (Fixture, LauncherApp) {
        let root =
            std::env::temp_dir().join(format!("dlssg-settings-{}-{case}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = LauncherApp::new(LauncherView::new(), None)
            .with_root(root.clone())
            .with_preferences(Some(root.join("preferences.json")));
        for id in ["a", "b"] {
            let directory = root.join(id);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("Game.exe"), b"fixture").unwrap();
            let mut profile = GameProfile::new(id, id, directory);
            profile.engine_host = "UE4".to_owned();
            profile.render_executable = Some("Game.exe".to_owned());
            app.persist_profile(profile).unwrap();
        }
        (Fixture(root), app)
    }

    #[test]
    fn selection_and_other_games_detection_cannot_redirect_a_settings_form() {
        let (_root, mut app) = fixture("bindings");
        app.view.select_by_id("a");
        app.open_settings("a", SettingsView::FrameGen);
        app.view.select_by_id("b");

        let target = DetectionTarget::from_profile(&app.load_profile("b").unwrap()).unwrap();
        let detected = launcher_core::unreal::Detection {
            major: Some(5),
            conflict: false,
            source: "fixture".to_owned(),
            evidence: vec![],
        };
        app.apply_detection(&target, &detected);
        assert_eq!(app.load_profile("b").unwrap().engine_host, "UE5");
        let mut form = app.settings.take().unwrap();
        assert_eq!(form.game_id, "a");
        assert_eq!(form.engine_host, "UE4");
        form.set_proxy(2);
        assert!(app.save_settings(&mut form));
        assert_eq!(app.load_profile("a").unwrap().proxy_index, 2);
        assert_eq!(app.load_profile("b").unwrap().proxy_index, 1);
        assert_eq!(app.load_profile("b").unwrap().settings, None);

        app.write_engine_host("b", EngineHost::None, None).unwrap();
        app.apply_detection(&target, &detected);
        assert_eq!(app.load_profile("b").unwrap().engine_host, "none");
        app.open_settings("b", SettingsView::Quality);
        assert!(app.settings.is_none());
        app.write_engine_host("b", EngineHost::Ue5, None).unwrap();
        app.open_settings("b", SettingsView::Quality);
        assert_eq!(app.settings.as_ref().unwrap().game_id, "b");
    }

    #[test]
    fn proxy_lock_is_refreshed_and_save_rechecks_it() {
        use launcher_core::journal::Journal;
        use launcher_core::journal_store;

        let (root, mut app) = fixture("proxy");
        app.view.select_by_id("a");
        app.open_settings("a", SettingsView::FrameGen);
        app.settings.as_mut().unwrap().set_proxy(2);
        let mut journal =
            Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json")).unwrap();
        journal.game_id = "a".to_owned();
        let path = root.0.join(journal_store::state_relative_path("a"));
        journal_store::save_atomic(&path, &journal).unwrap();

        // 在 UI 自动刷新前直接点保存，也必须拒绝过期的可编辑状态。
        let mut form = app.settings.take().unwrap();
        assert!(form.field_states().proxy_choice);
        assert!(!app.save_settings(&mut form));
        assert_eq!(app.load_profile("a").unwrap().proxy_index, 1);
        assert!(form.dirty);
        app.settings = Some(form);
        app.view.select_by_id("b");
        app.refresh_rows();
        assert!(!app.settings.as_ref().unwrap().field_states().proxy_choice);

        std::fs::remove_file(path).unwrap();
        app.last_refresh = None;
        app.refresh_rows();
        assert!(app.settings.as_ref().unwrap().field_states().proxy_choice);
        let mut form = app.settings.take().unwrap();
        assert!(app.save_settings(&mut form));
        assert_eq!(app.load_profile("a").unwrap().proxy_index, 2);
    }

    #[test]
    fn mutation_failures_keep_disk_data_and_visible_rows() {
        let (root, mut app) = fixture("failed-write");
        let path = root.0.join("preferences.json");
        let old = std::fs::read(&path).unwrap();
        let temporary = path.with_extension("json.tmp");
        std::fs::create_dir(&temporary).unwrap();
        let other = GameProfile::new("c", "c", root.0.join("c"));
        assert!(app.persist_profile(other.clone()).is_err());
        assert!(app.remove_profile("a").is_err());
        assert!(app.write_engine_host("a", EngineHost::None, None).is_err());
        assert_eq!(app.view.games.len(), 2);
        assert_eq!(app.view.games[0].engine_host, "UE4");
        assert_eq!(std::fs::read(&path).unwrap(), old);
        std::fs::remove_dir(temporary).unwrap();

        app.view.select_by_id("a");
        app.open_settings("a", SettingsView::FrameGen);
        let mut form = app.settings.take().unwrap();
        form.set_proxy(2);
        let corrupt = b"{corrupt";
        std::fs::write(&path, corrupt).unwrap();
        assert!(app.persist_profile(other).is_err());
        assert!(app.remove_profile("a").is_err());
        assert!(!app.save_settings(&mut form));
        assert!(form.error.as_deref().unwrap().contains("读取本机偏好失败"));
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        assert_eq!(app.view.games.len(), 2);
    }
}
