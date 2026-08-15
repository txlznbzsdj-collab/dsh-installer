//! 通过 npm 安装 / 卸载 / 校验 @deepseek-ai/dsh，输出流式写入共享日志。

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::log::SharedLog;

pub const DSH_PACKAGE: &str = "@deepseek-ai/dsh";
pub const DEFAULT_PORT: u16 = 3080;

#[derive(Debug, Clone, PartialEq)]
pub enum InstallMode {
    Global,
    Local(PathBuf),
}

impl InstallMode {
    pub fn label(&self) -> String {
        match self {
            InstallMode::Global => "全局安装 (npm -g)".to_string(),
            InstallMode::Local(dir) => format!("本地便携安装 ({})", dir.display()),
        }
    }
}

/// 在 Windows 上 .cmd/.bat 需要经由 cmd.exe 启动（CreateProcess 无法直接执行）。
/// 通用的命令启动：Windows 上自动为 .cmd/.bat 包装 cmd.exe /c。
fn spawn_any(program: &Path, args: &[&str]) -> Command {
    let lower = program.to_string_lossy().to_lowercase();
    let mut command = if cfg!(windows) && (lower.ends_with(".cmd") || lower.ends_with(".bat")) {
        let mut cmd = Command::new("cmd");
        cmd.arg("/d").arg("/c").arg(program).args(args);
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// 启动 npm 并流式收集输出到日志。
/// 返回 (退出码, 合并后的全部输出)。
fn run_npm_stream(log: &SharedLog, npm: &Path, args: &[&str]) -> Result<i32, String> {
    {
        let mut buf = log.lock().unwrap();
        buf.push(format!("> {} {}", npm.display(), args.join(" ")));
    }
    let mut child = spawn_any(npm, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 npm ({}): {e}", npm.display()))?;

    let stdout = child.stdout.take().ok_or("无法读取 npm 标准输出")?;
    let stderr = child.stderr.take().ok_or("无法读取 npm 错误输出")?;
    let log_stdout = std::sync::Arc::clone(log);
    let log_stderr = std::sync::Arc::clone(log);

    let t1 = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            log_stdout.lock().unwrap().push(line);
        }
    });
    let t2 = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            log_stderr.lock().unwrap().push(line);
        }
    });

    const NPM_TIMEOUT: Duration = Duration::from_secs(15 * 60);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < NPM_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(150));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "npm 执行超过 15 分钟，已停止本次安装；请检查网络或更换镜像源".to_string(),
                );
            }
            Err(error) => {
                let _ = child.kill();
                return Err(format!("等待 npm 进程失败: {error}"));
            }
        }
    };
    let _ = t1.join();
    let _ = t2.join();
    Ok(status.code().unwrap_or(-1))
}

/// 本地安装目录中 dsh 的 bin.js 路径。
fn local_bin_js(dir: &Path) -> PathBuf {
    dir.join("node_modules")
        .join(DSH_PACKAGE)
        .join("lib")
        .join("bin.js")
}

/// 安装 DSH。成功返回版本号，失败返回 Err。
pub fn install_dsh(
    log: &SharedLog,
    npm: &Path,
    node: &Path,
    npm_prefix: Option<&Path>,
    mode: &InstallMode,
) -> Result<String, String> {
    install_dsh_with_options(log, npm, node, npm_prefix, mode, DSH_PACKAGE, None)
}

