//! 安装/卸载过程的共享日志缓冲区：后台线程写入，UI 每帧轮询读取。

use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskStatus {
    #[default]
    Idle,
    Running,
    Success,
    Failed,
}

#[derive(Default)]
pub struct LogBuf {
    pub lines: Vec<String>,
    /// 自上次 UI 读取以来新增的行数（用于增量渲染）。
    pub new_count: usize,
    pub phase: String,
    pub status: TaskStatus,
    /// 任务成功时的结果说明（如 dsh 版本号）。
    pub summary: String,
}

pub type SharedLog = Arc<Mutex<LogBuf>>;

pub fn new_shared_log() -> SharedLog {
    Arc::new(Mutex::new(LogBuf::default()))
}

impl LogBuf {
    pub fn reset(&mut self) {
        self.lines.clear();
        self.new_count = 0;
        self.phase.clear();
        self.status = TaskStatus::Idle;
        self.summary.clear();
    }

    pub fn push(&mut self, line: String) {
        // 处理 \r 作为行分隔符（npm 进度条等），并去除 ANSI 控制序列。
        for part in line.split('\r') {
            let cleaned = strip_ansi(part);
            if cleaned.is_empty() {
                continue;
            }
            self.lines.push(cleaned);
            self.new_count += 1;
        }
    }

    pub fn set_phase(&mut self, phase: &str) {
        self.phase = phase.to_string();
    }

    pub fn finish(&mut self, ok: bool, summary: impl Into<String>) {
        self.status = if ok {
            TaskStatus::Success
        } else {
            TaskStatus::Failed
        };
        self.summary = summary.into();
    }
}

/// 去除 ANSI 转义序列（颜色码等）。
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
            continue;
        }
        if c == '\u{1b}' {
            in_escape = true;
            continue;
        }
        out.push(c);
    }
    out
}

pub fn run_in_thread<F>(log: &SharedLog, phase: &str, f: F)
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let log = Arc::clone(log);
    {
        let mut buf = log.lock().unwrap();
        buf.reset();
        buf.set_phase(phase);
        buf.status = TaskStatus::Running;
    }
    std::thread::spawn(move || {
        // 无论任务函数是正常返回还是意外 panic，都必须结束 Running 状态，
        // 否则界面会永久停留在“正在安装”。
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|payload| {
                let detail = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("未知后台异常");
                Err(format!("后台任务异常终止: {detail}"))
            });
        let mut buf = log.lock().unwrap();
        match result {
            Ok(summary) => buf.finish(true, summary),
            Err(err) => {
                buf.push(format!("[错误] {err}"));
                buf.finish(false, err);
            }
        }
    });
}
