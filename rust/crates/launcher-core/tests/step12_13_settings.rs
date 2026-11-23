//! Phase 1 步 12–13：12 键模型、诊断模板生成与识别。
//!
//! 判据全部来自 M0 的黄金样本：
//! - `templates/<game>/0.4.2.ini` + 参数矩阵 → `diagnostic/<game>/<combo>.ini`（逐字节）
//! - `diagnostic/recognition/*.ini` + `recognition.json`（接受/拒绝）
//! - 0.4.0 / 0.4.0-mode-off / 0.3.2 旧模板必须仍被识别

use launcher_core::settings::{
    DiagnosticAcceptance, FlowResolution, MARKER_PREFIX, Preset, RuntimeMode, Settings,
    SuperResolutionMode, UseHudless, extract_diagnostic_values, is_diagnostic_file_name,
    marker_identity, recognizes_diagnostic, render_diagnostic,
};

const GAMES: [&str; 3] = ["wuthering", "neverness", "steam-ue5"];

struct Combo {
    name: &'static str,
    mode: SuperResolutionMode,
    preset: Preset,
    optimized: u8,
    level: u8,
}

const COMBOS: [Combo; 4] = [
    Combo {
        name: "default",
        mode: SuperResolutionMode::Off,
        preset: Preset::A,
        optimized: 2,
        level: 2,
    },
    Combo {
        name: "sr-m-int8",
        mode: SuperResolutionMode::MInt8,
        preset: Preset::C,
        optimized: 3,
        level: 3,
    },
    Combo {
        name: "sr-l-fp16",
        mode: SuperResolutionMode::LFp16,
        preset: Preset::B,
        optimized: 0,
        level: 0,
    },
    Combo {
        name: "fair",
        mode: SuperResolutionMode::Off,
        preset: Preset::B,
        optimized: 1,
        level: 1,
    },
];

fn template(game: &str, version: &str) -> String {
    launcher_core::fixtures::read_text(&format!("templates/{game}/{version}.ini"))
}

/// 与 `Test-StaleDiagnosticIniTarget` 相同的接受集合：
/// 0.4.2 模板可再生成，其余旧文本逐字比较。
fn acceptance_for(game: &str) -> DiagnosticAcceptance {
    let legacy: Vec<String> = match game {
        "wuthering" => ["0.4.0", "0.4.0-mode-off", "0.3.2"]
            .iter()
            .map(|version| template("wuthering", version))
            .collect(),
        other => vec![template(other, "0.4.0")],
    };
    DiagnosticAcceptance::new(template(game, "0.4.2"), legacy)
}

#[test]
fn diagnostic_rendering_matches_every_recorded_fixture_byte_for_byte() {
    for game in GAMES {
        let source = template(game, "0.4.2");
        for combo in &COMBOS {
            let rendered = render_diagnostic(
                &source,
                combo.mode,
                combo.preset,
                combo.optimized,
                combo.level,
            );
            let expected = launcher_core::fixtures::read_text(&format!(
                "diagnostic/{game}/{}.ini",
                combo.name
            ));
            assert_eq!(rendered, expected, "{game} / {}", combo.name);
        }
    }
}

#[test]
fn recognition_matches_the_recorded_cases() {
    let index: serde_json::Value = serde_json::from_str(&launcher_core::fixtures::read_text(
        "diagnostic/recognition.json",
    ))
    .expect("索引");
    let template_text = template("wuthering", "0.4.2");
    let cases = index["Cases"].as_array().expect("Cases");
    assert_eq!(cases.len(), 9, "样本用例数");
    for case in cases {
        let name = case["Case"].as_str().expect("Case");
        let expected = case["Accepted"].as_bool().expect("Accepted");
        let contents =
            launcher_core::fixtures::read_text(&format!("diagnostic/recognition/{name}.ini"));
        let actual = recognizes_diagnostic(&template_text, &contents);
        assert_eq!(actual, expected, "用例 {name}");
    }
}

#[test]
fn legacy_templates_are_matched_literally_or_regenerated() {
    // 0.4.0 / 0.4.0-mode-off / 0.3.2 在现有实现里是逐字比较，不经过"提取→重生成"
    for game in GAMES {
        let acceptance = acceptance_for(game);
        for version in ["0.4.2", "0.4.0"] {
            let text = template(game, version);
            assert!(acceptance.accepts(&text), "{game}/{version} 应被接受");
        }
    }
    let wuthering = acceptance_for("wuthering");
    assert!(wuthering.accepts(&template("wuthering", "0.3.2")));
    assert!(wuthering.accepts(&template("wuthering", "0.4.0-mode-off")));
}

#[test]
fn per_game_template_sets_only_accept_their_own_legacy_texts() {
    // 鸣潮接受四份旧文本；异环与 Steam 只接受 0.4.2 与 0.4.0（PARITY C7）
    let wuthering = acceptance_for("wuthering");
    let neverness = acceptance_for("neverness");

    let wuthering_old = template("wuthering", "0.3.2");
    assert!(wuthering.accepts(&wuthering_old));
    assert!(
        !neverness.accepts(&wuthering_old),
        "异环不应认领鸣潮的 0.3.2 文本"
    );
    let nte_old = template("neverness", "0.4.0");
    assert!(neverness.accepts(&nte_old));
    assert!(!wuthering.accepts(&nte_old));
}