/// 使用指定的软件包来源和 npm registry 安装 DSH。
/// `package_spec` 可以是包名、带版本的包名，或本地 `.tgz` 文件路径。
pub fn install_dsh_with_options(
    log: &SharedLog,
    npm: &Path,
    node: &Path,
    npm_prefix: Option<&Path>,
    mode: &InstallMode,
    package_spec: &str,
    registry: Option<&str>,
) -> Result<String, String> {
    let mut args = vec!["install".to_string()];
    match mode {
        InstallMode::Global => args.push("-g".to_string()),
        InstallMode::Local(dir) => {
            args.push("--prefix".to_string());
            args.push(dir.to_string_lossy().to_string());
        }
    }
    args.extend(["--no-audit".to_string(), "--no-fund".to_string()]);
    if let Some(registry) = registry.filter(|r| !r.trim().is_empty()) {
        args.push("--registry".to_string());
        args.push(registry.trim().to_string());
    }
    args.push(package_spec.to_string());
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();

    let code = run_npm_stream(log, npm, &args_ref)?;

    if code != 0 {
        return Err(format!("npm 安装失败 (退出码 {code})，请查看上方日志"));
    }

    // 校验版本
    match verify_dsh_version(node, npm_prefix, mode) {
        Some(v) => {
            log.lock().unwrap().push(format!("✓ dsh {v} 安装成功"));
            Ok(v)
        }
        None => Err("npm 已执行完成，但未能校验 dsh 版本，请查看上方日志".to_string()),
    }
}

/// 卸载 DSH。成功返回摘要信息。
pub fn uninstall_dsh(log: &SharedLog, npm: &Path, mode: &InstallMode) -> Result<String, String> {
    let code = match mode {
        InstallMode::Global => run_npm_stream(log, npm, &["uninstall", "-g", DSH_PACKAGE])?,
        InstallMode::Local(dir) => {
            let _ = std::fs::remove_dir_all(dir.join("node_modules"));
            let _ = std::fs::remove_file(dir.join("package-lock.json"));
            let _ = std::fs::remove_file(dir.join("dsh.cmd"));
            let _ = std::fs::remove_file(dir.join("dsh"));
            log.lock()
                .unwrap()
                .push(format!("已删除本地安装目录 {}", dir.display()));
            0
        }
    };
    if code != 0 {
        return Err(format!("npm 卸载失败 (退出码 {code})"));
    }
    log.lock().unwrap().push("✓ DSH 已卸载".to_string());
    Ok("DSH 已卸载".to_string())
}

/// 校验已安装的 dsh 版本。
pub fn verify_dsh_version(
    node: &Path,
    npm_prefix: Option<&Path>,
    mode: &InstallMode,
) -> Option<String> {
    let (program, args): (PathBuf, Vec<String>) = match mode {
        InstallMode::Global => {
            // dsh.cmd 位于 npm 全局 bin 目录（npm prefix -g）。
            let mut cands: Vec<PathBuf> = Vec::new();
            if let Some(prefix) = npm_prefix {
                cands.push(prefix.join("dsh.cmd"));
                cands.push(prefix.join("dsh"));
            }
            if let Some(node_dir) = node.parent() {
                cands.push(node_dir.join("dsh.cmd"));
                cands.push(node_dir.join("dsh"));
            }
            cands.push(PathBuf::from("dsh"));
            let dsh = cands.into_iter().find(|c| c.is_file())?;
            (dsh, vec!["-V".to_string()])
        }
        InstallMode::Local(dir) => {
            let bin = local_bin_js(dir);
            if !bin.is_file() {
                return None;
            }
            (
                node.to_path_buf(),
                vec![bin.to_string_lossy().to_string(), "-V".to_string()],
            )
        }
    };

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = spawn_any(&program, &args_ref).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// 为本地安装模式生成启动器脚本（dsh.cmd / dsh）。
/// 返回启动器路径。
pub fn write_local_launcher(dir: &Path, node: &Path) -> std::io::Result<PathBuf> {
    let bin = local_bin_js(dir);
    if cfg!(windows) {
        let launcher = dir.join("dsh.cmd");
        let content = format!(
            "@echo off\r\nrem DSH launcher — generated by DSH Installer\r\n\"{}\" \"{}\" %*\r\n",
            node.display(),
            bin.display()
        );
        std::fs::write(&launcher, content)?;
        Ok(launcher)
    } else {
        let launcher = dir.join("dsh");
        let content = format!(
            "#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n",
            node.display(),
            bin.display()
        );
        std::fs::write(&launcher, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755));
        }
        Ok(launcher)
    }
}
