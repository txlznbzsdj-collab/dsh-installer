//! DSH 安装助手 — 四步向导界面（欢迎 / 环境检测 / 安装 / 完成）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::core::detect::{self, EnvFacts};
use crate::core::log::{SharedLog, TaskStatus, new_shared_log, run_in_thread};
use crate::core::npm::{self, DEFAULT_PORT, InstallMode};
use crate::core::shortcut;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4D, 0x6B, 0xFE);
const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(0x3E, 0xB5, 0x7F);
const WARN_ORANGE: egui::Color32 = egui::Color32::from_rgb(0xF0, 0xA0, 0x3C);
const ERR_RED: egui::Color32 = egui::Color32::from_rgb(0xE5, 0x4B, 0x4B);
const FOOTER_BUTTON_SIZE: egui::Vec2 = egui::vec2(124.0, 36.0);
const ACTION_BUTTON_SIZE: egui::Vec2 = egui::vec2(136.0, 38.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Welcome,
    Check,
    Install,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageAction {
    Install,
    Uninstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageSource {
    Online,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistryChoice {
    Official,
    Mirror,
    Custom,
}

pub struct DshInstallerApp {
    page: Page,

    // ── 环境检测 ──
    detecting: bool,
    detect_result: Arc<Mutex<Option<EnvFacts>>>,
    facts: Option<EnvFacts>,

    // ── 安装 ──
    mode: InstallMode,
    local_dir: String,
    port: String,
    package_source: PackageSource,
    version: String,
    registry_choice: RegistryChoice,
    custom_registry: String,
    offline_package: String,
    advanced_options: bool,
    install_log: SharedLog,
    /// 实际完成安装所使用的模式（用于卸载 / 快捷方式）。
    done_mode: Option<InstallMode>,
    /// 本地模式安装成功后生成的启动器路径。
    local_launcher: Option<PathBuf>,
    package_action: PackageAction,

    // ── 完成 ──
    create_shortcut: bool,
    launch_web: bool,
    finish_log: SharedLog,
    finish_done: bool,
}

impl DshInstallerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_fonts(&cc.egui_ctx);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.window_fill = egui::Color32::from_rgb(0x14, 0x17, 0x22);
        style.visuals.panel_fill = egui::Color32::from_rgb(0x14, 0x17, 0x22);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(0x0E, 0x11, 0x1A);
        style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0x1E, 0x24, 0x33);
        style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x22, 0x2A, 0x3C);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x2B, 0x35, 0x4B);
        style.visuals.widgets.active.bg_fill = ACCENT;
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        cc.egui_ctx.set_style(style);

        Self {
            page: Page::Welcome,
            detecting: false,
            detect_result: Arc::new(Mutex::new(None)),
            facts: None,
            mode: InstallMode::Global,
            local_dir: String::from(r"C:\dsh"),
            port: DEFAULT_PORT.to_string(),
            package_source: PackageSource::Online,
            version: String::from("latest"),
            registry_choice: RegistryChoice::Official,
            custom_registry: String::new(),
            offline_package: String::new(),
            advanced_options: false,
            install_log: new_shared_log(),
            done_mode: None,
            local_launcher: None,
            package_action: PackageAction::Install,
            create_shortcut: true,
            launch_web: true,
            finish_log: new_shared_log(),
            finish_done: false,
        }
    }

    // ───────────────────────── 动作 ─────────────────────────

    fn start_detect(&mut self) {
        self.detecting = true;
        self.facts = None;
        let result = Arc::clone(&self.detect_result);
        std::thread::spawn(move || {
            let facts = detect::detect();
            std::thread::sleep(std::time::Duration::from_millis(200));
            *result.lock().unwrap() = Some(facts);
        });
    }

    fn poll_detect(&mut self) {
        if !self.detecting {
            return;
        }
        if let Some(facts) = self.detect_result.lock().unwrap().take() {
            self.facts = Some(facts);
            self.detecting = false;
        }
    }

    fn install_valid(&self) -> bool {
        let source_valid = match self.package_source {
            PackageSource::Online => {
                !self.version.trim().is_empty()
                    && (self.registry_choice != RegistryChoice::Custom
                        || self.custom_registry.trim().starts_with("http"))
            }
            PackageSource::Offline => {
                let path = Path::new(self.offline_package.trim());
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("tgz"))
            }
        };
        self.facts
            .as_ref()
            .map(|f| f.node_ok && f.npm_path.is_some())
            .unwrap_or(false)
            && self.port.parse::<u16>().is_ok()
            && source_valid
    }

    fn start_install(&mut self) {
        let Some(facts) = self.facts.clone() else {
            return;
        };
        let Some(node) = facts.node_path.clone() else {
            return;
        };
        let Some(npm) = facts.npm_path.clone() else {
            return;
        };
        if !facts.node_ok {
            return;
        }
        self.package_action = PackageAction::Install;
        let npm_prefix = facts.npm_prefix.as_deref().map(PathBuf::from);
        let mode = self.mode.clone();
        let package_spec = match self.package_source {
            PackageSource::Online => {
                format!("{}@{}", npm::DSH_PACKAGE, self.version.trim())
            }
            PackageSource::Offline => self.offline_package.trim().to_string(),
        };
        let registry = match self.package_source {
            PackageSource::Offline => None,
            PackageSource::Online => match self.registry_choice {
                RegistryChoice::Official => Some("https://registry.npmjs.org".to_string()),
                RegistryChoice::Mirror => Some("https://registry.npmmirror.com".to_string()),
                RegistryChoice::Custom => Some(self.custom_registry.trim().to_string()),
            },
        };
        let log = Arc::clone(&self.install_log);
        let log_thread = Arc::clone(&log);
        run_in_thread(&log, "正在通过 npm 安装 DSH…", move || {
            let version = npm::install_dsh_with_options(
                &log_thread,
                &npm,
                &node,
                npm_prefix.as_deref(),
                &mode,
                &package_spec,
                registry.as_deref(),
            )?;
            if let InstallMode::Local(dir) = &mode {
                // 生成便携启动器
                let launcher = npm::write_local_launcher(dir, &node).map_err(|e| e.to_string())?;
                log_thread
                    .lock()
                    .unwrap()
                    .push(format!("已生成启动器 {}", launcher.display()));
            }
            Ok(version)
        });
    }

    fn start_uninstall(&mut self) {
        let Some(facts) = self.facts.clone() else {
            return;
        };
        let Some(npm) = facts.npm_path.clone() else {
            return;
        };
        // 优先卸载“实际安装”的模式；否则若检测到全局已装则卸载全局。
        let mode = self
            .done_mode
            .clone()
            .or_else(|| facts.dsh_version.map(|_| InstallMode::Global));
        let Some(mode) = mode else { return };
        self.package_action = PackageAction::Uninstall;
        let log = Arc::clone(&self.install_log);
        let log_thread = Arc::clone(&log);
        run_in_thread(&log, "正在卸载 DSH…", move || {
            let result = npm::uninstall_dsh(&log_thread, &npm, &mode);
            // 顺带删除「DSH Web」桌面快捷方式
            shortcut::remove_desktop_shortcut(shortcut::SHORTCUT_NAME);
            result
        });
    }

    fn finish_actions(&mut self) {
        let Some(facts) = self.facts.clone() else {
            self.finish_done = true;
            return;
        };
        let done_mode = self.done_mode.clone();
        let create_shortcut = self.create_shortcut;
        let launch_web = self.launch_web;
        let port: u16 = self.port.parse().unwrap_or(DEFAULT_PORT);
        let local_launcher = self.local_launcher.clone();
        let finish_log = Arc::clone(&self.finish_log);
        self.finish_done = false;

        std::thread::spawn(move || {
            let mut buf = finish_log.lock().unwrap();
            buf.reset();
            buf.status = TaskStatus::Running;
            buf.set_phase("正在完成安装配置…");

            // 1. 桌面快捷方式
            if create_shortcut {
                match shortcut_target(&facts, done_mode.as_ref(), local_launcher.as_deref()) {
                    Ok(dsh_target) => {
                        let installer = std::env::current_exe();
                        let shortcut_args = format!(
                            "--launch-web --entry \"{}\" --port {port}",
                            dsh_target.display()
                        );
                        let target = installer.as_deref().unwrap_or(&dsh_target);
                        let args = if installer.is_ok() {
                            shortcut_args.as_str()
                        } else {
                            "web"
                        };
                        match shortcut::create_desktop_shortcut(
                            shortcut::SHORTCUT_NAME,
                            target,
                            args,
                            Some(target),
                        ) {
                            Ok(lnk) => buf.push(format!("✓ 已创建桌面快捷方式 {}", lnk.display())),
                            Err(e) => buf.push(format!("⚠ 创建快捷方式失败: {e}")),
                        }
                    }
                    Err(e) => buf.push(format!("⚠ {e}")),
                }
            }

            // 2. 启动 Web 界面
            if launch_web {
                match dsh_entry(&facts, done_mode.as_ref(), local_launcher.as_deref()) {
                    Some((program, prefix_args)) => {
                        let _ = prefix_args;
                        match std::env::current_exe().and_then(|exe| {
                            std::process::Command::new(exe)
                                .arg("--launch-web")
                                .arg("--entry")
                                .arg(&program)
                                .arg("--port")
                                .arg(port.to_string())
                                .spawn()
                        }) {
                            Ok(_) => buf.push(format!(
                                "✓ 正在程序内打开 DSH Web (http://127.0.0.1:{port})"
                            )),
                            Err(e) => buf.push(format!("⚠ 打开 DSH Web 窗口失败: {e}")),
                        }
                    }
                    None => buf.push("⚠ 未找到 dsh 入口，无法启动 Web 界面".to_string()),
                }
            }

            buf.set_phase("完成");
            buf.finish(true, "");
        });
    }

    // ───────────────────────── 顶栏 / 底栏 ─────────────────────────

    fn render_top(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("DSH 安装助手")
                        .size(20.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("DeepSeek Harness 一键安装")
                        .size(11.0)
                        .color(egui::Color32::GRAY),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let steps = ["① 欢迎", "② 环境检测", "③ 安装", "④ 完成"];
                let current = match self.page {
                    Page::Welcome => 0,
                    Page::Check => 1,
                    Page::Install => 2,
                    Page::Finish => 3,
                };
                for (i, s) in steps.iter().enumerate().rev() {
                    let color = if i == current {
                        ACCENT
                    } else if i < current {
                        OK_GREEN
                    } else {
                        egui::Color32::from_gray(90)
                    };
                    ui.label(egui::RichText::new(*s).size(13.0).strong().color(color));
                    if i > 0 {
                        ui.label(egui::RichText::new("›").color(egui::Color32::from_gray(70)));
                    }
                }
            });
        });
        ui.add_space(4.0);
    }

    fn render_footer(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("DSH 安装助手 v0.1.0 · 基于 npm 安装 @deepseek-ai/dsh · MIT")
                    .size(11.0)
                    .color(egui::Color32::from_gray(100)),
            );
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match self.page {
                    Page::Welcome => {
                        if ui
                            .add_sized(FOOTER_BUTTON_SIZE, egui::Button::new("退出"))
                            .clicked()
                        {
                            std::process::exit(0);
                        }
                    }
                    Page::Check => {
                        let ready = self
                            .facts
                            .as_ref()
                            .map(|f| f.node_ok && f.npm_path.is_some())
                            .unwrap_or(false);
                        if ui
                            .add_enabled(
                                ready,
                                egui::Button::new(egui::RichText::new("继续安装 ›").strong())
                                    .min_size(FOOTER_BUTTON_SIZE)
                                    .fill(ACCENT),
                            )
                            .clicked()
                        {
                            self.page = Page::Install;
                        }
                        if ui
                            .add_sized(FOOTER_BUTTON_SIZE, egui::Button::new("‹ 上一步"))
                            .clicked()
                        {
                            self.page = Page::Welcome;
                        }
                    }
                    Page::Install => {
                        let can_next = (self.install_log.lock().unwrap().status
                            == TaskStatus::Success
                            && self.package_action == PackageAction::Install)
                            || self
                                .facts
                                .as_ref()
                                .and_then(|f| f.dsh_version.as_ref())
                                .is_some();
                        if ui
                            .add_enabled(
                                can_next,
                                egui::Button::new(egui::RichText::new("继续 ›").strong())
                                    .min_size(FOOTER_BUTTON_SIZE)
                                    .fill(ACCENT),
                            )
                            .clicked()
                        {
                            self.page = Page::Finish;
                        }
                        if ui
                            .add_sized(FOOTER_BUTTON_SIZE, egui::Button::new("‹ 上一步"))
                            .clicked()
                        {
                            self.page = Page::Check;
                        }
                    }
                    Page::Finish => {
                        if ui
                            .add_sized(FOOTER_BUTTON_SIZE, egui::Button::new("退出"))
                            .clicked()
                        {
                            std::process::exit(0);
                        }
                    }
                },
            );
        });
        ui.add_space(4.0);
    }

    // ───────────────────────── 各页面 ─────────────────────────

    fn render_welcome(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.add_space(18.0);
            ui.label(
                egui::RichText::new("🛠 欢迎使用 DSH 安装助手")
                    .size(30.0)
                    .strong()
                    .color(egui::Color32::WHITE),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "DeepSeek Harness (DSH) — DeepSeek 推出的智能体开发框架，现已公测",
                )
                .size(15.0)
                .color(egui::Color32::from_gray(180)),
            );
            ui.add_space(24.0);

            egui::Frame::group(ui.style())
                .fill(egui::Color32::from_rgb(0x1A, 0x20, 0x2E))
                .inner_margin(egui::Margin::symmetric(28, 18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 40.0);
                    ui.label(
                        egui::RichText::new("本助手将为您完成：")
                            .size(15.0)
                            .strong()
                            .color(ACCENT),
                    );
                    ui.add_space(6.0);
                    for line in [
                        "① 检测 Node.js / npm 运行环境",
                        "② 通过 npm 一键安装官方 @deepseek-ai/dsh",
                        "③ 创建「DSH Web」桌面快捷方式",
                        "④ 启动 DSH Web 图形界面 (http://127.0.0.1:3080)",
                    ] {
                        ui.label(egui::RichText::new(format!("  {line}")).size(14.0));
                    }
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(
                            "环境要求：Node.js 18 及以上（推荐 20+）· Windows / macOS / Linux",
                        )
                        .size(12.0)
                        .color(egui::Color32::from_gray(140)),
                    );
                });
            ui.add_space(28.0);
            if ui
                .add_sized(
                    [240.0, 44.0],
                    egui::Button::new(egui::RichText::new("开始检测环境 ›").size(16.0).strong()),
                )
                .clicked()
            {
                self.page = Page::Check;
                self.start_detect();
            }
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(
                    "DSH 由 DeepSeek 官方发布，MIT 协议 · 本质为 npm 安装 @deepseek-ai/dsh",
                )
                .size(11.0)
                .color(egui::Color32::from_gray(110)),
            );
        });
    }

    fn render_check(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading(egui::RichText::new("环境检测").color(egui::Color32::WHITE));
        ui.label(
            egui::RichText::new("正在检查您的电脑是否满足运行 DSH 的要求…")
                .color(egui::Color32::from_gray(160)),
        );
        ui.add_space(10.0);

        self.poll_detect();

        if self.detecting {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.add(egui::Spinner::new().size(36.0));
                ui.add_space(8.0);
                ui.label(egui::RichText::new("检测中…").size(15.0));
            });
        } else if let Some(facts) = &self.facts {
            render_fact_row(
                ui,
                "Node.js",
                facts
                    .node_version
                    .as_deref()
                    .map(|v| format!("v{}", v.trim_start_matches('v'))),
                facts.node_ok,
                "必需",
            );
            render_fact_row(
                ui,
                "npm",
                facts.npm_version.clone(),
                facts.npm_version.is_some(),
                "必需",
            );
            render_fact_row(
                ui,
                "DSH (已安装)",
                facts.dsh_version.clone(),
                facts.dsh_version.is_some(),
                "可选",
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("ℹ {}", facts.node_requirement_note()))
                    .size(13.0)
                    .color(if facts.node_ok {
                        egui::Color32::from_gray(160)
                    } else {
                        WARN_ORANGE
                    }),
            );
            if !facts.node_ok {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "请先前往 https://nodejs.org 安装 Node.js 18+，然后点击「重新检测」。",
                    )
                    .size(13.0)
                    .color(ERR_RED),
                );
            }
            ui.add_space(12.0);
            if ui
                .add_sized(ACTION_BUTTON_SIZE, egui::Button::new("↻ 重新检测"))
                .clicked()
            {
                self.start_detect();
            }
        }
    }

    fn render_install(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading(egui::RichText::new("安装 DSH").color(egui::Color32::WHITE));
        ui.label(
            egui::RichText::new(
                "选择安装方式，然后点击「开始安装」。安装过程通过网络下载，请保持网络畅通。",
            )
            .color(egui::Color32::from_gray(160)),
        );
        ui.add_space(10.0);

        if ui
            .checkbox(&mut self.advanced_options, "高级安装选项")
            .changed()
            && !self.advanced_options
        {
            self.package_source = PackageSource::Online;
            self.version = "latest".to_string();
            self.registry_choice = RegistryChoice::Official;
        }
        if !self.advanced_options {
            ui.label(
                egui::RichText::new("默认使用 npm 官方源安装最新版本")
                    .size(11.0)
                    .color(egui::Color32::from_gray(130)),
            );
        }
        ui.add_space(6.0);

        let status = self.install_log.lock().unwrap().status;

        egui::Grid::new("install_opts")
            .num_columns(2)
            .spacing([12.0, 10.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("安装方式").strong());
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            self.mode == InstallMode::Global,
                            egui::RichText::new("全局安装 (推荐)").size(14.0),
                        )
                        .clicked()
                    {
                        self.mode = InstallMode::Global;
                    }
                    if ui
                        .selectable_label(
                            matches!(self.mode, InstallMode::Local(_)),
                            egui::RichText::new("本地便携安装").size(14.0),
                        )
                        .clicked()
                    {
                        self.mode = InstallMode::Local(PathBuf::from(self.local_dir.clone()));
                    }
                    if self.mode == InstallMode::Global
                        && let Some(prefix) =
                            &self.facts.as_ref().and_then(|f| f.npm_prefix.clone())
                    {
                        ui.label(
                            egui::RichText::new(format!("→ {prefix}"))
                                .size(11.0)
                                .color(egui::Color32::from_gray(130)),
                        );
                    }
                });
                ui.end_row();

                if self.advanced_options {
                    ui.label(egui::RichText::new("软件包来源").strong());
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.package_source,
                            PackageSource::Online,
                            "在线安装",
                        );
                        ui.selectable_value(
                            &mut self.package_source,
                            PackageSource::Offline,
                            "本地 .tgz 安装包",
                        );
                    });
                    ui.end_row();

                    match self.package_source {
                        PackageSource::Online => {
                            ui.label(egui::RichText::new("版本").strong());
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.version)
                                        .desired_width(150.0)
                                        .hint_text("latest 或 0.1.0-rc.6"),
                                );
                                ui.label(
                                    egui::RichText::new("latest、next 或指定版本号")
                                        .size(11.0)
                                        .color(egui::Color32::from_gray(130)),
                                );
                            });
                            ui.end_row();

                            ui.label(egui::RichText::new("npm 下载源").strong());
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("registry_choice")
                                    .selected_text(match self.registry_choice {
                                        RegistryChoice::Official => "npm 官方源",
                                        RegistryChoice::Mirror => "npmmirror 国内镜像",
                                        RegistryChoice::Custom => "自定义源",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.registry_choice,
                                            RegistryChoice::Official,
                                            "npm 官方源",
                                        );
                                        ui.selectable_value(
                                            &mut self.registry_choice,
                                            RegistryChoice::Mirror,
                                            "npmmirror 国内镜像",
                                        );
                                        ui.selectable_value(
                                            &mut self.registry_choice,
                                            RegistryChoice::Custom,
                                            "自定义源",
                                        );
                                    });
                                if self.registry_choice == RegistryChoice::Custom {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.custom_registry)
                                            .desired_width(260.0)
                                            .hint_text("https://registry.example.com"),
                                    );
                                }
                            });
                            ui.end_row();
                        }
                        PackageSource::Offline => {
                            ui.label(egui::RichText::new("离线安装包").strong());
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.offline_package)
                                        .desired_width(330.0)
                                        .hint_text("选择 deepseek-ai-dsh-*.tgz"),
                                );
                                if ui.button("选择文件…").clicked()
                                    && let Some(file) = rfd::FileDialog::new()
                                        .set_title("选择 DSH 离线安装包")
                                        .add_filter("npm 安装包", &["tgz"])
                                        .pick_file()
                                {
                                    self.offline_package = file.to_string_lossy().to_string();
                                }
                            });
                            ui.end_row();
                        }
                    }
                }

                ui.label(egui::RichText::new("安装目录").strong());
                match &self.mode {
                    InstallMode::Local(_) => {
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.local_dir)
                                    .desired_width(300.0)
                                    .hint_text("选择安装目录，如 C:\\dsh"),
                            );
                            if resp.changed() {
                                self.mode =
                                    InstallMode::Local(PathBuf::from(self.local_dir.clone()));
                            }
                            if ui.button("浏览…").clicked()
                                && let Some(dir) = rfd::FileDialog::new()
                                    .set_title("选择 DSH 安装目录")
                                    .pick_folder()
                            {
                                self.local_dir = dir.to_string_lossy().to_string();
                                self.mode = InstallMode::Local(dir);
                            }
                        });
                    }
                    InstallMode::Global => {
                        ui.label(
                            egui::RichText::new(
                                "安装到 npm 全局目录，dsh 命令可直接使用（无需管理员权限）",
                            )
                            .color(egui::Color32::from_gray(150)),
                        );
                    }
                }
                ui.end_row();

                ui.label(egui::RichText::new("Web 端口").strong());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.port)
                            .desired_width(80.0)
                            .hint_text("3080"),
                    );
                    ui.label(
                        egui::RichText::new("DSH Web 界面默认端口 (http://127.0.0.1:3080)")
                            .size(11.0)
                            .color(egui::Color32::from_gray(130)),
                    );
                });
                ui.end_row();
            });

        ui.add_space(10.0);

        // 已全局安装提示
        if self.done_mode.is_none()
            && let Some(v) = &self.facts.as_ref().and_then(|f| f.dsh_version.clone())
        {
            egui::Frame::group(ui.style())
                .fill(egui::Color32::from_rgb(0x1A, 0x26, 0x1F))
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "✓ 检测到已全局安装 dsh {v}，可直接进入下一步。"
                        ))
                        .color(OK_GREEN),
                    );
                });
            ui.add_space(8.0);
        }

        // 操作按钮
        ui.horizontal(|ui| match status {
            TaskStatus::Idle | TaskStatus::Success | TaskStatus::Failed => {
                let label = if status == TaskStatus::Success
                    && self.package_action == PackageAction::Install
                {
                    "↻ 重新安装"
                } else {
                    "开始安装"
                };
                if ui
                    .add_enabled(
                        self.install_valid(),
                        egui::Button::new(egui::RichText::new(label).size(15.0).strong())
                            .min_size(ACTION_BUTTON_SIZE)
                            .fill(ACCENT),
                    )
                    .clicked()
                {
                    self.start_install();
                }
                let has_installed = self.done_mode.is_some()
                    || self
                        .facts
                        .as_ref()
                        .and_then(|f| f.dsh_version.clone())
                        .is_some();
                if ui
                    .add_enabled(
                        has_installed && status != TaskStatus::Running,
                        egui::Button::new("卸载").min_size(egui::vec2(88.0, 38.0)),
                    )
                    .clicked()
                {
                    self.start_uninstall();
                }
            }
            TaskStatus::Running => {
                ui.add(egui::Spinner::new().size(20.0));
                ui.label(
                    egui::RichText::new(self.install_log.lock().unwrap().phase.clone())
                        .size(14.0)
                        .color(ACCENT),
                );
            }
        });

        ui.add_space(8.0);

        // 结果横幅 + 记录安装结果
        match status {
            TaskStatus::Success => {
                let summary = self.install_log.lock().unwrap().summary.clone();
                if self.package_action == PackageAction::Uninstall {
                    ui.label(
                        egui::RichText::new("✓ DSH 已卸载")
                            .size(15.0)
                            .strong()
                            .color(OK_GREEN),
                    );
                    self.done_mode = None;
                    self.local_launcher = None;
                    if let Some(facts) = &mut self.facts {
                        facts.dsh_version = None;
                        facts.dsh_cmd = None;
                    }
                    ui.add_space(6.0);
                    return;
                }
                ui.label(
                    egui::RichText::new(format!("✓ 安装成功：dsh {summary}"))
                        .size(15.0)
                        .strong()
                        .color(OK_GREEN),
                );
                if self.done_mode.is_none() {
                    let mode = self.mode.clone();
                    if let InstallMode::Local(dir) = &mode {
                        // 安装线程已生成启动器，路径是确定性的
                        let launcher = dir.join(if cfg!(windows) { "dsh.cmd" } else { "dsh" });
                        self.local_launcher = Some(launcher);
                    }
                    self.done_mode = Some(mode);
                    if let Some(facts) = &mut self.facts {
                        facts.dsh_version = Some(summary);
                    }
                }
                ui.add_space(6.0);
            }
            TaskStatus::Failed => {
                let summary = self.install_log.lock().unwrap().summary.clone();
                ui.label(
                    egui::RichText::new(format!("✗ {summary}"))
                        .size(15.0)
                        .strong()
                        .color(ERR_RED),
                );
                ui.add_space(6.0);
            }
            _ => {}
        }

        // 日志区
        let lines = {
            let buf = self.install_log.lock().unwrap();
            buf.lines.clone()
        };
        egui::Frame::group(ui.style())
            .fill(egui::Color32::from_rgb(0x0C, 0x0F, 0x16))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_min_height(150.0);
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if lines.is_empty() {
                            ui.label(
                                egui::RichText::new("日志输出将显示在这里…")
                                    .monospace()
                                    .color(egui::Color32::from_gray(90)),
                            );
                        }
                        let start = lines.len().saturating_sub(2000);
                        for line in &lines[start..] {
                            ui.label(
                                egui::RichText::new(line.as_str())
                                    .monospace()
                                    .size(12.0)
                                    .color(egui::Color32::from_gray(190)),
                            );
                        }
                    });
            });
    }

    fn render_finish(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading(egui::RichText::new("完成").color(egui::Color32::WHITE));
        ui.add_space(10.0);

        let version = self
            .facts
            .as_ref()
            .and_then(|f| f.dsh_version.clone())
            .unwrap_or_else(|| "未知".to_string());
        let mode_label = self
            .done_mode
            .as_ref()
            .map(|m| m.label())
            .or_else(|| {
                self.facts
                    .as_ref()
                    .and_then(|f| f.dsh_version.as_ref())
                    .map(|_| "全局安装".to_string())
            })
            .unwrap_or_else(|| "未安装".to_string());

        egui::Frame::group(ui.style())
            .fill(egui::Color32::from_rgb(0x1A, 0x20, 0x2E))
            .inner_margin(egui::Margin::symmetric(20, 14))
            .show(ui, |ui| {
                egui::Grid::new("summary")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("DSH 版本").strong());
                        ui.label(egui::RichText::new(&version).color(OK_GREEN));
                        ui.end_row();
                        ui.label(egui::RichText::new("安装方式").strong());
                        ui.label(&mode_label);
                        ui.end_row();
                        ui.label(egui::RichText::new("Web 地址").strong());
                        let port = self.port.parse::<u16>().unwrap_or(DEFAULT_PORT);
                        ui.label(format!("http://127.0.0.1:{port}"));
                        ui.end_row();
                    });
            });

        ui.add_space(14.0);
        ui.checkbox(&mut self.create_shortcut, "创建「DSH Web」桌面快捷方式");
        ui.add_space(4.0);
        let port = self.port.parse::<u16>().unwrap_or(DEFAULT_PORT);
        ui.checkbox(
            &mut self.launch_web,
            format!("安装完成后启动 DSH Web 界面 (http://127.0.0.1:{port})"),
        );

        ui.add_space(12.0);
        if !self.finish_done {
            let running = self.finish_log.lock().unwrap().status == TaskStatus::Running;
            if ui
                .add_enabled(
                    !running,
                    egui::Button::new(egui::RichText::new("完成").size(15.0).strong())
                        .min_size(ACTION_BUTTON_SIZE)
                        .fill(ACCENT),
                )
                .clicked()
            {
                self.finish_actions();
            }
        }

        // 完成动作日志
        let (lines, status) = {
            let buf = self.finish_log.lock().unwrap();
            (buf.lines.clone(), buf.status)
        };
        if !lines.is_empty() || status != TaskStatus::Idle {
            ui.add_space(8.0);
            for line in &lines {
                let color = if line.starts_with('✓') {
                    OK_GREEN
                } else if line.starts_with('⚠') {
                    WARN_ORANGE
                } else {
                    egui::Color32::from_gray(190)
                };
                ui.label(egui::RichText::new(line.as_str()).color(color));
            }
            if status == TaskStatus::Success || status == TaskStatus::Failed {
                self.finish_done = true;
            }
        }
    }
}

