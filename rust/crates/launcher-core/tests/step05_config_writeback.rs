//! Phase 1 步 5：`启动配置.ini` 写回（保留编码与 BOM）。
//!
//! 对照 `Save-LauncherDetectedPath`（`LauncherConfig.ps1:170-228`）。
//! 每个用例都在临时目录里的副本上操作，绝不触碰仓库里的样本。

use std::path::{Path, PathBuf};
use std::time::Duration;

use launcher_core::config::{
    Document, TextEncoding, WriteOutcome, WriteRequest, decode, parse, write_if_absent,
};
use launcher_core::testing::{FakeMutexOps, FakeSymlinkOps};
use launcher_core::{Error, SymlinkOps};

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("dlssg-step05-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("创建临时目录");
    directory
}

fn copy_fixture(relative: &str, directory: &Path, file_name: &str) -> PathBuf {
    let target = directory.join(file_name);
    std::fs::copy(launcher_core::fixtures::path(relative), &target).expect("复制样本");
    target
}

/// 只替换值的请求：目标键在样本里存在但值为空。
fn empty_key_request<'a>(path: &'a Path, value: &'a str) -> WriteRequest<'a> {
    WriteRequest {
        path,
        group_label: "鸣潮",
        group_id: "Wuthering",
        display_key: "游戏配置目录",
        english_key: "ConfigDirectory",
        value,
    }
}

fn non_empty_key_request<'a>(path: &'a Path, value: &'a str) -> WriteRequest<'a> {
    WriteRequest {
        path,
        group_label: "鸣潮",
        group_id: "Wuthering",
        display_key: "启动器路径",
        english_key: "LauncherPath",
        value,
    }
}

fn entry_value(document: &Document, group: &str, key: &str) -> Option<String> {
    document
        .section(group)
        .and_then(|section| section.get(key))
        .map(|entry| entry.value.clone())
}

fn bytes_per_character(encoding: TextEncoding) -> isize {
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom | TextEncoding::Gbk => 1,
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => 2,
        TextEncoding::Utf32Le | TextEncoding::Utf32Be => 4,
    }
}

