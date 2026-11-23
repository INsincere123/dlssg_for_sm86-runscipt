//! Phase 1 步 4：`启动配置.ini` 的路径语法。
//!
//! 对照 `Resolve-LauncherConfigPath`（`LauncherConfig.ps1:122-168`）。
//! 环境变量与 `~` 都通过注入提供，测试不依赖真实进程环境。

use std::path::Path;

use launcher_core::Error;
use launcher_core::config::PathEnvironment;

const ROOT: &str = r"D:\Games\dlssg_for_sm86\启动脚本";
const HOME: &str = r"C:\Users\Mutsumi";

fn environment<'a>(get_env: &'a dyn Fn(&str) -> Option<String>) -> PathEnvironment<'a> {
    PathEnvironment {
        root: Path::new(ROOT),
        home: Some(Path::new(HOME)),
        get_env,
    }
}

fn no_env(_name: &str) -> Option<String> {
    None
}

fn sample_env(name: &str) -> Option<String> {
    match name.to_ascii_uppercase().as_str() {
        "TEMP" => Some(r"C:\Users\Mutsumi\AppData\Local\Temp".to_owned()),
        "GAMEDIR" => Some(r"E:\Games".to_owned()),
        _ => None,
    }
}

#[test]
fn empty_value_stays_empty() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(env.resolve("", "配置源").expect("解析"), "");
    assert_eq!(env.resolve("   ", "配置源").expect("解析"), "");
}

#[test]
fn relative_value_is_joined_to_the_launcher_root() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(
        env.resolve(r"配置\鸣潮\Engine.ini", "配置源")
            .expect("解析"),
        r"D:\Games\dlssg_for_sm86\启动脚本\配置\鸣潮\Engine.ini"
    );
    assert_eq!(
        env.resolve("./配置/异环/Engine.ini", "配置源")
            .expect("解析"),
        r"D:\Games\dlssg_for_sm86\启动脚本\配置\异环\Engine.ini"
    );
}

#[test]
fn leading_single_backslash_uses_the_root_drive() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(
        env.resolve(r"\Games\x.ini", "配置源").expect("解析"),
        r"D:\Games\x.ini"
    );
}

#[test]
fn absolute_paths_pass_through_unchanged() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(
        env.resolve(r"D:\WuWa\Engine.ini", "配置源").expect("解析"),
        r"D:\WuWa\Engine.ini"
    );
    assert_eq!(
        env.resolve(r"\\server\share\game.ini", "配置源")
            .expect("解析"),
        r"\\server\share\game.ini"
    );
}

#[test]
fn quotes_are_stripped_before_resolution() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(
        env.resolve("\"配置\\鸣潮\\Engine.ini\"", "配置源")
            .expect("解析"),
        r"D:\Games\dlssg_for_sm86\启动脚本\配置\鸣潮\Engine.ini"
    );
    assert!(env.resolve("\"未闭合", "配置源").is_err());
}

#[test]
fn all_three_environment_syntaxes_expand_case_insensitively() {
    let get_env = sample_env;
    let env = environment(&get_env);
    let expected = r"C:\Users\Mutsumi\AppData\Local\Temp\x.ini";
    for raw in [
        r"%TEMP%\x.ini",
        r"%temp%\x.ini",
        r"${env:TEMP}\x.ini",
        r"$ENV:TEMP\x.ini",
    ] {
        assert_eq!(
            env.resolve(raw, "配置源").expect("解析"),
            expected,
            "输入：{raw}"
        );
    }
}

#[test]
fn undefined_variable_is_an_error_not_a_silent_pass() {
    let get_env = sample_env;
    let env = environment(&get_env);
    let error = env
        .resolve(r"%NOT_DEFINED%\x.ini", "配置源")
        .expect_err("应报错");
    let text = error.to_string();
    assert!(text.contains("未定义的环境变量"), "实际：{text}");
    assert!(text.contains("NOT_DEFINED"), "实际：{text}");
}

#[test]
fn tilde_expands_to_the_user_profile() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(env.resolve("~", "配置源").expect("解析"), HOME);
    assert_eq!(
        env.resolve(r"~\Saved\Config\Windows\Engine.ini", "配置源")
            .expect("解析"),
        r"C:\Users\Mutsumi\Saved\Config\Windows\Engine.ini"
    );
}

#[test]
fn filesystem_prefixes_are_stripped() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(
        env.resolve(r"FileSystem::D:\x\y.ini", "配置源")
            .expect("解析"),
        r"D:\x\y.ini"
    );
    assert_eq!(
        env.resolve(
            r"Microsoft.PowerShell.Core\FileSystem::D:\x\y.ini",
            "配置源"
        )
        .expect("解析"),
        r"D:\x\y.ini"
    );
}

#[test]
fn file_uris_are_accepted_for_files_only() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(
        env.resolve("file:///D:/Games/WuWa/Engine.ini", "配置源")
            .expect("解析"),
        r"D:\Games\WuWa\Engine.ini"
    );
    assert_eq!(
        env.resolve("file://server/share/game.ini", "配置源")
            .expect("解析"),
        r"\\server\share\game.ini"
    );
    assert_eq!(
        env.resolve("file:///D:/Games/a%20b/Engine.ini", "配置源")
            .expect("解析"),
        r"D:\Games\a b\Engine.ini"
    );
    let error = env
        .resolve("https://example.com/x.ini", "配置源")
        .expect_err("应报错");
    assert!(error.to_string().contains("file URI"), "实际：{error}");
    let with_query = env
        .resolve("file:///D:/x.ini?raw=1", "配置源")
        .expect_err("应报错");
    assert!(
        with_query.to_string().contains("查询串"),
        "实际：{with_query}"
    );
}

#[test]
fn extended_prefixes_are_normalised() {
    let get_env = no_env;
    let env = environment(&get_env);
    assert_eq!(
        env.resolve(r"\\?\D:\Games\Foo", "配置源").expect("解析"),
        r"D:\Games\Foo"
    );
    assert_eq!(
        env.resolve(r"\\?\UNC\server\share\game.ini", "配置源")
            .expect("解析"),
        r"\\server\share\game.ini"
    );
    let volume = env
        .resolve(r"\\?\Volume{1234}\x.ini", "配置源")
        .expect_err("应报错");
    assert!(volume.to_string().contains("扩展路径"), "实际：{volume}");
}

#[test]
fn ambiguous_drive_relative_paths_are_rejected() {
    let get_env = no_env;
    let env = environment(&get_env);
    for raw in ["C:", "C:foo", "d:bar\\x.ini"] {
        let error = env.resolve(raw, "配置源").expect_err("应报错");
        assert!(
            error.to_string().contains("盘符相对路径"),
            "{raw} → {error}"
        );
    }
}

#[test]
fn invalid_characters_are_rejected() {
    let get_env = no_env;
    let env = environment(&get_env);
    for raw in [r"D:\a<b", r"D:\a>b", r"D:\a|b", r"D:\a?b", r"D:\a*b"] {
        assert!(env.resolve(raw, "配置源").is_err(), "{raw} 应被拒绝");
    }
    // 单引号、井号、等号都是合法文件名字符
    assert_eq!(
        env.resolve(r"D:\a#b=c'\x.ini", "配置源").expect("解析"),
        r"D:\a#b=c'\x.ini"
    );
}

#[test]
fn error_messages_name_the_config_key() {
    let get_env = no_env;
    let env = environment(&get_env);
    let error = env.resolve("C:relative", "游戏目录").expect_err("应报错");
    let text = error.to_string();
    assert!(text.contains("游戏目录"), "实际：{text}");
    assert!(matches!(error, Error::Invalid { .. }));
}
