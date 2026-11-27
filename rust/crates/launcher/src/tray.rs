//! 原生托盘进程。GUI 作为同一 EXE 的子进程按需打开。
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use launcher_win::activate;
use serde::{Deserialize, Serialize};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MB_ICONERROR, MSG, MessageBoxW, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, TranslateMessage, WM_APP,
};

use crate::handoff::{Detached, Outcome};
use crate::{EXIT_OK, Failure, ui::WINDOW_TITLE};

const TRAY_MUTEX: &str = r"Local\DlssgLauncherTray";
const TRAY_ACTIVATE: &str = r"Local\DlssgLauncherTrayActivate";

#[derive(Serialize, Deserialize)]
pub enum GuiCommand {
    Show,
    Quit,
    Failure { title: String, detail: String },
}

#[derive(Serialize, Deserialize)]
pub struct GuiMessage {
    pub sequence: u64,
    pub command: GuiCommand,
}

#[derive(Serialize, Deserialize)]
pub struct GuiExit {
    pub handled: u64,
    pub action: ExitAction,
}

#[derive(Serialize, Deserialize)]
pub enum ExitAction {
    Park,
    HandOff { engine: Detached, game_name: String },
    Quit,
}

enum Event {
    Show,
    Logs,
    Quit,
    GuiExited(io::Result<Output>),
    EngineFailed { title: String, detail: String },
}

#[derive(Clone)]
struct Wake {
    sender: Sender<Event>,
    thread: u32,
}

impl Wake {
    fn send(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            unsafe { PostThreadMessageW(self.thread, WM_APP, 0, 0) };
        }
    }
}

pub fn run() -> Result<u8, Failure> {
    let single = launcher_win::acquire_named_session(TRAY_MUTEX, Duration::ZERO)
        .map_err(|error| Failure::Usage(format!("托盘单实例不可用：{error}")))?;
    let Some(_single) = single else {
        activate::allow_activation();
        activate::signal_named(TRAY_ACTIVATE);
        return Ok(EXIT_OK);
    };
    // 兼容已在运行的旧版 GUI 或调试窗口。
    if activate::request_activation(WINDOW_TITLE) {
        return Ok(EXIT_OK);
    }

    let mut message = MSG::default();
    // PostThreadMessage 要求目标线程先建立消息队列。
    unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    let (sender, events) = mpsc::channel();
    let wake = Wake {
        sender,
        thread: unsafe { GetCurrentThreadId() },
    };
    let Some(_icon) = create_icon(&wake) else {
        return crate::run_gui(&[], None);
    };
    if let Ok(activation) = activate::ActivationEvent::create_named(TRAY_ACTIVATE) {
        let wake = wake.clone();
        activate::spawn_waiter(activation, move || wake.send(Event::Show));
    }
    let mut broker = Broker {
        wake,
        gui: None,
        sequence: 0,
        sent: Vec::new(),
    };
    broker.deliver(GuiCommand::Show);

    loop {
        let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if result < 0 {
            return Err(Failure::Usage(io::Error::last_os_error().to_string()));
        }
        if result == 0 {
            return Ok(EXIT_OK);
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        while let Ok(event) = events.try_recv() {
            match event {
                Event::Show => broker.deliver(GuiCommand::Show),
                Event::Logs => open_logs(),
                Event::Quit if broker.gui.is_none() => return Ok(EXIT_OK),
                Event::Quit => broker.deliver(GuiCommand::Quit),
                Event::GuiExited(output) => {
                    if broker.gui_exited(output) {
                        return Ok(EXIT_OK);
                    }
                }
                Event::EngineFailed { title, detail } => {
                    broker.deliver(GuiCommand::Failure { title, detail });
                }
            }
        }
    }
}

struct Broker {
    wake: Wake,
    gui: Option<ChildStdin>,
    sequence: u64,
    // 即使写入成功，子进程退出前也可能尚未消费消息。按退出回执重发。
    sent: Vec<GuiMessage>,
}

impl Broker {
    fn start_gui(&mut self) -> io::Result<()> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--tray-gui")
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        self.gui = child.stdin.take();
        let wake = self.wake.clone();
        std::thread::spawn(move || wake.send(Event::GuiExited(child.wait_with_output())));
        Ok(())
    }

    fn deliver(&mut self, command: GuiCommand) {
        if self.gui.is_none()
            && let Err(error) = self.start_gui()
        {
            let detail = match command {
                GuiCommand::Failure { title, detail } => {
                    format!("{title}\n{detail}\n\n无法打开启动器：{error}")
                }
                _ => format!("无法打开启动器：{error}"),
            };
            show_error(&detail);
            return;
        }
        match command {
            GuiCommand::Show => self
                .sent
                .retain(|message| !matches!(message.command, GuiCommand::Show)),
            GuiCommand::Quit => self
                .sent
                .retain(|message| !matches!(message.command, GuiCommand::Quit)),
            GuiCommand::Failure { .. } => {}
        }
        self.sequence += 1;
        let message = GuiMessage {
            sequence: self.sequence,
            command,
        };
        let _ = write_message(self.gui.as_mut().expect("子进程使用管道输入"), &message);
        self.sent.push(message);
    }

    fn gui_exited(&mut self, output: io::Result<Output>) -> bool {
        self.gui = None;
        let handled = match output {
            Ok(output) if output.status.success() => {
                match serde_json::from_slice::<GuiExit>(&output.stdout) {
                    Ok(exit) => {
                        match exit.action {
                            ExitAction::Quit => return true,
                            ExitAction::HandOff { engine, game_name } => {
                                self.watch_engine(engine, game_name)
                            }
                            ExitAction::Park => {}
                        }
                        Some(exit.handled)
                    }
                    Err(_) => {
                        // 旧 GUI 在极短的并发窗口里先拿到界面锁时，本次子进程可能只发激活信号。
                        if !activate::request_activation(WINDOW_TITLE) {
                            show_error("界面已退出，未收到托盘交接结果。可从托盘重新打开。");
                        }
                        None
                    }
                }
            }
            Ok(output) => {
                show_error(&format!(
                    "界面异常退出（{}）。可从托盘重新打开。",
                    output.status
                ));
                None
            }
            Err(error) => {
                show_error(&format!("无法读取界面退出结果：{error}"));
                None
            }
        };
        for message in std::mem::take(&mut self.sent) {
            if handled.is_some_and(|handled| message.sequence > handled) {
                if matches!(message.command, GuiCommand::Quit) && self.gui.is_none() {
                    return true;
                }
                self.deliver(message.command);
            }
        }
        false
    }

    fn watch_engine(&self, engine: Detached, game_name: String) {
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            // 只从本进程为直接 GUI 子进程创建的输出管道读取一次退出回执。
            if let Outcome::Failed { message } | Outcome::SessionBusy { message } =
                unsafe { engine.wait() }
            {
                wake.send(Event::EngineFailed {
                    title: format!("「{game_name}」启动失败"),
                    detail: message,
                });
            }
        });
    }
}

