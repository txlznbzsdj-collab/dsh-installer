//! 启动 `dsh web` 并检测本地服务端口。

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::npm::DEFAULT_PORT;

/// 判断本机端口是否已有服务监听。
pub fn port_is_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(350),
    )
    .is_ok()
}

/// Resolve a usable loopback port for DSH.
///
/// Windows may reserve sizeable dynamic port ranges. Binding one of those ports
/// fails with EACCES even when no process is listening, which used to leave the
/// launcher showing a generic timeout. Keep an already-running DSH instance on
/// the requested port; otherwise probe the requested port and nearby ports.
pub fn resolve_launch_port(requested: u16) -> Result<u16, String> {
    if port_is_open(requested) {
        return Ok(requested);
    }

    let end = requested.saturating_add(1000);
    for port in requested..=end {
        if TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], port))).is_ok() {
            return Ok(port);
        }
    }

    Err(format!(
        "端口 {requested} 至 {end} 均不可用，请检查 Windows 端口保留范围或防火墙设置"
    ))
}

/// 在独立控制台窗口中启动 dsh web（不阻塞）。
/// `prefix_args` 是入口程序前置参数（如本地模式下的 bin.js 路径）。
pub fn launch_dsh_web(
    entry: &Path,
    prefix_args: &[String],
    port: u16,
) -> Result<std::process::Child, String> {
    let mut cmd = if cfg!(windows) {
        let lower = entry.to_string_lossy().to_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            let mut c = Command::new("cmd");
            c.arg("/c").arg(entry);
            c
        } else {
            Command::new(entry)
        }
    } else {
        Command::new(entry)
    };
    cmd.args(prefix_args);
    cmd.arg("web");
    if port != DEFAULT_PORT {
        cmd.arg("--port").arg(port.to_string());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // Keep stderr so the launcher can show the real DSH startup error.
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 dsh 失败: {e}"))?;

    // 保持子进程句柄存活，避免其控制台被回收（仅 Windows 需要）。
    Ok(child)
}

/// 打开系统默认浏览器。
#[cfg(not(windows))]
pub fn open_browser(url: &str) {
    let _ = if cfg!(windows) {
        Command::new("cmd").args(["/c", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
}

/// 向标准错误打印一行（调试辅助）。
#[allow(dead_code)]
pub fn debug_line(line: &str) {
    let _ = std::io::stderr().write_all(format!("{line}\n").as_bytes());
}
