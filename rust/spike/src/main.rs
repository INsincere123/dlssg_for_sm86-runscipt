//! M0 egui spike：只回答四个问题，不实现任何启动器逻辑。
//! 1) 空窗口 + 中文 + PerMonitorV2 的常驻内存；2) 冷启动到首帧；3) `windows` 子系统下有无控制台窗口；
//! 4) `ShellExecuteEx(runas)` 拉起隐藏子进程的链路（UAC 需人工确认）。
//!
//! 用法：
//!   spike.exe --report <json>            图形模式，首帧写报告后退出
//!   spike.exe --report <json> --hold <ms> 图形模式，保持 <ms> 毫秒（供外部采样内存）
//!   spike.exe --child <json>             子进程模式，写一行后退出，不创建窗口
//!   spike.exe --elevate <json>           以 runas 拉起自身 --child，观察报告是否出现

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Instant;

type Hwnd = isize;
type Handle = isize;

#[repr(C)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[repr(C)]
struct ShellExecuteInfoW {
    cb_size: u32,
    f_mask: u32,
    hwnd: Hwnd,
    lp_verb: *const u16,
    lp_file: *const u16,
    lp_parameters: *const u16,
    lp_directory: *const u16,
    n_show: i32,
    h_inst_app: Handle,
    lp_idlist: *mut c_void,
    lp_class: *const u16,
    h_key_class: Handle,
    dw_hot_key: u32,
    h_icon_or_monitor: Handle,
    h_process: Handle,
    h_thread: Handle,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetConsoleWindow() -> Hwnd;
    fn GetCurrentProcess() -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
    fn GetDpiForSystem() -> u32;
}

#[link(name = "psapi")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(process: Handle, counters: *mut ProcessMemoryCounters, cb: u32) -> i32;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> i32;
}

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
const SEE_MASK_NOASYNC: u32 = 0x0000_0100;

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn working_set_bytes() -> usize {
    let mut counters: ProcessMemoryCounters = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
    let cb = counters.cb;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, cb) };
    if ok == 0 {
        0
    } else {
        counters.working_set_size
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn elevate_child(report: PathBuf) -> String {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => return format!("current_exe failed: {error}"),
    };
    let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
    let file = wide(exe.as_os_str());
    let parameters = wide(OsStr::new(&format!("--child \"{}\"", report.display())));
    let mut info: ShellExecuteInfoW = unsafe { std::mem::zeroed() };
    info.cb_size = std::mem::size_of::<ShellExecuteInfoW>() as u32;
    info.f_mask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lp_verb = verb.as_ptr();
    info.lp_file = file.as_ptr();
    info.lp_parameters = parameters.as_ptr();
    info.n_show = 0; // SW_HIDE
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        return format!(
            "ShellExecuteExW failed (uac denied or cancelled): {}",
            std::io::Error::last_os_error()
        );
    }
    if info.h_process != 0 {
        unsafe {
            WaitForSingleObject(info.h_process, 20_000);
            CloseHandle(info.h_process);
        }
    }
    if report.exists() {
        format!("elevated child wrote {}", report.display())
    } else {
        "elevated child produced no report (uac pending/denied or hidden child failed)".to_string()
    }
}

struct Metrics {
    entry: Instant,
    first_frame_ms: Option<f64>,
    dpi_awareness_applied: bool,
    dpi_system: u32,
    font_bytes: usize,
    font_error: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut report: Option<PathBuf> = None;
    let mut child: Option<PathBuf> = None;
    let mut elevate: Option<PathBuf> = None;
    let mut hold_ms: u64 = 0;
    let mut font_path = String::from("C:\\Windows\\Fonts\\msyh.ttc");
    let mut no_font = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--no-font" => {
                no_font = true;
                index += 1;
            }
            "--report" => {
                report = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--child" => {
                child = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--elevate" => {
                elevate = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--font" => {
                if let Some(value) = args.get(index + 1) {
                    font_path = value.clone();
                }
                index += 2;
            }
            "--hold" => {
                hold_ms = args
                    .get(index + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                index += 2;
            }
            _ => index += 1,
        }
    }

    if let Some(path) = elevate {
        let result = elevate_child(path);
        println!("{result}");
        return;
    }

    // 子进程模式：证明「无控制台窗口」的进程能启动并落盘，不创建任何窗口。
    if let Some(path) = child {
        let payload = format!(
            "child pid={} console_window={}\r\n",
            std::process::id(),
            unsafe { GetConsoleWindow() }
        );
        let _ = std::fs::write(&path, payload);
        return;
    }

    let entry = Instant::now();
    let applied =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } != 0;
    let _ = unsafe { CoInitializeEx(std::ptr::null_mut(), 2) };

    let (font_bytes, font_error, font_data) = if no_font {
        (0, "disabled by --no-font".to_string(), None)
    } else {
        match std::fs::read(&font_path) {
            Ok(bytes) => {
                let len = bytes.len();
                (len, String::new(), Some(bytes))
            }
            Err(error) => (0, error.to_string(), None),
        }
    };

    let metrics = Metrics {
        entry,
        first_frame_ms: None,
        dpi_awareness_applied: applied,
        dpi_system: unsafe { GetDpiForSystem() },
        font_bytes,
        font_error,
    };
    let report_path = report.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 580.0])
            .with_title("DLSSG Launcher M0 Spike"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "DLSSG Launcher M0 Spike",
        options,
        Box::new(move |cc| {
            if let Some(bytes) = font_data {
                let mut fonts = egui::FontDefinitions::default();
                fonts
                    .font_data
                    .insert("msyh".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(family).or_default().insert(0, "msyh".to_owned());
                }
                cc.egui_ctx.set_fonts(fonts);
            }
            Ok(Box::new(SpikeApp {
                metrics,
                hold_ms,
                report_path,
                frame: 0,
                close_after: None,
            }))
        }),
    );

    if let Err(error) = result {
        println!("eframe error: {error}");
    }
}

