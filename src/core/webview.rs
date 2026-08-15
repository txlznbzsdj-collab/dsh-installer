//! 使用系统 WebView 在应用窗口内显示 DSH Web 界面。

#[cfg(windows)]
pub fn open(url: String, port: u16, child: Option<std::process::Child>) -> Result<(), String> {
    use base64::Engine;
    use std::io::Read;
    use winit::application::ApplicationHandler;
    use winit::dpi::{LogicalPosition, LogicalSize};
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{Icon, Window, WindowId};
    use wry::{Rect, WebView, WebViewBuilder};

    struct BrowserApp {
        url: String,
        window: Option<Window>,
        webview: Option<WebView>,
        error: Option<String>,
        port: u16,
        page_loaded: bool,
        started_at: std::time::Instant,
        child: Option<std::process::Child>,
        startup_failed: bool,
    }

    impl ApplicationHandler for BrowserApp {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let icon_image = image::load_from_memory(include_bytes!("../../assets/dsh-icon.png"))
                .expect("embedded DSH icon must be valid PNG")
                .into_rgba8();
            let (icon_width, icon_height) = icon_image.dimensions();
            let icon = Icon::from_rgba(icon_image.into_raw(), icon_width, icon_height).ok();
            let attributes = Window::default_attributes()
                .with_title("DSH Web")
                .with_window_icon(icon)
                .with_inner_size(LogicalSize::new(1200.0, 800.0))
                .with_min_inner_size(LogicalSize::new(900.0, 600.0))
                // WebView 完成初始化前不显示宿主窗口，避免黑白背景闪烁。
                .with_visible(false);
            let window = match event_loop.create_window(attributes) {
                Ok(window) => window,
                Err(error) => {
                    self.error = Some(format!("创建 DSH Web 窗口失败: {error}"));
                    event_loop.exit();
                    return;
                }
            };
            let size = window.inner_size().to_logical::<u32>(window.scale_factor());
            let loading_html = LOADING_HTML.replace(
                "__DSH_ICON__",
                &base64::engine::general_purpose::STANDARD
                    .encode(include_bytes!("../../assets/dsh-icon.png")),
            );
            match WebViewBuilder::new()
                .with_html(&loading_html)
                .with_background_color((255, 255, 255, 255))
                .with_bounds(Rect {
                    position: LogicalPosition::new(0, 0).into(),
                    size: LogicalSize::new(size.width, size.height).into(),
                })
                .build_as_child(&window)
            {
                Ok(webview) => {
                    self.webview = Some(webview);
                    self.window = Some(window);
                    if let Some(window) = &self.window {
                        window.set_visible(true);
                    }
                }
                Err(error) => {
                    self.error = Some(format!("加载 WebView2 失败: {error}"));
                    event_loop.exit();
                }
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _window_id: WindowId,
            event: WindowEvent,
        ) {
            match event {
                WindowEvent::Resized(size) => {
                    if let (Some(window), Some(webview)) = (&self.window, &self.webview) {
                        let size = size.to_logical::<u32>(window.scale_factor());
                        let _ = webview.set_bounds(Rect {
                            position: LogicalPosition::new(0, 0).into(),
                            size: LogicalSize::new(size.width, size.height).into(),
                        });
                    }
                }
                WindowEvent::CloseRequested => {
                    // 先隐藏宿主窗口再销毁 WebView，避免关闭动画露出黑色底层。
                    if let Some(window) = &self.window {
                        window.set_visible(false);
                    }
                    self.webview = None;
                    event_loop.exit();
                }
                _ => {}
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if !self.page_loaded && super::launch::port_is_open(self.port) {
                if let Some(webview) = &self.webview {
                    match webview.load_url(&self.url) {
                        Ok(()) => self.page_loaded = true,
                        Err(error) => self.error = Some(format!("加载 DSH 页面失败: {error}")),
                    }
                }
            } else if !self.page_loaded && !self.startup_failed {
                let exited = self
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten());
                let timed_out = self.started_at.elapsed().as_secs() >= 45;
                if exited.is_some() || timed_out {
                    self.startup_failed = true;
                    if let Some(webview) = &self.webview {
                        let message = if let Some(status) = exited {
                            let mut detail = String::new();
                            if let Some(stderr) =
                                self.child.as_mut().and_then(|child| child.stderr.as_mut())
                            {
                                let _ = stderr.read_to_string(&mut detail);
                            }
                            let detail = detail.trim();
                            let summary = if detail.is_empty() {
                                "请检查 DSH 安装后重试。".to_string()
                            } else {
                                detail.chars().take(800).collect()
                            };
                            format!(
                                "DSH 服务启动失败（进程退出码：{}）。\n\n{}",
                                status
                                    .code()
                                    .map_or_else(|| "未知".into(), |code| code.to_string()),
                                summary
                            )
                        } else {
                            "DSH 服务在 45 秒内未能启动。请关闭窗口后检查端口、网络或安装状态，再重试。"
                                .to_string()
                        };
                        let script = format!(
                            "document.querySelector('.spinner').style.display='none';\
                             document.querySelector('h2').textContent='DSH 启动失败';\
                             document.querySelector('#status').textContent={message:?};"
                        );
                        let _ = webview.evaluate_script(&script);
                    }
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now() + std::time::Duration::from_millis(250),
            ));
        }
    }

    const LOADING_HTML: &str = r#"<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
  html,body{height:100%;margin:0;background:#f7f8fc;color:#202434;font-family:"Microsoft YaHei",system-ui,sans-serif}
  body{display:grid;place-items:center}.box{text-align:center}.logo{display:block;width:72px;height:72px;margin:auto;border-radius:20px;object-fit:cover}
  .spinner{width:28px;height:28px;margin:28px auto 18px;border:3px solid #dce1fa;border-top-color:#4d6bfe;border-radius:50%;animation:spin .8s linear infinite}
  h2{margin:0 0 10px;font-size:20px}p{margin:0;color:#70778f;font-size:14px}@keyframes spin{to{transform:rotate(360deg)}}
</style>
<div class="box"><img class="logo" src="data:image/png;base64,__DSH_ICON__" alt="DSH"><div class="spinner"></div><h2>正在启动 DSH</h2><p id="status">服务就绪后将自动进入工作台…</p></div>"#;

    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut app = BrowserApp {
        url,
        window: None,
        webview: None,
        error: None,
        port,
        page_loaded: false,
        started_at: std::time::Instant::now(),
        child,
        startup_failed: false,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|error| error.to_string())?;
    app.error.map_or(Ok(()), Err)
}

#[cfg(not(windows))]
pub fn open(url: String, _port: u16, _child: Option<std::process::Child>) -> Result<(), String> {
    super::launch::open_browser(&url);
    Ok(())
}