/// 根据安装模式计算桌面快捷方式的目标程序。
fn shortcut_target(
    facts: &EnvFacts,
    done_mode: Option<&InstallMode>,
    local_launcher: Option<&Path>,
) -> Result<PathBuf, String> {
    match done_mode {
        Some(InstallMode::Local(_)) => {
            if let Some(l) = local_launcher
                && l.is_file()
            {
                return Ok(l.to_path_buf());
            }
            Err("未找到本地启动器".to_string())
        }
        _ => {
            // 全局：dsh.cmd 位于 npm 全局 bin 目录
            if let Some(prefix) = &facts.npm_prefix {
                for cand in [
                    PathBuf::from(prefix).join("dsh.cmd"),
                    PathBuf::from(prefix).join("dsh"),
                ] {
                    if cand.is_file() {
                        return Ok(cand);
                    }
                }
            }
            if let Some(cmd) = &facts.dsh_cmd {
                return Ok(cmd.clone());
            }
            Err("未找到全局 dsh 命令，无法创建快捷方式".to_string())
        }
    }
}

/// 计算启动 dsh web 的入口。
fn dsh_entry(
    facts: &EnvFacts,
    done_mode: Option<&InstallMode>,
    local_launcher: Option<&Path>,
) -> Option<(PathBuf, Vec<String>)> {
    match done_mode {
        Some(InstallMode::Local(_)) => local_launcher.map(|l| (l.to_path_buf(), vec![])),
        _ => {
            if let Some(prefix) = &facts.npm_prefix {
                for cand in [
                    PathBuf::from(prefix).join("dsh.cmd"),
                    PathBuf::from(prefix).join("dsh"),
                ] {
                    if cand.is_file() {
                        return Some((cand, vec![]));
                    }
                }
            }
            if let Some(cmd) = &facts.dsh_cmd {
                return Some((cmd.clone(), vec![]));
            }
            None
        }
    }
}