#[test]
fn diagnostic_file_name_rules_follow_the_template_prefixes() {
    assert!(is_diagnostic_file_name(
        "dlssg_sm86-wuthering-",
        "dlssg_sm86-wuthering-4242.ini"
    ));
    assert!(is_diagnostic_file_name(
        "dlssg_sm86-nte-",
        "dlssg_sm86-nte-1.ini"
    ));
    assert!(is_diagnostic_file_name(
        "dlssg_sm86-steam-4356430-",
        "dlssg_sm86-steam-4356430-99.ini"
    ));
    assert!(!is_diagnostic_file_name(
        "dlssg_sm86-nte-",
        "dlssg_sm86-wuthering-1.ini"
    ));
}

#[test]
fn extract_rejects_duplicates_and_unsupported_values() {
    let text = launcher_core::fixtures::read_text("diagnostic/recognition/duplicate-key.ini");
    assert!(
        extract_diagnostic_values(&text).is_none(),
        "重复键应提取失败"
    );
    let text = launcher_core::fixtures::read_text("diagnostic/recognition/unsupported-value.ini");
    assert!(
        extract_diagnostic_values(&text).is_none(),
        "非法值应提取失败"
    );
}

#[test]
fn settings_can_be_imported_from_the_release_ini() {
    // 发行配置：11 键（12 键模型里 Path 只对 External/Pinned 有意义）
    let repo_root = launcher_core::fixtures::root()
        .parent()
        .expect("rust 目录")
        .parent()
        .expect("仓库根")
        .to_path_buf();
    let release = std::fs::read_to_string(repo_root.join("dlssg_sm86.ini")).expect("发行配置");
    let mut settings = Settings::default();
    let report = settings.apply_ini(&release).expect("导入");

    assert_eq!(report.unknown, Vec::<String>::new(), "不应有未知键");
    assert_eq!(report.applied.len(), 11, "发行配置是 11 键");
    assert!(settings.enabled);
    assert_eq!(settings.max_generated_frames, 3);
    assert_eq!(settings.preset, Preset::A);
    assert_eq!(settings.optimized, 1);
    assert_eq!(settings.level, 1);
    assert_eq!(settings.super_resolution, SuperResolutionMode::Off);
    assert_eq!(settings.flow_resolution, FlowResolution::Middle);
    assert_eq!(settings.use_hudless, UseHudless::Auto);
    assert_eq!(settings.runtime_mode, RuntimeMode::Bundled);
    assert_eq!(settings.directory, "dlssg_sm86\\logs");
    settings.validate().expect("发行值应合法");
}

#[test]
fn render_then_reimport_round_trips() {
    let settings = Settings {
        engine: launcher_core::settings::EngineKind::XeSs,
        max_generated_frames: 2,
        super_resolution: SuperResolutionMode::MInt8,
        level: 3,
        flow_resolution: FlowResolution::Fine,
        use_hudless: UseHudless::One,
        ..Settings::default()
    };

    let text = settings.render("0123456789abcdef").expect("渲染");
    assert_eq!(marker_identity(&text), Some("0123456789abcdef"));

    let mut imported = Settings::default();
    let report = imported.apply_ini(&text).expect("重新导入");
    assert_eq!(report.unknown, Vec::<String>::new());
    assert_eq!(imported, settings);
    assert_eq!(imported.render("0123456789abcdef").expect("再渲染"), text);
}

#[test]
fn custom_ini_keeps_user_text_and_skips_validation() {
    let custom = "; 我自己的配置\r\n[General]\r\nEnabled=0\r\n[Custom]\r\nFoo=1\r\n".to_owned();
    let settings = Settings {
        custom_ini: Some(custom.clone()),
        max_generated_frames: 250, // 自定义模式不校验
        ..Settings::default()
    };
    let text = settings.render("deadbeef").expect("渲染");
    assert!(text.starts_with(&format!("{MARKER_PREFIX}deadbeef\r\n")));
    assert!(text.ends_with(&custom));
    assert_eq!(marker_identity(&text), Some("deadbeef"));
}

#[test]
fn import_reports_unknown_sections_without_failing() {
    let mut settings = Settings::default();
    let report = settings
        .apply_ini("[General]\r\nEnabled=1\r\n[Compatibility]\r\nFoo=Bar\r\n")
        .expect("导入");
    assert_eq!(report.unknown, vec!["Compatibility.Foo".to_owned()]);
    assert!(report.hints.iter().any(|hint| hint.contains("12 键模型")));
}

#[test]
fn render_output_has_no_bare_lf() {
    let pinned = Settings {
        runtime_mode: RuntimeMode::Pinned,
        runtime_path: Some(r"D:\x\nvngx_dlssg.dll".to_owned()),
        ..Settings::default()
    };
    for settings in [Settings::default(), pinned] {
        let text = settings.render("id").expect("渲染");
        let bare = text.matches('\n').count() - text.matches("\r\n").count();
        assert_eq!(bare, 0, "生成文本必须全 CRLF");
    }
}