pub fn write_message(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

pub fn gui_commands(wake: impl Fn() + Send + 'static) -> mpsc::Receiver<GuiMessage> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if let Ok(command) = serde_json::from_str(&line) {
                if send.send(command).is_err() {
                    break;
                }
                wake();
            }
        }
        drop(send);
        wake();
    });
    receive
}

fn open_logs() {
    let Some(install) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_owned))
    else {
        return;
    };
    let root = launcher_core::prefs::preferences_path(&install)
        .ok()
        .and_then(|path| launcher_core::preferences::load(&path).ok())
        .and_then(|preferences| preferences.launcher_root)
        .filter(|root| !root.trim().is_empty())
        .map(Into::into)
        .unwrap_or(install);
    let logs = crate::about::logs_directory(&root);
    let _ = std::fs::create_dir_all(&logs);
    let _ = launcher_win::open_path(&logs);
}

fn show_error(text: &str) {
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let title: Vec<u16> = WINDOW_TITLE.encode_utf16().chain([0]).collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR,
        )
    };
}

fn create_icon(wake: &Wake) -> Option<TrayIcon> {
    let menu = Menu::new();
    let show = MenuItem::new("显示窗口", true, None);
    let logs = MenuItem::new("打开日志目录", true, None);
    let quit = MenuItem::new("退出", true, None);
    menu.append(&show).ok()?;
    menu.append(&logs).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;
    menu.append(&quit).ok()?;

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(WINDOW_TITLE)
        .with_icon(tray_icon::Icon::from_resource(1, None).ok()?)
        .build()
        .ok()?;
    let (show_id, logs_id, quit_id) = (show.id().clone(), logs.id().clone(), quit.id().clone());
    let menu_wake = wake.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            menu_wake.send(Event::Show);
        } else if event.id == logs_id {
            menu_wake.send(Event::Logs);
        } else if event.id == quit_id {
            menu_wake.send(Event::Quit);
        }
    }));
    let icon_wake = wake.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            icon_wake.send(Event::Show);
        }
    }));
    Some(icon)
}