fn render_fact_row(ui: &mut egui::Ui, name: &str, value: Option<String>, ok: bool, tag: &str) {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let icon = if ok { "✓" } else { "✗" };
        let color = if ok { OK_GREEN } else { ERR_RED };
        ui.label(egui::RichText::new(icon).size(18.0).strong().color(color));
        ui.label(egui::RichText::new(name).size(14.0).strong());
        match value {
            Some(v) => {
                ui.label(
                    egui::RichText::new(v)
                        .size(14.0)
                        .color(egui::Color32::from_gray(200)),
                );
            }
            None => {
                ui.label(
                    egui::RichText::new("未检测到")
                        .size(14.0)
                        .color(egui::Color32::from_gray(120)),
                );
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(tag)
                    .size(11.0)
                    .color(egui::Color32::from_gray(110)),
            );
        });
    });
}

/// 加载系统中文字体（egui 默认字体不含 CJK 字符）。
fn install_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Windows\Fonts\msyh.ttc",   // 微软雅黑
            r"C:\Windows\Fonts\msyhbd.ttc", // 微软雅黑粗体
            r"C:\Windows\Fonts\simhei.ttf", // 黑体
            r"C:\Windows\Fonts\simsun.ttc", // 宋体
            r"C:\Windows\Fonts\Deng.ttf",   // 等线
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ]
    };

    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("cjk".to_owned());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

impl eframe::App for DshInstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 后台检测不会产生窗口事件；定时唤醒 UI，确保结果能及时显示。
        if self.detecting {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let install_running = self.install_log.lock().unwrap().status == TaskStatus::Running;
        let finish_running = self.finish_log.lock().unwrap().status == TaskStatus::Running;
        if install_running || finish_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 10)))
            .show(ctx, |ui| {
                self.render_top(ui);
            });

        egui::TopBottomPanel::bottom("footer")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 4)))
            .show(ctx, |ui| {
                self.render_footer(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(28, 8)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Welcome => self.render_welcome(ui),
                        Page::Check => self.render_check(ui),
                        Page::Install => self.render_install(ui),
                        Page::Finish => self.render_finish(ui),
                    });
            });
    }
}
