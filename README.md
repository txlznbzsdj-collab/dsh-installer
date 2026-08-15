# DSH 安装助手 (dsh-installer)

用 Rust 编写的 **DeepSeek Harness (DSH) 图形化安装程序**。DSH 公测后通过 `npx @deepseek-ai/dsh web` 启动 Web 图形界面；本程序把"检测环境 → 安装 → 建快捷方式 → 启动 Web 界面"全流程封装成一个图形化向导，双击即可完成，无需记忆任何命令。

- 技术栈：Rust + [egui/eframe](https://github.com/emilk/egui)（跨平台，单文件可执行程序，无运行时依赖）
- 语言：界面中文
- 平台：Windows / macOS / Linux（Windows 优先完善）

## 功能

| 步骤 | 功能 |
|---|---|
| ① 欢迎 | 介绍 DSH 与本助手 |
| ② 环境检测 | 自动检测 Node.js / npm / 已装 dsh 版本 |
| ③ 安装 | 一键执行 `npm install -g @deepseek-ai/dsh`（全局）或便携安装到指定目录；实时滚动显示安装日志；支持卸载 |
| ④ 完成 | 创建「DSH Web」桌面快捷方式；自动启动服务并在程序内嵌窗口加载 `http://127.0.0.1:3080` |

### 安装方式说明

- **全局安装（推荐）**：等价于 `npm install -g @deepseek-ai/dsh`，`dsh` 命令全局可用；Windows 上安装到 `%APPDATA%\npm`，无需管理员权限。
- **本地便携安装**：等价于 `npm install --prefix <目录> @deepseek-ai/dsh`，并在目录内生成 `dsh.cmd` / `dsh` 启动器，便于绿色携带、多版本并存。
- **指定版本安装**：可填写 `latest`、`next` 或具体版本号，方便升级、降级和锁定版本。
- **镜像/自定义源安装**：内置 npm 官方源和 npmmirror 国内镜像，也可填写自定义 Registry。
- **离线安装**：选择预先下载好的 `deepseek-ai-dsh-*.tgz`，适用于内网或网络受限环境。

## 快速开始

### 使用（直接运行）

下载 `dsh-installer` 可执行文件（见 [Releases](#) 或自行构建），双击运行，按向导操作即可。

### 环境要求

- **Node.js 18+（推荐 20+）**：<https://nodejs.org>
- 网络可达 npm registry（国内可配置镜像，见 FAQ）
- Windows 7+ / macOS / Linux

## 从源码构建

```bash
# 需要 Rust 工具链 (https://rustup.rs)
cargo build --release
# 产物: target/release/dsh-installer(.exe)
```

### 无头自检（开发/诊断）

```bash
# 仅环境检测
cargo run -- --selftest

# 检测 + 真实全局安装（会改动本机 npm 全局环境）
cargo run -- --selftest --install

# 检测 + 便携安装到指定目录
cargo run -- --selftest --install --local C:\dsh-test
```

> 注意：release 版没有控制台窗口，`--selftest` 的输出只在 debug 构建（`cargo run`）中可见。

## 项目结构

```
dsh-installer/
├── Cargo.toml
└── src/
    ├── main.rs          # 入口；--selftest 无头自检
    ├── app.rs           # 四步向导 GUI + 中文字体加载
    └── core/
        ├── detect.rs    # 环境检测（定位 node/npm/dsh、解析版本）
        ├── npm.rs       # npm 安装/卸载/校验，输出流式写入日志
        ├── shortcut.rs  # 桌面快捷方式（Windows .lnk / Linux .desktop）
        ├── launch.rs    # 启动 dsh web + 检测服务端口
        ├── webview.rs   # 使用系统 WebView2 在程序窗口内显示网页
        └── log.rs       # 后台线程 → UI 的共享日志缓冲
```

## FAQ

**Q: 安装时报 npm 网络错误？**
A: 国内用户可为 npm 配置镜像源后重试：`npm config set registry https://registry.npmmirror.com`

**Q: DSH Web 界面是什么？**
A: `dsh web` 是 `dsh --profile web` 的别名，启动 DeepSeek Harness 的浏览器图形界面，默认地址 `http://127.0.0.1:3080`（可在安装页修改端口）。

**Q: 如何卸载 DSH？**
A: 在安装页点击「卸载」；或手动执行 `npm uninstall -g @deepseek-ai/dsh`。

**Q: 为什么需要 Node.js？**
A: DSH 是 npm 分发的 Node.js 应用（`@deepseek-ai/dsh`，MIT 协议），安装与运行都依赖 Node.js 运行时。

## License

MIT。DSH 本身为 DeepSeek 官方发布，MIT 协议：<https://github.com/deepseek-ai/DeepSeek-Harness>
