//! Phase 1 步 1（关卡 A 的起点）：骨架、错误类型、四个能力 trait、样本读取器。
//!
//! 这一步没有业务逻辑，验收标准是：
//! - `cargo test` 全绿；
//! - `cargo clippy -- -D warnings` 无告警；
//! - 样本目录能被稳定定位与逐字节读取。

use std::time::Duration;

use launcher_core::error::Error;
use launcher_core::fixtures;
use launcher_core::ops::{LinkKind, MutexLease, Platform};
use launcher_core::testing::{FakeMutexOps, FakeProcessProbe, FakeSymlinkOps, FixedClock};
use launcher_core::{SymlinkOps, VERSION};

#[test]
fn version_is_available_for_logs() {
    assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
}

#[test]
fn fixture_root_contains_the_frozen_samples() {
    assert!(fixtures::exists("manifest.json"));
    assert!(fixtures::exists("diagnostic/wuthering/default.ini"));
    assert!(fixtures::exists("journal/01-new.json"));
    assert!(fixtures::exists("paths/normalized-path.json"));
    assert!(fixtures::exists("templates/wuthering/0.3.2.ini"));
}

#[test]
fn fixture_bytes_are_verbatim() {
    // 0.4.2 鸣潮模板里 MaxGeneratedFrames 是 5（发行 INI 是 3），
    // 这个差异是 PARITY D-2 记录的行为变更点，样本必须如实保留。
    let text = fixtures::read_text("templates/wuthering/0.4.2.ini");
    assert!(text.contains("MaxGeneratedFrames=5"), "样本被改动过");
    let generated = fixtures::read_text("diagnostic/wuthering/default.ini");
    assert!(generated.contains("Mode=Off"));
    assert!(generated.contains("Optimized=2"));
    assert!(generated.contains("Level=2"));
}

#[test]
fn symlink_fake_reports_links_and_plain_paths() {
    let ops = FakeSymlinkOps::new();
    let link = std::path::Path::new(r"D:\game\version.dll");
    let target = std::path::Path::new(r"D:\launcher\version.dll");

    // 未登记 = 不是链接（普通文件或不存在）
    assert!(ops.inspect(link).expect("inspect").is_none());

    ops.create(link, target, false).expect("create");
    let info = ops.inspect(link).expect("inspect").expect("应有链接");
    assert_eq!(info.kind, LinkKind::SymbolicLink);
    assert_eq!(info.target, target);
    assert!(!info.target_is_dir);

    ops.remove(link).expect("remove");
    assert!(ops.inspect(link).expect("inspect").is_none());
    assert_eq!(ops.removed(), vec![link.to_path_buf()]);
}

#[test]
fn mutex_lease_releases_on_drop() {
    let ops = FakeMutexOps::new();
    {
        let lease = MutexLease::acquire(&ops, "Global\\DLSSG-Deployment-abc", Duration::ZERO)
            .expect("acquire");
        assert!(ops.is_held("Global\\DLSSG-Deployment-abc"));
        assert_eq!(lease.name(), "Global\\DLSSG-Deployment-abc");
    }
    assert!(!ops.is_held("Global\\DLSSG-Deployment-abc"));
    assert_eq!(ops.releases.borrow().len(), 1);
}

#[test]
fn mutex_timeout_is_a_distinct_error_kind() {
    let ops = FakeMutexOps::blocked_on("Global\\DLSSG-Config-abc");
    let error = MutexLease::acquire(
        &ops,
        "Global\\DLSSG-Config-abc",
        Duration::from_millis(5000),
    )
    .expect_err("应当超时");
    assert!(
        matches!(error, Error::MutexTimeout { .. }),
        "实际：{error:?}"
    );
    let acquisitions = ops.acquisitions.borrow();
    assert_eq!(acquisitions.len(), 1);
    assert_eq!(acquisitions[0].1, Duration::from_millis(5000));
}

#[test]
fn platform_bundles_all_four_capabilities() {
    let symlink = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let process = FakeProcessProbe::empty();
    let clock = FixedClock::default();
    let platform = Platform {
        symlink: &symlink,
        mutex: &mutex,
        process: &process,
        clock: &clock,
    };
    assert_eq!(platform.process.current().expect("current").pid, 4242);
    assert_eq!(
        platform.clock.now(),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(FixedClock::EPOCH_2026_11_22)
    );
    assert!(
        platform
            .symlink
            .inspect(std::path::Path::new("x"))
            .expect("inspect")
            .is_none()
    );
}
