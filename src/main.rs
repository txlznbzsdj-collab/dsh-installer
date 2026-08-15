//! DSH 安装助手 — DeepSeek Harness 图形化安装程序入口。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod core;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--repair-shortcut") {
        std::process::exit(repair_shortcut());
    }

    // 桌面快捷方式入口：确保 DSH 服务运行，然后打开内嵌网页窗口。
    if std::env::args().any(|a| a == "--launch-web") {
        std::process::exit(run_web_launcher());
    }

    // 无头自检模式（开发测试/诊断用；release 无控制台，报告写入文件）
    if std::env::args().any(|a| a == "--selftest") {
        std::process::exit(run_selftest());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DSH 安装助手 — DeepSeek Harness 一键安装")
            .with_icon(std::sync::Arc::new(app_icon()))
            .with_inner_size([900.0, 620.0])
            .with_min_inner_size([860.0, 560.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "dsh-installer",
        options,
        Box::new(|cc| Ok(Box::new(app::DshInstallerApp::new(cc)))),
    )
}

fn app_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/dsh-icon.png"))
        .expect("embedded DSH icon must be valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn repair_shortcut() -> i32 {
    let facts = crate::core::detect::detect();
    let Some(entry) = facts.dsh_cmd else {
        return 1;
    };
    let Ok(installer) = std::env::current_exe() else {
        return 1;
    };
    let args = format!(
        "--launch-web --entry \"{}\" --port {}",
        entry.display(),
        crate::core::npm::DEFAULT_PORT
    );
    match crate::core::shortcut::create_desktop_shortcut(
        crate::core::shortcut::SHORTCUT_NAME,
        &installer,
        &args,
        Some(&installer),
    ) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

fn run_web_launcher() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let value_after = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
    };
    let Some(entry) = value_after("--entry").map(PathBuf::from) else {
        return 2;
    };
    let requested_port = value_after("--port")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(crate::core::npm::DEFAULT_PORT);
    let port = match crate::core::launch::resolve_launch_port(requested_port) {
        Ok(port) => port,
        Err(_) => return 1,
    };
    let url = format!("http://127.0.0.1:{port}");

    if crate::core::launch::port_is_open(port) {
        return if crate::core::webview::open(url, port, None).is_ok() {
            0
        } else {
            1
        };
    }
    match crate::core::launch::launch_dsh_web(&entry, &[], port) {
        Ok(child) => match crate::core::webview::open(url, port, Some(child)) {
            Ok(()) => 0,
            Err(_) => 1,
        },
        Err(_) => 1,
    }
}

/// 自检报告文件路径：位于可执行文件同目录。
fn report_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("dsh-installer-selftest.txt")))
        .unwrap_or_else(|| PathBuf::from("dsh-installer-selftest.txt"))
}

/// 输出到 stdout（若有控制台）并追加写入报告文件。
fn report(line: &str) {
    println!("{line}");
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(report_path())
    {
        let _ = writeln!(f, "{line}");
    }
}

/// `--selftest [--install] [--local <dir>]`：无头执行环境检测，可选执行安装。
fn run_selftest() -> i32 {
    use crate::core::detect;
    use crate::core::log::new_shared_log;
    use crate::core::npm::{self, InstallMode};

    // 覆盖旧报告
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(report_path())
    {
        let _ = writeln!(f, "=== DSH Installer self-test ===");
    }

    let args: Vec<String> = std::env::args().collect();
    let do_install = args.iter().any(|a| a == "--install");
    let local_dir = args
        .iter()
        .position(|a| a == "--local")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);

    report(&format!("report file: {}", report_path().display()));
    report(&format!("OS: {}", std::env::consts::OS));
    report(&format!("DSH package: {}", npm::DSH_PACKAGE));

    let facts = detect::detect();
    report(&format!(
        "node path    : {}",
        facts
            .node_path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "NOT FOUND".into())
    ));
    report(&format!(
        "node version : {}",
        facts.node_version.as_deref().unwrap_or("N/A")
    ));
    report(&format!(
        "npm path     : {}",
        facts
            .npm_path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "NOT FOUND".into())
    ));
    report(&format!(
        "npm version  : {}",
        facts.npm_version.as_deref().unwrap_or("N/A")
    ));
    report(&format!(
        "npm prefix   : {}",
        facts.npm_prefix.as_deref().unwrap_or("N/A")
    ));
    report(&format!(
        "dsh version  : {}",
        facts.dsh_version.as_deref().unwrap_or("(not installed)")
    ));

    if !do_install {
        report("(pass --install to run a real install)");
        return if facts.node_ok && facts.npm_path.is_some() {
            0
        } else {
            1
        };
    }

    let Some(node) = facts.node_path.clone() else {
        report("ERROR: node not found");
        return 1;
    };
    let Some(npm) = facts.npm_path.clone() else {
        report("ERROR: npm not found");
        return 1;
    };
    let npm_prefix = facts.npm_prefix.as_deref().map(std::path::Path::new);
    let mode = match &local_dir {
        Some(dir) => InstallMode::Local(dir.clone()),
        None => InstallMode::Global,
    };
    report(&format!("install mode : {}", mode.label()));

    let log = new_shared_log();
    let result = npm::install_dsh(&log, &npm, &node, npm_prefix, &mode);

    let buf = log.lock().unwrap();
    report("");
    report("--- npm output ---");
    for line in &buf.lines {
        report(line);
    }
    report("--- end ---");
    drop(buf);

    // 本地模式：生成启动器并验证
    if let InstallMode::Local(dir) = &mode {
        match npm::write_local_launcher(dir, &node) {
            Ok(launcher) => {
                report(&format!("launcher     : {}", launcher.display()));
                match crate::core::detect::run_capture(&launcher, &["-V"]) {
                    Ok((out, _err, 0)) => report(&format!("launcher -V  : {out}")),
                    _ => report("launcher -V  : FAILED"),
                }
            }
            Err(e) => report(&format!("launcher write FAILED: {e}")),
        }
    }

    match result {
        Ok(v) => {
            report(&format!("\nSELFTEST OK: dsh {v}"));
            0
        }
        Err(e) => {
            report(&format!("\nSELFTEST FAIL: {e}"));
            1
        }
    }
}