struct SpikeApp {
    metrics: Metrics,
    hold_ms: u64,
    report_path: Option<PathBuf>,
    frame: u32,
    close_after: Option<Instant>,
}

impl SpikeApp {
    fn write_report(&mut self, pixels_per_point: f32) {
        if let Some(path) = self.report_path.take() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let cold = self.metrics.first_frame_ms.unwrap_or(-1.0);
            let json = format!(
                concat!(
                    "{{\n",
                    "  \"exe\": \"{exe}\",\n",
                    "  \"pid\": {pid},\n",
                    "  \"cold_start_ms\": {cold:.2},\n",
                    "  \"first_frame_unix_ms\": {first_frame},\n",
                    "  \"console_window\": {console},\n",
                    "  \"dpi_awareness_applied\": {applied},\n",
                    "  \"dpi_system\": {dpi_system},\n",
                    "  \"native_pixels_per_point\": {ppp:.3},\n",
                    "  \"font_msyh_bytes\": {font_bytes},\n",
                    "  \"font_error\": \"{font_error}\",\n",
                    "  \"working_set_kb\": {ws_kb}\n",
                    "}}\n"
                ),
                exe = escape(
                    &std::env::current_exe()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ),
                pid = std::process::id(),
                cold = cold,
                ppp = pixels_per_point,
                first_frame = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
                console = unsafe { GetConsoleWindow() } != 0,
                applied = self.metrics.dpi_awareness_applied,
                dpi_system = self.metrics.dpi_system,
                font_bytes = self.metrics.font_bytes,
                font_error = escape(&self.metrics.font_error),
                ws_kb = working_set_bytes() / 1024,
            );
            let _ = std::fs::write(&path, json);
        }
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame += 1;
        if self.metrics.first_frame_ms.is_none() {
            self.metrics.first_frame_ms = Some(self.metrics.entry.elapsed().as_secs_f64() * 1000.0);
            let ppp = root.ctx().pixels_per_point();
            self.write_report(ppp);
            if self.hold_ms == 0 {
                root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.close_after = Some(Instant::now() + std::time::Duration::from_millis(self.hold_ms));
            }
        }
        if let Some(deadline) = self.close_after {
            if Instant::now() >= deadline {
                root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("DLSSG 启动器 M0 Spike");
            ui.label("中文字形测试：鸣潮 / 异环 / 黑神话：悟空 / 帧生成 / 超分辨率");
            ui.label("English mixed 文本 0123456789 —— 检查字重与断行");
            ui.separator();
            ui.label(format!("frame = {}", self.frame));
            ui.label(format!("native_pixels_per_point = {:.3}", ui.ctx().pixels_per_point()));
            ui.label(format!(
                "dpi_system = {}，DPI 感知上下文 = {}",
                self.metrics.dpi_system, self.metrics.dpi_awareness_applied
            ));
            ui.label(format!("font msyh = {} bytes", self.metrics.font_bytes));
            if !self.metrics.font_error.is_empty() {
                ui.label(format!("font error: {}", self.metrics.font_error));
            }
            ui.label(format!("working set = {} KB", working_set_bytes() / 1024));
            ui.label(format!("console window handle = {}", unsafe { GetConsoleWindow() }));
        });

        if self.hold_ms > 0 {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}
