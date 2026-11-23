//! M3-03：渲染程序扫描（深度/数量上限、跳过规则、x64 判定、打分、可信阈值）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use launcher_core::discovery::{
    DiscoveryLimits, is_auxiliary_executable, is_x64_executable, renderer_candidates,
    trusted_renderer,
};

struct Tree {
    root: PathBuf,
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("dlssg-disc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("根目录");
        Self { root }
    }

    fn dir(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        std::fs::create_dir_all(&path).expect("目录");
        path
    }

    fn file(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("父目录");
        }
        std::fs::write(&path, "x").expect("文件");
        path
    }

    /// 写一个最小 x64 PE（可选 DLL 位）。
    fn pe(&self, relative: &str, x64: bool, dll: bool) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("父目录");
        }
        let mut data = vec![0u8; 512];
        data[0] = b'M';
        data[1] = b'Z';
        let offset: usize = 128;
        data[60..64].copy_from_slice(&(offset as i32).to_le_bytes());
        data[offset..offset + 4].copy_from_slice(b"PE\0\0");
        let machine: u16 = if x64 { 0x8664 } else { 0x014c };
        data[offset + 4..offset + 6].copy_from_slice(&machine.to_le_bytes());
        let mut flags: u16 = 0x0002;
        if dll {
            flags |= 0x2000;
        }
        data[offset + 22..offset + 24].copy_from_slice(&flags.to_le_bytes());
        std::fs::write(&path, data).expect("PE");
        path
    }
}

fn never() -> bool {
    false
}

fn scan(root: &Path) -> launcher_core::discovery::DiscoveryOutcome {
    renderer_candidates(root, None, DiscoveryLimits::default(), &never).expect("扫描")
}

#[test]
fn ue_game_with_frame_generation_scores_highest() {
    let tree = Tree::new("ue");
    tree.pe("Game/Binaries/Win64/Game-Win64-Shipping.exe", true, false);
    tree.file("Game/Binaries/Win64/nvngx_dlssg.dll");
    tree.file("Game/Binaries/Win64/nvngx_dlss.dll");
    tree.pe("Game/Binaries/Win64/SomeHelper.exe", true, false);
    tree.pe("Game/Binaries/Win64/Other-Win64-Shipping.exe", true, false);

    let outcome = scan(&tree.root.join("Game"));
    assert!(!outcome.cancelled);
    let top = &outcome.candidates[0];
    assert!(
        top.relative.ends_with("Game-Win64-Shipping.exe"),
        "{:?}",
        top.relative
    );
    assert_eq!(
        top.score,
        80 + 60 + 10,
        "帧生成 80 + UE Shipping 60 + 超分 10"
    );
    assert!(top.reasons.iter().any(|reason| reason.contains("帧生成")));
    assert!(
        !outcome
            .candidates
            .iter()
            .any(|item| item.relative.contains("SomeHelper")),
        "辅助程序必须被排除"
    );
    // 两个同分候选（都带帧生成组件）→ 并列即不可信，必须人工确认
    assert!(
        trusted_renderer(&outcome.candidates).is_none(),
        "同分候选不能自动采纳：{:?}",
        outcome.candidates
    );
    // 去掉竞争者后就唯一可信
    let only: Vec<_> = outcome
        .candidates
        .iter()
        .filter(|item| item.relative.ends_with("Game-Win64-Shipping.exe"))
        .cloned()
        .collect();
    assert_eq!(
        trusted_renderer(&only).map(|item| item.relative.clone()),
        Some(top.relative.clone())
    );
}

#[test]
fn unity_pair_scores_sixty() {
    let tree = Tree::new("unity");
    tree.pe("Game/Game.exe", true, false);
    tree.file("Game/unityplayer.dll");
    tree.dir("Game/Game_Data");

    let outcome = scan(&tree.root.join("Game"));
    let top = &outcome.candidates[0];
    assert_eq!(top.score, 60);
    assert!(top.reasons.iter().any(|reason| reason.contains("Unity")));
}

#[test]
fn non_x64_and_dll_and_auxiliary_are_skipped() {
    let tree = Tree::new("filter");
    tree.pe("Game/Game32.exe", false, false);
    tree.pe("Game/Plugin.dll", true, true);
    tree.pe("Game/CrashReportClient.exe", true, false);
    tree.file("Game/readme.txt");

    let outcome = scan(&tree.root.join("Game"));
    assert!(outcome.candidates.is_empty(), "{:?}", outcome.candidates);
    assert!(is_x64_executable(&tree.pe("Game/ok.exe", true, false)));
    assert!(!is_x64_executable(&tree.root.join("Game/readme.txt")));
    assert!(is_auxiliary_executable("UnityCrashHandler64.exe"));
}

