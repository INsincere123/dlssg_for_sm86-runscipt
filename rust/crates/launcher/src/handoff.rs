//! 按请求跟踪界面与提权引擎的交接。
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use launcher_core::{Error, Result};
use launcher_win::Spawned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    Run,
    Restore,
    Stop,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "run" => Some(Self::Run),
            "restore" => Some(Self::Restore),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Restore => "restore",
            Self::Stop => "stop",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Run => "启动",
            Self::Restore => "恢复原状",
            Self::Stop => "结束并恢复",
        }
    }

    fn feedback_timeout(self) -> Duration {
        match self {
            Self::Run => Duration::from_secs(15),
            Self::Restore | Self::Stop => Duration::from_secs(180),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub game: String,
    pub command: Command,
}

impl Request {
    pub fn new(game: &str, command: Command) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let count = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seed = format!("{}-{nanos}-{count}", std::process::id());
        let id = launcher_core::path::sha256_upper(seed.as_bytes())[..32].to_owned();
        Self {
            id,
            game: game.to_owned(),
            command,
        }
    }

    pub fn report_path(&self, logs: &Path) -> PathBuf {
        logs.join(format!("ui-{}-{}.json", self.command.as_str(), self.id))
    }

    fn arguments(&self, root: &Path, report: &Path) -> String {
        format!(
            "--engine {} --game {} --root {} --report {} --ui-request {}",
            self.command.as_str(),
            quote_arg(&self.game),
            quote_arg(&root.to_string_lossy()),
            quote_arg(&report.to_string_lossy()),
            self.id,
        )
    }
}

