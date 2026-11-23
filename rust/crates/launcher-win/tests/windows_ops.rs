//! `launcher-win` 的真实 Win32 行为验证（需要能创建符号链接；M2-01 验收）。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use launcher_core::Error;
use launcher_core::ops::{Clock, LinkKind, MutexOps, ProcessProbe, SymlinkOps};
use launcher_win::{SystemClock, WinMutexOps, WinProcessProbe, WinSymlinkOps};

struct Scratch {
    root: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn scratch(name: &str) -> Scratch {
    let root = std::env::temp_dir().join(format!("dlssg-win-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("临时目录");
    Scratch { root }
}

fn can_create_links(scratch: &Scratch) -> bool {
    let target = scratch.root.join("probe.txt");
    let link = scratch.root.join("probe.lnk");
    if std::fs::write(&target, "x").is_err() {
        return false;
    }
    let ops = WinSymlinkOps;
    match ops.create(&link, &target, false) {
        Ok(()) => {
            let _ = ops.remove(&link);
            true
        }
        Err(_) => false,
    }
}

#[test]
fn file_link_round_trip_keeps_the_target() {
    let scratch = scratch("file");
    if !can_create_links(&scratch) {
        eprintln!("跳过：当前环境不能创建符号链接");
        return;
    }
    let ops = WinSymlinkOps;
    let target = scratch.root.join("target.dll");
    let link = scratch.root.join("version.dll");
    std::fs::write(&target, "payload").expect("目标文件");

    ops.create(&link, &target, false).expect("建文件链接");
    let info = ops.inspect(&link).expect("查询").expect("应有链接");
    assert_eq!(info.kind, LinkKind::SymbolicLink);
    assert!(!info.target_is_dir, "文件链接的目标不是目录");
    assert_eq!(
        info.target.file_name().and_then(|name| name.to_str()),
        Some("target.dll")
    );

    ops.remove(&link).expect("删链接");
    assert!(ops.inspect(&link).expect("查询").is_none());
    assert!(target.is_file(), "目标必须还在");
}

#[test]
fn directory_link_uses_remove_directory() {
    let scratch = scratch("dir");
    if !can_create_links(&scratch) {
        eprintln!("跳过：当前环境不能创建符号链接");
        return;
    }
    let ops = WinSymlinkOps;
    let target = scratch.root.join("runtime");
    let link = scratch.root.join("dlssg_sm86").join("runtime");
    std::fs::create_dir_all(&target).expect("目标目录");
    std::fs::write(target.join("nvngx_dlssg.dll"), "dll").expect("目标内文件");
    std::fs::create_dir_all(link.parent().expect("父目录")).expect("父目录");

    ops.create(&link, &target, true).expect("建目录链接");
    let info = ops.inspect(&link).expect("查询").expect("应有链接");
    assert_eq!(info.kind, LinkKind::SymbolicLink);
    assert!(info.target_is_dir, "目录链接的目标必须是目录");

    ops.remove(&link).expect("删目录链接");
    assert!(ops.inspect(&link).expect("查询").is_none());
    assert!(target.join("nvngx_dlssg.dll").is_file(), "目标内容必须保留");
}

#[test]
fn junction_is_reported_as_junction_not_symbolic_link() {
    let scratch = scratch("junction");
    let ops = WinSymlinkOps;
    let target = scratch.root.join("junction-target");
    let link = scratch.root.join("junction-link");
    std::fs::create_dir_all(&target).expect("目标目录");
    let status = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let created = matches!(status, Ok(value) if value.success()) && link.exists();
    if !created {
        eprintln!("跳过：无法创建 junction（需要管理员或开发者模式）");
        return;
    }
    let info = ops.inspect(&link).expect("查询").expect("应有重解析点");
    assert_eq!(info.kind, LinkKind::Junction, "junction 不能被当成符号链接");
    ops.remove(&link).expect("删除 junction 本身");
    assert!(target.is_dir(), "junction 目标必须保留");
}

#[test]
fn mutex_blocks_the_second_acquire_and_frees_after_release() {
    let ops = WinMutexOps;
    let name = format!("Global\\DlssgTest-{}", std::process::id());
    let handle = ops.acquire(&name, Duration::ZERO).expect("首次获取");
    // Win32 具名互斥是**线程所有且可递归**的：同线程再获取会成功，
    // 只有另一个线程才会被挡住。PS 的 Mutex 语义相同。
    let other_name = name.clone();
    let other = std::thread::spawn(move || {
        let ops = WinMutexOps;
        ops.acquire(&other_name, Duration::from_millis(100))
    });
    let result = other.join().expect("线程");
    assert!(
        matches!(result, Err(Error::MutexTimeout { .. })),
        "另一个线程应当超时，实际：{result:?}"
    );
    ops.release(handle).expect("释放");
    let again = ops
        .acquire(&name, Duration::ZERO)
        .expect("释放后应能再获取");
    ops.release(again).expect("再次释放");
}

#[test]
fn process_probe_sees_the_current_process() {
    let probe = WinProcessProbe;
    let current = probe.current().expect("当前进程");
    let entries = probe.list().expect("枚举");
    assert!(
        entries.iter().any(|entry| entry.pid == current.pid),
        "枚举结果应含当前进程"
    );
    assert!(
        !current.name.to_ascii_lowercase().ends_with(".exe"),
        "名字应去掉 .exe"
    );
    let started = probe.started_at(current.pid).expect("启动时间");
    assert!(started <= SystemClock.now(), "启动时间不应在未来");
}

#[test]
fn clock_is_close_to_the_system_clock() {
    let clock = SystemClock;
    let now = clock.now();
    let elapsed = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("时间在 1970 之后");
    assert!(elapsed.as_secs() > 1_700_000_000, "系统时间明显不对");
    let _ = Path::new("x");
}

#[test]
fn spawn_process_runs_a_hidden_child_and_reports_its_exit_code() {
    use launcher_win::spawn_process;

    // cmd.exe /c exit 7：不需要管理员，不写文件，退出码可预测。
    let child = spawn_process(
        &PathBuf::from(r"C:\Windows\System32\cmd.exe"),
        &["/c".to_owned(), "exit".to_owned(), "7".to_owned()],
        Some(&std::env::temp_dir()),
        true,
    )
    .expect("启动子进程");
    assert!(child.pid > 0, "应拿到 pid");
    let code = child.wait(Duration::from_secs(20)).expect("应正常退出");
    assert_eq!(code, 7, "退出码必须原样传回");
}

#[test]
fn spawn_process_rejects_a_missing_executable() {
    use launcher_win::spawn_process;

    let error = spawn_process(
        &PathBuf::from(r"D:\definitely-not-here-dlssg.exe"),
        &[],
        None,
        true,
    )
    .expect_err("应报错");
    assert!(
        error.to_string().contains("文件操作失败") || error.to_string().contains("错误"),
        "{error}"
    );
}