#[test]
fn skipped_directories_are_not_entered() {
    let tree = Tree::new("skip");
    tree.pe("Game/redist/vcredist_setup_Game.exe", true, false);
    tree.pe("Game/Binaries/Win64/Game-Win64-Shipping.exe", true, false);

    let outcome = scan(&tree.root.join("Game"));
    assert_eq!(outcome.candidates.len(), 1, "{:?}", outcome.candidates);
    assert!(outcome.candidates[0].relative.contains("Binaries"));
}

#[test]
fn depth_and_directory_budgets_report_issues() {
    let tree = Tree::new("limits");
    // 13 层深，制造第 13 层的 exe
    let mut deep = String::from("Game");
    for index in 0..13 {
        deep.push_str(&format!("/level{index}"));
    }
    tree.pe(&format!("{deep}/Deep.exe"), true, false);

    let shallow = renderer_candidates(
        &tree.root.join("Game"),
        None,
        DiscoveryLimits {
            max_directories: 12_000,
            max_depth: 3,
        },
        &never,
    )
    .expect("扫描");
    assert!(shallow.candidates.is_empty(), "{:?}", shallow.candidates);
    assert!(
        shallow
            .issues
            .iter()
            .any(|issue| issue.contains("达到扫描深度限制")),
        "{:?}",
        shallow.issues
    );

    let tight = renderer_candidates(
        &tree.root.join("Game"),
        None,
        DiscoveryLimits {
            max_directories: 2,
            max_depth: 12,
        },
        &never,
    )
    .expect("扫描");
    assert!(
        tight
            .issues
            .iter()
            .any(|issue| issue.contains("扫描目录达到上限")),
        "{:?}",
        tight.issues
    );
}

#[test]
fn cancellation_stops_the_scan() {
    let tree = Tree::new("cancel");
    for index in 0..5 {
        tree.pe(&format!("Game/dir{index}/Game{index}.exe"), true, false);
    }
    let calls = AtomicUsize::new(0);
    let cancel = || {
        let count = calls.fetch_add(1, Ordering::Relaxed);
        count >= 3
    };
    let outcome = renderer_candidates(
        &tree.root.join("Game"),
        None,
        DiscoveryLimits::default(),
        &cancel,
    )
    .expect("扫描");
    assert!(outcome.cancelled, "应被取消");
}

#[test]
fn declared_executable_gets_the_bonus() {
    let tree = Tree::new("declared");
    let declared = tree.pe("Game/Binaries/Win64/Game-Win64-Shipping.exe", true, false);
    tree.file("Game/Binaries/Win64/nvngx_dlssg.dll");
    let outcome = renderer_candidates(
        &tree.root.join("Game"),
        Some(&declared),
        DiscoveryLimits::default(),
        &never,
    )
    .expect("扫描");
    let top = &outcome.candidates[0];
    assert_eq!(top.score, 80 + 60 + 30);
    assert!(top.reasons.iter().any(|reason| reason.contains("平台声明")));
}

#[test]
fn missing_directory_is_an_error() {
    let flag = AtomicBool::new(false);
    let error = renderer_candidates(
        Path::new(r"D:\definitely-not-here-dlssg"),
        None,
        DiscoveryLimits::default(),
        &|| flag.load(Ordering::Relaxed),
    )
    .expect_err("应报错");
    assert!(error.to_string().contains("普通文件夹"), "{error}");
}

#[test]
fn discover_tags_candidates_per_source_and_dedupes_roots() {
    use launcher_core::discovery::{DiscoveryRoot, discover};

    let tree = Tree::new("roots");
    tree.pe(
        "SteamGame/Binaries/Win64/Steam-Win64-Shipping.exe",
        true,
        false,
    );
    tree.file("SteamGame/Binaries/Win64/nvngx_dlssg.dll");
    tree.pe("GogGame/Game.exe", true, false);

    let roots = vec![
        DiscoveryRoot {
            label: "Steam".to_owned(),
            path: tree.root.join("SteamGame"),
        },
        DiscoveryRoot {
            label: "Duplicate".to_owned(),
            path: tree.root.join("SteamGame/"),
        },
        DiscoveryRoot {
            label: "GOG".to_owned(),
            path: tree.root.join("GogGame"),
        },
        DiscoveryRoot {
            label: "Missing".to_owned(),
            path: tree.root.join("Nope"),
        },
    ];

    let reports = discover(&roots, None, DiscoveryLimits::default(), &never);
    assert_eq!(
        reports.len(),
        3,
        "重复路径必须合并：{:?}",
        reports.iter().map(|r| &r.label).collect::<Vec<_>>()
    );
    assert_eq!(reports[0].label, "Steam");
    assert_eq!(reports[0].outcome.candidates.len(), 1);
    assert_eq!(reports[1].label, "GOG");
    assert_eq!(reports[2].label, "Missing");
    assert!(
        reports[2]
            .outcome
            .issues
            .iter()
            .any(|issue| issue.contains("普通文件夹")),
        "{:?}",
        reports[2].outcome.issues
    );
}
