//! 创建桌面快捷方式（Windows 直接调用 IShellLink，Linux 使用 .desktop 文件）。

use std::path::{Path, PathBuf};

pub const SHORTCUT_NAME: &str = "DSH Web";

pub fn desktop_dir() -> Option<PathBuf> {
    dirs::desktop_dir()
}

/// 创建桌面快捷方式，返回快捷方式路径。
pub fn create_desktop_shortcut(
    name: &str,
    target: &Path,
    args: &str,
    icon: Option<&Path>,
) -> Result<PathBuf, String> {
    let desktop = desktop_dir().ok_or("无法定位桌面目录")?;
    if cfg!(windows) {
        create_windows_lnk(&desktop, name, target, args, icon)
    } else if cfg!(target_os = "linux") {
        create_linux_desktop(&desktop, name, target, args)
    } else {
        Err("当前系统暂不支持自动创建快捷方式".to_string())
    }
}

fn create_windows_lnk(
    desktop: &Path,
    name: &str,
    target: &Path,
    args: &str,
    icon: Option<&Path>,
) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{Interface, PCWSTR};

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    let lnk = desktop.join(format!("{name}.lnk"));

    // 此函数在安装器自己的后台线程中运行，可以安全地初始化 STA COM apartment。
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("初始化 Windows COM 失败: {e}"))?;

        let result = (|| -> windows::core::Result<()> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let target_w = wide(target.as_os_str());
            link.SetPath(PCWSTR(target_w.as_ptr()))?;

            if !args.is_empty() {
                let args_w = wide(std::ffi::OsStr::new(args));
                link.SetArguments(PCWSTR(args_w.as_ptr()))?;
            }
            if let Some(home) = dirs::home_dir() {
                let home_w = wide(home.as_os_str());
                link.SetWorkingDirectory(PCWSTR(home_w.as_ptr()))?;
            }
            let description_w = wide(std::ffi::OsStr::new("DeepSeek Harness Web GUI"));
            link.SetDescription(PCWSTR(description_w.as_ptr()))?;
            if let Some(icon) = icon {
                let icon_w = wide(icon.as_os_str());
                link.SetIconLocation(PCWSTR(icon_w.as_ptr()), 0)?;
            }

            let persist: IPersistFile = link.cast()?;
            let lnk_w = wide(lnk.as_os_str());
            persist.Save(PCWSTR(lnk_w.as_ptr()), true)?;
            Ok(())
        })();
        CoUninitialize();
        result.map_err(|e| format!("Windows API 创建快捷方式失败: {e}"))?;
    }

    if !lnk.exists() {
        return Err("快捷方式未生成".to_string());
    }
    Ok(lnk)
}

fn create_linux_desktop(
    desktop: &Path,
    name: &str,
    target: &Path,
    args: &str,
) -> Result<PathBuf, String> {
    let file = desktop.join(format!("{name}.desktop"));
    let content = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=DeepSeek Harness Web GUI\nExec={} {}\nTerminal=true\nCategories=Development;\n",
        target.display(),
        args
    );
    std::fs::write(&file, content).map_err(|e| e.to_string())?;
    Ok(file)
}

/// 删除桌面快捷方式。
pub fn remove_desktop_shortcut(name: &str) {
    if let Some(desktop) = desktop_dir() {
        for ext in ["lnk", "desktop"] {
            let _ = std::fs::remove_file(desktop.join(format!("{name}.{ext}")));
        }
    }
}