fn quote_arg(arg: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in arg.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        let count = if character == '"' {
            slashes * 2 + 1
        } else {
            slashes
        };
        quoted.extend(std::iter::repeat_n('\\', count));
        quoted.push(character);
        slashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Outcome {
    Accepted,
    Completed { message: String },
    SessionBusy { message: String },
    Failed { message: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct Report {
    request: Request,
    pid: u32,
    outcome: Outcome,
}

fn accepted_path(report: &Path) -> PathBuf {
    report.with_extension("accepted.json")
}

pub fn publish(request: &Request, report: &Path, outcome: Outcome) -> Result<()> {
    let accepted;
    let path = if outcome == Outcome::Accepted {
        accepted = accepted_path(report);
        accepted.as_path()
    } else {
        report
    };
    let text = serde_json::to_vec_pretty(&Report {
        request: request.clone(),
        pid: std::process::id(),
        outcome,
    })
    .map_err(|error| Error::invalid(format!("交接报告无法序列化：{error}")))?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let temporary = path.with_extension("tmp");
    let result = std::fs::write(&temporary, &text).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error| Error::io(path, error))
}

fn read_matching(path: &Path, request: &Request, pid: u32) -> Option<Outcome> {
    let report: Report = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (report.request == *request && (pid == 0 || report.pid == pid)).then_some(report.outcome)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    ChildCreated,
    Accepted { auto_hide: bool },
    Overdue,
    OverdueWithReason(String),
    Cancelled,
    Finished(Outcome),
}

enum State {
    Elevating(Receiver<Result<Option<Spawned>>>),
    Running {
        child: Spawned,
        since: Instant,
        progress: Progress,
    },
}

#[derive(Default)]
struct Progress {
    accepted: bool,
    overdue: bool,
}

impl Progress {
    fn poll(
        &mut self,
        confirmed: bool,
        elapsed: Duration,
        timeout: Duration,
        auto_hide: bool,
    ) -> Option<Event> {
        if self.accepted {
            return None;
        }
        if confirmed {
            self.accepted = true;
            return Some(Event::Accepted {
                auto_hide: auto_hide && !self.overdue && elapsed < timeout,
            });
        }
        if !self.overdue && elapsed >= timeout {
            self.overdue = true;
            return Some(Event::Overdue);
        }
        None
    }
}

pub struct Pending {
    pub request: Request,
    report: PathBuf,
    state: State,
    auto_hide: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Detached {
    request: Request,
    report: PathBuf,
    pid: u32,
    handle: isize,
}

impl Detached {
    /// 从本进程创建的 GUI 子进程接收一次交接后，继续用同一个进程对象等待最终结果。
    ///
    /// # Safety
    /// 本对象必须来自本进程启动的 GUI，且其中的句柄已经复制到本进程。
    pub unsafe fn wait(self) -> Outcome {
        // 句柄来自父进程为此 GUI 建立的专用输出管道。
        let child = unsafe { Spawned::from_transferred_handle(self.handle) };
        if child.pid != self.pid {
            return Outcome::Failed {
                message: "托盘无法核对接管后的引擎进程。".into(),
            };
        }
        let Some(code) = child.wait(Duration::MAX) else {
            return Outcome::Failed {
                message: "托盘无法等到引擎退出。请检查会话日志。".into(),
            };
        };
        match read_finished(&self.request, &self.report, child.pid, code) {
            Event::Finished(outcome) => outcome,
            _ => unreachable!("最终报告只能产生最终结果"),
        }
    }
}

impl Pending {
    pub fn start(request: Request, root: &Path, logs: &Path) -> Result<Self> {
        let executable = std::env::current_exe().map_err(Error::io_plain)?;
        let report = request.report_path(logs);
        let arguments = request.arguments(root, &report);
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("engine-elevation".to_owned())
            .spawn(move || {
                let _ = sender.send(launcher_win::elevate::start_elevated(
                    &executable,
                    &arguments,
                ));
            })
            .map_err(Error::io_plain)?;
        Ok(Self {
            request,
            report,
            state: State::Elevating(receiver),
            auto_hide: true,
        })
    }

    pub fn child_pid(&self) -> Option<u32> {
        match &self.state {
            State::Running { child, .. } if child.pid != 0 => Some(child.pid),
            _ => None,
        }
    }

    pub fn keep_visible(&mut self) {
        self.auto_hide = false;
    }

    pub fn can_detach(&self) -> bool {
        self.request.command == Command::Run
            && matches!(&self.state, State::Running { progress, .. } if progress.accepted)
    }

    pub fn detach_to(&self, pid: u32, publish: impl FnOnce(Detached) -> Result<()>) -> Result<()> {
        let State::Running { child, .. } = &self.state else {
            return Err(Error::invalid("引擎尚未接管。"));
        };
        if !self.can_detach() {
            return Err(Error::invalid("当前引擎操作需要在界面中等待结果。"));
        }
        child.transfer_to(pid, |handle| {
            publish(Detached {
                request: self.request.clone(),
                report: self.report.clone(),
                pid: child.pid,
                handle,
            })
        })
    }

    pub fn poll(&mut self) -> Option<Event> {
        match &mut self.state {
            State::Elevating(receiver) => match receiver.try_recv() {
                Ok(Ok(Some(child))) => {
                    self.state = State::Running {
                        child,
                        since: Instant::now(),
                        progress: Progress::default(),
                    };
                    Some(Event::ChildCreated)
                }
                Ok(Ok(None)) => Some(Event::Cancelled),
                Ok(Err(error)) => Some(failed(format!("未能创建提权子进程：{error}"))),
                Err(TryRecvError::Disconnected) => Some(failed("提权工作线程异常结束。".into())),
                Err(TryRecvError::Empty) => None,
            },
            State::Running {
                child,
                since,
                progress,
            } => match child.try_wait() {
                Ok(Some(code)) => Some(read_finished(&self.request, &self.report, child.pid, code)),
                Ok(None) => {
                    let confirmed =
                        !progress.accepted && is_accepted(&self.request, &self.report, child.pid);
                    progress.poll(
                        confirmed,
                        since.elapsed(),
                        self.request.command.feedback_timeout(),
                        self.auto_hide,
                    )
                }
                Err(error) if !progress.overdue => {
                    progress.overdue = true;
                    // 句柄无法核验时保留原请求，防止重复启动或恢复。
                    self.auto_hide = false;
                    Some(Event::OverdueWithReason(error.to_string()))
                }
                Err(_) => None,
            },
        }
    }
}

fn is_accepted(request: &Request, report: &Path, pid: u32) -> bool {
    request.command == Command::Run
        && pid != 0
        && !report.exists()
        && read_matching(&accepted_path(report), request, pid) == Some(Outcome::Accepted)
}

fn read_finished(request: &Request, report: &Path, pid: u32, code: u32) -> Event {
    let outcome = read_matching(report, request, pid).filter(|outcome| match outcome {
        Outcome::Completed { .. } => code == 0,
        Outcome::SessionBusy { .. } => code == u32::from(crate::EXIT_SESSION_BUSY),
        Outcome::Failed { .. } => code != 0,
        Outcome::Accepted => false,
    });
    match outcome {
        Some(outcome) => Event::Finished(outcome),
        None => failed(format!(
            "引擎已退出（退出码 {code}），未收到本次请求的有效结果。"
        )),
    }
}

fn failed(message: String) -> Event {
    Event::Finished(Outcome::Failed { message })
}

pub fn can_start(active: impl IntoIterator<Item = (Command, bool)>, command: Command) -> bool {
    // bool 表示一个启动请求与当前游戏及其实际活跃子进程一致。
    active.into_iter().all(|(pending, same_engine)| {
        command == Command::Stop && pending == Command::Run && same_engine
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_carry_the_identity_and_preserve_root_backslash() {
        let request = Request::new("direct-896a", Command::Run);
        let root = Path::new(r"D:\");
        let report = request.report_path(Path::new(r"D:\some logs"));
        let arguments = request.arguments(root, &report);
        assert!(
            arguments.contains(r#"--engine run --game "direct-896a" --root "D:\\""#),
            "{arguments}"
        );
        assert!(arguments.ends_with(&format!("--ui-request {}", request.id)));
        assert_ne!(
            report,
            Request::new("direct-896a", Command::Run).report_path(Path::new(r"D:\some logs"))
        );
    }

    #[test]
    fn busy_requests_only_allow_stopping_the_same_active_run_once() {
        assert!(can_start([], Command::Run));
        assert!(!can_start([(Command::Run, true)], Command::Run));
        assert!(!can_start([(Command::Run, true)], Command::Restore));
        assert!(!can_start([(Command::Run, false)], Command::Stop));
        assert!(can_start([(Command::Run, true)], Command::Stop));
        assert!(!can_start(
            [(Command::Run, true), (Command::Stop, true)],
            Command::Stop
        ));
    }

    #[test]
    fn rejects_stale_identity_and_accepts_only_the_bound_child_once() {
        let request = Request::new("nte", Command::Run);
        let logs = std::env::temp_dir().join(format!("dlssg-handoff-{}", request.id));
        let report = request.report_path(&logs);
        let mut stale = request.clone();
        stale.id = Request::new("nte", Command::Run).id;
        publish(&stale, &report, Outcome::Accepted).unwrap();
        let pid = std::process::id();
        assert!(!is_accepted(&request, &report, pid));
        publish(&request, &report, Outcome::Accepted).unwrap();
        assert!(!is_accepted(&request, &report, pid + 1));
        assert!(is_accepted(&request, &report, pid));
        let busy = Outcome::SessionBusy {
            message: "已占用".into(),
        };
        publish(&request, &report, busy.clone()).unwrap();
        assert!(!is_accepted(&request, &report, pid));
        assert_eq!(
            read_finished(&request, &report, pid, u32::from(crate::EXIT_SESSION_BUSY)),
            Event::Finished(busy)
        );
        assert!(matches!(
            read_finished(&stale, &report, pid, 0),
            Event::Finished(Outcome::Failed { .. })
        ));
        let _ = std::fs::remove_file(&report);
        let _ = std::fs::remove_file(accepted_path(&report));
        let _ = std::fs::remove_dir(logs);
    }

    #[test]
    fn slow_or_late_acknowledgement_does_not_hide_the_window() {
        let timeout = Command::Run.feedback_timeout();
        let mut progress = Progress::default();
        assert_eq!(
            progress.poll(false, Duration::from_secs(1), timeout, true),
            None
        );
        assert_eq!(
            progress.poll(false, timeout, timeout, true),
            Some(Event::Overdue)
        );
        assert_eq!(
            progress.poll(true, timeout, timeout, true),
            Some(Event::Accepted { auto_hide: false })
        );
        assert_eq!(progress.poll(true, timeout, timeout, true), None);

        let mut progress = Progress::default();
        assert_eq!(
            progress.poll(true, Duration::from_secs(1), timeout, true),
            Some(Event::Accepted { auto_hide: true })
        );
        assert_eq!(progress.poll(true, timeout, timeout, true), None);

        let mut progress = Progress::default();
        assert_eq!(
            progress.poll(true, Duration::from_secs(1), timeout, false),
            Some(Event::Accepted { auto_hide: false })
        );
    }

    #[test]
    fn cancelled_elevation_does_not_create_an_engine_session() {
        let (send, receive) = mpsc::sync_channel(1);
        let mut pending = Pending {
            request: Request::new("nte", Command::Run),
            report: PathBuf::new(),
            state: State::Elevating(receive),
            auto_hide: true,
        };
        assert_eq!(pending.poll(), None);
        send.send(Ok(None)).unwrap();
        assert_eq!(pending.poll(), Some(Event::Cancelled));
        assert_eq!(pending.child_pid(), None);
    }
}