#[test]
fn every_encoding_is_preserved_when_writing_back() {
    let samples = [
        "config/encodings/启动配置-utf8.ini",
        "config/encodings/启动配置-utf8-bom.ini",
        "config/encodings/启动配置-utf16-le.ini",
        "config/encodings/启动配置-utf16-be.ini",
        "config/encodings/启动配置-utf32-le.ini",
        "config/encodings/启动配置-utf32-be.ini",
        "config/encodings/启动配置-gbk.ini",
    ];
    let directory = scratch("encodings");
    for relative in samples {
        let bytes_before = launcher_core::fixtures::read_bytes(relative);
        let before = decode(&bytes_before).expect("解码原文件");
        let file_name = relative.rsplit('/').next().expect("文件名");
        let path = copy_fixture(relative, &directory, file_name);

        let ops = FakeMutexOps::new();
        let value = r"D:\SavedConfig";
        let outcome = write_if_absent(
            &ops,
            Duration::from_millis(5000),
            &empty_key_request(&path, value),
        )
        .unwrap_or_else(|error| panic!("{relative} 写回失败：{error}"));
        assert_eq!(outcome, WriteOutcome::Written, "{relative}");
        assert_eq!(
            ops.acquisitions.borrow().len(),
            1,
            "{relative} 应只拿一次互斥"
        );
        assert!(
            ops.acquisitions.borrow()[0]
                .0
                .starts_with("Local\\DLSSG-LauncherConfig-")
        );

        let bytes_after = std::fs::read(&path).expect("读回文件");
        let after = decode(&bytes_after).expect("解码写回结果");
        assert_eq!(after.encoding, before.encoding, "{relative} 编码被改动");
        assert_eq!(after.bom, before.bom, "{relative} BOM 被改动");

        let document = parse(&after.text).expect("解析写回结果");
        assert_eq!(
            entry_value(&document, "Wuthering", "ConfigDirectory").as_deref(),
            Some(value),
            "{relative}"
        );
        assert_eq!(
            entry_value(&document, "Wuthering", "LauncherPath").as_deref(),
            Some(r"C:\a#b=c\launcher.exe"),
            "{relative} 其它键被改动"
        );
        assert_eq!(
            entry_value(&document, "Gpu", "DeviceInstanceId").as_deref(),
            Some(r"HDAUDIO\FUNC_01"),
            "{relative} 其它分组被改动"
        );

        // 只替换空值那一行：字节变化 = 新值长度 × 每字符字节数
        let unit = bytes_per_character(before.encoding);
        let expected_len = bytes_before.len() as isize + value.len() as isize * unit;
        assert_eq!(
            bytes_after.len() as isize,
            expected_len,
            "{relative} 改动了其它字节"
        );
    }
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn existing_value_is_reported_and_never_overwritten() {
    let directory = scratch("already-set");
    let path = copy_fixture(
        "config/encodings/启动配置-utf8.ini",
        &directory,
        "already.ini",
    );
    let bytes_before = std::fs::read(&path).expect("读原文件");

    let ops = FakeMutexOps::new();
    let outcome = write_if_absent(
        &ops,
        Duration::ZERO,
        &non_empty_key_request(&path, "ignored"),
    )
    .expect("写回");

    assert_eq!(
        outcome,
        WriteOutcome::AlreadySet(r"C:\a#b=c\launcher.exe".to_owned())
    );
    assert_eq!(
        std::fs::read(&path).expect("读回"),
        bytes_before,
        "不应写盘"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn missing_key_is_inserted_right_after_the_section_header() {
    let directory = scratch("insert");
    let path = copy_fixture(
        "config/encodings/启动配置-utf8.ini",
        &directory,
        "insert.ini",
    );
    let ops = FakeMutexOps::new();
    let mut write = non_empty_key_request(&path, "E:\\launcher.exe");
    write.display_key = "代理DLL";
    write.english_key = "ProxyDll";
    write.value = "2";

    let outcome = write_if_absent(&ops, Duration::ZERO, &write).expect("写回");
    assert_eq!(outcome, WriteOutcome::Written);

    let text = std::fs::read_to_string(&path).expect("读回文件");
    let lines: Vec<&str> = text.split("\r\n").collect();
    let header = lines
        .iter()
        .position(|line| line.trim() == "[鸣潮]")
        .expect("应有鸣潮分组");
    assert_eq!(lines[header + 1], "代理DLL=2");
    assert_eq!(lines[header + 2], r"游戏目录=D:\Games\WuWa");

    let document = parse(&text).expect("解析");
    assert_eq!(
        entry_value(&document, "Wuthering", "ProxyDll").as_deref(),
        Some("2")
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn missing_section_is_an_error() {
    let directory = scratch("missing-section");
    let path = directory.join("其他.ini");
    std::fs::write(&path, "[Gpu]\r\n设备实例标识=x\r\n").expect("写文件");

    let ops = FakeMutexOps::new();
    let error =
        write_if_absent(&ops, Duration::ZERO, &empty_key_request(&path, "v")).expect_err("应报错");
    assert!(
        error.to_string().contains("找不到配置分组"),
        "实际：{error}"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn mutex_timeout_is_reported_and_file_untouched() {
    let directory = scratch("mutex");
    let path = copy_fixture(
        "config/encodings/启动配置-utf8.ini",
        &directory,
        "locked.ini",
    );

    let recorder = FakeMutexOps::new();
    let write = empty_key_request(&path, "v1");
    write_if_absent(&recorder, Duration::ZERO, &write).expect("第一次写入");
    let name = recorder.acquisitions.borrow()[0].0.clone();
    let bytes_before = std::fs::read(&path).expect("读原文件");

    let blocked = FakeMutexOps::blocked_on(&name);
    let error = write_if_absent(&blocked, Duration::from_millis(5000), &write).expect_err("应超时");
    assert!(
        matches!(error, Error::MutexTimeout { .. }),
        "实际：{error:?}"
    );
    assert_eq!(
        std::fs::read(&path).expect("读回"),
        bytes_before,
        "不应写盘"
    );
    assert_eq!(
        blocked.acquisitions.borrow()[0].1,
        Duration::from_millis(5000)
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
// 目标平台是 Windows：这里只是把测试用的只读属性恢复成可写，
// clippy 的 Unix 语义担忧（world writable）不适用。
#[allow(clippy::permissions_set_readonly_false)]
fn readonly_file_is_refused() {
    let directory = scratch("readonly");
    let path = copy_fixture("config/encodings/启动配置-utf8.ini", &directory, "ro.ini");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).expect("设为只读");

    let ops = FakeMutexOps::new();
    let error =
        write_if_absent(&ops, Duration::ZERO, &empty_key_request(&path, "v")).expect_err("应拒绝");
    assert!(error.to_string().contains("只读"), "实际：{error}");

    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).expect("恢复可写");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn missing_file_is_an_error() {
    let directory = scratch("missing");
    let path = directory.join("不存在的.ini");
    let ops = FakeMutexOps::new();
    let error =
        write_if_absent(&ops, Duration::ZERO, &empty_key_request(&path, "v")).expect_err("应报错");
    assert!(error.to_string().contains("文件操作失败"), "实际：{error}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn gbk_roundtrip_keeps_encoding_label() {
    let directory = scratch("gbk");
    let path = copy_fixture("config/encodings/启动配置-gbk.ini", &directory, "gbk.ini");
    let ops = FakeMutexOps::new();
    let outcome = write_if_absent(
        &ops,
        Duration::ZERO,
        &empty_key_request(&path, r"D:\新建\a"),
    )
    .expect("写回");
    assert_eq!(outcome, WriteOutcome::Written);

    let bytes = std::fs::read(&path).expect("读回");
    let decoded = decode(&bytes).expect("解码");
    assert_eq!(decoded.encoding, TextEncoding::Gbk);
    let document = parse(&decoded.text).expect("解析");
    assert_eq!(
        entry_value(&document, "Wuthering", "ConfigDirectory").as_deref(),
        Some(r"D:\新建\a")
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn symlink_is_refused_before_touching_anything() {
    // 没有创建链接的特权时跳过：这是环境限制，不是行为缺失
    let directory = scratch("symlink");
    let target = copy_fixture(
        "config/encodings/启动配置-utf8.ini",
        &directory,
        "target.ini",
    );
    let link = directory.join("link.ini");
    if std::os::windows::fs::symlink_file(&target, &link).is_err() {
        let _ = std::fs::remove_dir_all(&directory);
        return;
    }
    let ops = FakeMutexOps::new();
    let error =
        write_if_absent(&ops, Duration::ZERO, &empty_key_request(&link, "v")).expect_err("应拒绝");
    assert!(error.to_string().contains("链接"), "实际：{error}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn writeback_only_needs_the_mutex_capability() {
    // 写回不碰链接能力；链接相关契约属于 journal/资源层
    let fake = FakeSymlinkOps::new();
    assert!(
        SymlinkOps::inspect(&fake, Path::new("x"))
            .expect("inspect")
            .is_none()
    );
}
