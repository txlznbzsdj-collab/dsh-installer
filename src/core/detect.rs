//! 环境检测：定位 Node.js / npm / dsh，并解析版本。

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct EnvFacts {
    pub node_path: Option<PathBuf>,
    pub node_version: Option<String>,
    pub node_ok: bool,
    pub npm_path: Option<PathBuf>,
    pub npm_version: Option<String>,
    pub npm_prefix: Option<String>,
    /// 已全局安装的 dsh 版本（None 表示未安装）。
    pub dsh_version: Option<String>,
    pub dsh_cmd: Option<PathBuf>,
}

impl EnvFacts {
    /// Node 主版本号。
    pub fn node_major(&self) -> Option<u32> {
        self.node_version
            .as_deref()
            .and_then(parse_version)
            .map(|(m, _, _)| m)
    }

    pub fn node_requirement_note(&self) -> String {
        match self.node_major() {
            None => {
                "未检测到 Node.js，请先安装 Node.js 18 或更高版本 (https://nodejs.org)".to_string()
            }
            Some(m) if m < 18 => {
                format!(
                    "Node.js 版本过低 (v{}，需要 18+)，请升级 Node.js 后再安装 DSH",
                    self.node_version.as_deref().unwrap_or("?")
                )
            }
            Some(m) if m < 20 => format!(
                "Node.js v{} 可运行 DSH，但推荐使用 20+ 版本",
                self.node_version.as_deref().unwrap_or("?")
            ),
            Some(_) => format!(
                "Node.js {} ✓ 满足要求",
                self.node_version.as_deref().unwrap_or("?")
            ),
        }
    }
}

/// 运行命令并捕获输出。
pub fn run_capture(program: &Path, args: &[&str]) -> Result<(String, String, i32), String> {
    let lower = program.to_string_lossy().to_ascii_lowercase();
    let mut command = if cfg!(windows) && (lower.ends_with(".cmd") || lower.ends_with(".bat")) {
        let mut command = Command::new("cmd");
        command.arg("/d").arg("/c").arg(program);
        command
    } else {
        Command::new(program)
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let out = command
        .args(args)
        .output()
        .map_err(|e| format!("无法运行 {}: {e}", program.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Ok((stdout, stderr, out.status.code().unwrap_or(-1)))
}

/// 解析 "v24.13.0" / "24.13.0" → (24, 13, 0)。
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim().trim_start_matches('v');
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn is_windows() -> bool {
    cfg!(windows)
}

/// 在 PATH 中查找可执行文件。
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let names = if is_windows() && Path::new(name).extension().is_none() {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
        ]
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(&path_var) {
        for name in &names {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

fn node_common_locations() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(prog) = std::env::var("ProgramFiles") {
        v.push(PathBuf::from(prog).join("nodejs\\node.exe"));
    }
    if let Ok(prog) = std::env::var("ProgramFiles(x86)") {
        v.push(PathBuf::from(prog).join("nodejs\\node.exe"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        v.push(PathBuf::from(local).join("Programs\\nodejs\\node.exe"));
    }
    v.push(PathBuf::from(r"C:\nodejs\node.exe"));
    v
}

/// 定位 node 可执行文件。
pub fn find_node() -> Option<PathBuf> {
    if let Some(p) = find_on_path("node") {
        return Some(p);
    }
    node_common_locations().into_iter().find(|p| p.is_file())
}

/// 给定 node 路径，定位 npm。
fn find_npm(node_path: &Path) -> Option<PathBuf> {
    if is_windows() {
        // 官方安装布局：npm.cmd 与 node.exe 同目录。
        let cand = node_path.parent()?.join("npm.cmd");
        if cand.is_file() {
            return Some(cand);
        }
    }
    find_on_path(if is_windows() { "npm.cmd" } else { "npm" })
}

/// 执行完整环境检测。
pub fn detect() -> EnvFacts {
    let mut facts = EnvFacts::default();

    // Node.js
    if let Some(node) = find_node() {
        facts.node_path = Some(node.clone());
        if let Ok((out, _err, code)) = run_capture(&node, &["--version"])
            && code == 0
            && !out.is_empty()
        {
            facts.node_version = Some(out);
        }
        if let Some(major) = facts.node_major() {
            facts.node_ok = major >= 18;
        }
        // npm
        if let Some(npm) = find_npm(&node) {
            facts.npm_path = Some(npm.clone());
            if let Ok((out, _err, code)) = run_capture(&npm, &["--version"])
                && code == 0
                && !out.is_empty()
            {
                facts.npm_version = Some(out);
            }
            if let Ok((out, _err, code)) = run_capture(&npm, &["prefix", "-g"])
                && code == 0
                && !out.is_empty()
            {
                facts.npm_prefix = Some(out);
            }
        }
    } else if let Some(npm) = find_on_path(if is_windows() { "npm.cmd" } else { "npm" }) {
        // 罕见情况：有 npm 但没有 node 在 PATH 上
        facts.npm_path = Some(npm);
    }

    // dsh（全局安装检测）
    if let Some(dsh) = find_on_path("dsh")
        && let Ok((out, _err, code)) = run_capture(&dsh, &["-V"])
        && code == 0
        && !out.is_empty()
    {
        facts.dsh_version = Some(out);
        facts.dsh_cmd = Some(dsh);
    }

    facts
}
