# FluffyChat Portable (Tauri 版) —— 离线单二进制桌面客户端

把 [FluffyChat](https://fluffychat.im)（Matrix 客户端）Web 版打包为**离线单二进制桌面应用**。
解决 fluffychat.im 托管在 GitHub Pages（境外）导致的加载慢问题：界面资源全部本地化，
用系统 WebView 渲染（Windows = WebView2 / Linux = WebKitGTK），**应用本体零外网依赖**。

- 版本: FluffyChat **v2.8.0** (build 3558)
- 体积: 单 exe ≈ **33MB**（含全部前端资源, 已裁剪 debug/sourcemap/非目标渲染器）
- 平台: Windows x64（本目录为 Tauri 版；Go 版见 `main` 分支）

> 与 `main` 分支（Go embed + 本地 HTTP server + 浏览器）的区别：本版为**原生桌面窗口**，
> 独立窗口 + 系统托盘 + 单实例，不再依赖浏览器。两者功能对齐（动态 config.json、托盘常驻）。

## 快速开始（Windows）

```bat
:: 双击即用: 弹出桌面窗口, 直接进入 FluffyChat
fluffychat-portable.exe

:: 预设默认 Matrix 服务器 (开箱即用)
fluffychat-portable.exe -homeserver matrix.rumor.work

:: 登录页提供多个可选服务器 (逗号分隔)
fluffychat-portable.exe -preset "server1.example,server2.example"
```

> 关闭窗口 = **隐藏到系统托盘**（不退出，聊天会话保持）。托盘图标：左键唤回窗口，右键菜单「打开 FluffyChat / 退出」。
> 重复启动不会开第二个窗口，而是唤醒已有窗口（单实例）。

## 命令行参数

| 参数 | 说明 | 默认 |
|---|---|---|
| `-homeserver <url>` | 预设默认 Matrix homeserver | 空 (matrix.org) |
| `-preset <a,b,c>` | 登录页预设服务器列表 (优先级高于 homeserver) | 空 |

> Go 版的 `-port` / `-no-open` / `-dir` / `-stop` 在 Tauri 版不再适用（无 HTTP server）；
> 单实例已由插件接管，退出走托盘菜单。

## 实现要点

- **前端资源本地化**：`prepare.sh` 下载官方 release 并裁剪，产出 `build/web_offline/`，
  由 `rust-embed` 编译期嵌入，经 **`fc://` 自定义协议**提供（等价于 Go 版的本地 HTTP server）。
  `prepare.sh` 完成三处必要改写：
  1. `<base href="/web/">` → `/`
  2. canvaskit 强制本地加载（v2.8.0 loader 需同时注入 `buildConfig.useLocalCanvasKit` 和
     `load({config:{canvasKitBaseUrl}})` 双保险）
  3. 移除 Flutter 自带 Service Worker
- **动态 config.json**：协议处理器对 `/config.json` 返回运行时生成的配置（等价 Go 版
  `-homeserver` / `-preset` 参数），其余路径从嵌入资源读取（含正确 MIME，`wasm` → `application/wasm`）。
- **SSO 登录 (OIDC / legacy SSO)**：FluffyChat 的 SSO 用 `window.open` 开鉴权弹窗（Tauri 默认拦截）。
  本版通过 `on_new_window` 创建真正的子窗口——WebView2 `SetNewWindow` 自动建立 `window.opener`
  关系并导航到鉴权页，完成后 `auth.html` 经 `postMessage` 回传 loginToken，支持 Mozilla 等 SSO 服务器。
  该弹窗是通用的（聊天外链、文件预览等同样走这里）：标题跟随页面真实 `document.title`
  （导航中显示目标域名），不再写死为“登录”，SSO 输入凭据时也能看清当前站点，防钓鱼。
- **内部浏览器弹窗**：聊天里的链接/SSO/文件预览统一在内部弹窗打开（共享 WebView 存储）。
  弹窗补齐了“地址可获取、可转交系统浏览器”的能力（信令通道 `fc://__shell/*`）：
  - **Ctrl+L**：复制当前页地址（toast 同时展示完整地址，复制失败也能获取）；
  - **Ctrl+Shift+O**：当前页转交系统默认浏览器打开；
  - 菜单栏「页面(P)」：复制地址 / 在系统浏览器打开 / 后退（Alt+←）/ 前进（Alt+→）/ 刷新（Ctrl+R）
    （后三个快捷键为 WebView2 原生 accelerator）；
  - 弹窗内 `target=_blank` 二级链接可继续点击打开（递归弹窗）。
- **主窗口 Ctrl+左键**：按住 Ctrl 点击聊天链接，直接用系统浏览器打开（绕过内部弹窗）；
  SSO 登录链接始终走内部弹窗，保证登录回跳流程不被打断。
- **文件下载**：聊天里保存图片/文件（FluffyChat Web = blob URL + `<a download>`）由 `on_download`
  统一接管——保存到系统“下载”目录，同名自动追加序号（`photo (1).png`），完成后自动在文件管理器中
  定位该文件（Windows 资源管理器选中态 / macOS `open -R` / Linux `xdg-open`）。主窗口与弹窗均生效。
- **系统托盘**：关闭窗口隐藏到托盘；左键唤回，右键菜单「打开 / 退出」。
- **单实例**：`tauri-plugin-single-instance`，二次启动唤醒已有窗口。
- **离线验证**（main 分支已用 Playwright 拦截外部请求实测）：Flutter app 完整启动
  （`#/home` 路由 + canvas 渲染 + vodozemac E2EE wasm 本地加载），唯一外部请求是
  Flutter 引擎的字体回退（失败静默切系统字体，无白屏）。本版渲染内核同为 WebKit/Blink，行为一致。

## 从源码构建

```bash
# 前置: Rust 1.77+ ; Windows 构建需 MSVC 工具链或 cargo-xwin (见下); Linux 需 WebKitGTK

# 1) 生成离线 web 资源 (build/web_offline/) + 图标 (src-tauri/icons/)
./prepare.sh v2.8.0          # 国内网络自动走 gh.ddlc.top 加速镜像

# 2) Windows x64 单文件 exe
#    原生 Windows 机器:
cd src-tauri && cargo build --release          # 产物 target/release/fluffychat-portable.exe
#    Linux 交叉编译 (需 clang + lld):
cargo install cargo-xwin
cargo xwin build --release --target x86_64-pc-windows-msvc

# 3) 打包安装器 (NSIS/MSI, 需 tauri-cli, 建议在 Windows 上执行)
cargo install tauri-cli --locked
cd src-tauri && cargo tauri build
```

> `build/web_offline/`、`src-tauri/icons/`、`src-tauri/target/` 均为生成物，不纳入版本控制。

## 数据存储与升级（登录 / E2EE / 消息缓存）

登录会话、olm/megolm E2EE 密钥、消息数据库全部由 FluffyChat Web 写入 WebView 的
IndexedDB / localStorage（无独立文件），落在 WebView **用户数据目录（UDF）**：

| 运行方式 | 数据位置 |
|---|---|
| 便携 exe（目录可写，最常见） | exe 同目录下 `fluffychat-portable.exe.WebView2\`（WebView2 默认 UDF，数据跟着 exe 走） |
| exe 在只读目录（如 Program Files） | 自动回退 `%LOCALAPPDATA%\im.fluffychat.portable\`（否则 WebView2 会创建 UDF 失败无法启动） |
| 安装器版（NSIS/MSI） | 安装目录（`%LOCALAPPDATA%\Programs\FluffyChat Portable\`）下的同名 UDF |
| Linux | `~/.local/share/` 下（WebKitGTK 默认） |

**升级是否丢数据：**

- ✅ 便携 exe **原地覆盖**（同一目录替换旧 exe）：不丢
- ✅ 安装器**升级安装**：不丢（UDF 在安装目录，升级只替换程序文件）
- ⚠️ 新 exe **放到别的目录运行**：相当于全新 profile（旧数据仍在旧目录旁，把整个目录连同
  `fluffychat-portable.exe.WebView2\` 一起搬移即可找回）
- ❌ **卸载安装器版**：安装目录连同 UDF 一起删除，登录与 E2EE 密钥丢失——卸载前请先在
  FluffyChat 设置中导出加密备份
- 换机器：同上，走加密备份恢复

## CI / 自动跟随上游发布

- `ci.yml`：push/PR 验证可重现构建（prepare.sh + 双平台编译）
- `release.yml`：手动触发（输入上游版本号）→ 构建 + 发布 Release；同时作为可复用工作流被调用。
  构建时会校验下载的 web 包版本与请求一致（防陈旧产物），并把 `tauri.conf.json`/`Cargo.toml`
  的版本同步为上游版本（否则安装器命名/exe 属性停留在仓库基线版本号）
- `auto-upstream.yml`：每 6 小时轮询 FluffyChat 上游 releases，发现新版本（正式 release 且带
  `fluffychat-web.tar.gz` 资产）且本仓库尚无对应 `tauri-<版本>` Release 时，自动调用
  `release.yml` 构建发布

> ⚠️ GitHub 的 `schedule` 触发只在**默认分支**生效；本仓库默认分支为 `main`（Go 版），
> 将 `tauri` 分支合入 `main`（或把默认分支切到 `tauri`）后自动跟随才会定时运行。
> 上游构建布局变化导致 `prepare.sh` 失败时自动构建会持续报错，需手动修复脚本
>（期间可先在 Settings → Actions 中禁用该工作流，修好后可手动触发重出包）。

## 注意事项

- **Matrix 通信仍需联网**：本方案只解决"界面加载慢"；消息收发走你所选的 homeserver。
  建议配合国内自建/公共服务器（`-homeserver` 预设），否则收发仍慢。
- **WebView2 依赖**：Win10/11 普遍内置；极老/精简系统需先装一次 WebView2 Runtime（微软官方）。
- **字体**：离线时 UI 使用系统字体（中文 = 微软雅黑），观感略有差异。
- **E2EE 数据**：见上文「数据存储与升级」——登录会话/密钥/消息缓存在 WebView UDF 中，
  便携原地覆盖升级不丢；卸载安装器版会删除，请先导出加密备份。
- **更新**：上游发新版本时重跑 `./prepare.sh <新版本>` + 重新构建即可。

## 许可证

- 本项目整体: **GNU AGPL-3.0** (见 [LICENSE](LICENSE))
- 打包的 FluffyChat Web (v2.8.0): **AGPL-3.0**, 上游源码 https://github.com/krille-chan/fluffychat
- 依赖与第三方组件声明: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

## 目录

```
src-tauri/
  src/main.rs          Rust 入口 (自定义协议 + 动态 config.json + 托盘 + 单实例 + 窗口)
  tauri.conf.json      窗口/打包配置
  capabilities/        权限能力声明
  icons/               窗口/托盘/打包图标 (prepare.sh 生成)
  static/              占位 frontendDist (真实资源经 fc:// 协议 + rust-embed 提供)
prepare.sh             下载/裁剪/改写官方 release → build/web_offline/ + 图标 (生成物, 不入库)
LICENSE                AGPL-3.0
THIRD_PARTY_NOTICES.md   第三方组件与合规声明
EVALUATION.md          方案评估报告
```
