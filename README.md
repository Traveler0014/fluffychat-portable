# FluffyChat Portable —— 离线单二进制版 FluffyChat Web

把 [FluffyChat](https://fluffychat.im)（Matrix 客户端）Web 版打包为**离线单二进制**。
解决 fluffychat.im 托管在 GitHub Pages（境外）导致的加载慢问题：界面资源全部本地化，
本地起 HTTP 服务后自动打开浏览器，**应用本体零外网依赖**。

- 版本: FluffyChat **v2.8.0** (build 3558)
- 体积: 单 exe ≈ **36MB**（含全部前端资源, 已裁剪 debug/sourcemap/非目标渲染器）
- 平台: Windows x64 / **win32 x86** / Linux x64

## 快速开始（Windows）

```bat
:: 双击即用: 自动起本地服务 (127.0.0.1:8800) 并打开默认浏览器
fluffychat-portable-win64-gui-v2.8.0.exe

:: 预设默认 Matrix 服务器 (开箱即用, 会写入运行时 config.json)
fluffychat-portable-win64-gui-v2.8.0.exe -homeserver matrix.rumor.work

:: 登录页提供多个可选服务器 (逗号分隔)
fluffychat-portable-win64-gui-v2.8.0.exe -preset "server1.example,server2.example"

:: 固定/随机端口
fluffychat-portable-win64-gui-v2.8.0.exe -port 0
```

> GUI 静默版 (`-gui`) 双击无黑窗, **常驻系统托盘**: 右键托盘图标可“打开 FluffyChat”或“退出”。
> 重复启动同一 exe 不会开第二个进程, 而是唤醒浏览器并退出（单实例探测）。

## 退出方式

| 平台 | 方式 |
|---|---|
| Windows (推荐) | 系统托盘图标 → 右键 → **退出** (托盘常驻, 随时可退) |
| Windows | 控制台版 Ctrl+C; 或再次运行 `fluffychat-portable.exe -stop` |
| Linux | Ctrl+C; 或 `fluffychat-portable -stop` |

> 说明: 关闭浏览器标签页**不会**自动退出服务(避免误杀正在聊天的会话), 退出请使用托盘/Ctrl+C/-stop。

## 命令行参数

| 参数 | 说明 | 默认 |
|---|---|---|
| `-port N` | 监听端口, `0` = 随机空闲端口 | 8800 |
| `-homeserver <url>` | 预设默认 Matrix homeserver | 空 (matrix.org) |
| `-preset <a,b,c>` | 登录页预设服务器列表 (优先级高于 homeserver) | 空 |
| `-no-open` | 不自动打开浏览器 | false |
| `-dir <path>` | 调试: 从外部目录加载 web 资源 (免重新编译) | 空 (内置) |
| `-stop` | 请求运行中的实例退出 (需与实例端口一致, 见 `-port`) | false |

## 从源码构建

```bash
# 前置: Go 1.21+ (可选 python3 + Pillow 用于生成托盘图标)

# 1) 生成离线 web 资源 (build/web_offline/)
./prepare.sh v2.8.0          # 国内网络自动走 gh.ddlc.top 加速镜像

# 2) 构建 (Linux + Windows x64/x86 + GUI 版, 产物在 dist/)
./build.sh

# 3) 直接运行 / 调试
go build -o fluffychat-portable .
./fluffychat-portable -homeserver matrix.rumor.work
./fluffychat-portable -dir ./build/web_offline    # 改资源不用重编译
```

> Windows 构建为交叉编译 (无需 Windows 机器)。`build/web_offline` 和 `dist/` 均为生成物,
> 不纳入版本控制, 由上述脚本可重现。

## 实现要点

- **资源本地化**: `go:embed` 嵌入裁剪后的 Flutter Web 产物; `prepare.sh` 完成三处必要改写:
  1. `<base href="/web/">` → `/` (否则本地路径错乱)
  2. canvaskit 强制本地加载 (默认走 `gstatic.com` CDN, v2.8.0 的 loader 需同时注入
     `buildConfig.useLocalCanvasKit` 和 `load({config:{canvasKitBaseUrl}})` 双保险)
  3. 移除 Flutter 自带 Service Worker (本地服务无需, 避免版本缓存)
- **运行时 config.json**: FluffyChat 支持从 `config.json` 读取默认服务器/欢迎语等配置,
  由 Go handler 动态生成 (线上该文件 404, app 回退内置默认; 动态生成则完全可控)
- **单实例**: 固定端口被占用时探测 `/health` 握手, 确认是自己实例则唤醒浏览器后退出
- **系统托盘** (Windows): 常驻托盘图标, 菜单含“打开 FluffyChat”/“退出”; `-stop` 通过 `/shutdown` 接口请求退出
- **离线验证**: Playwright 拦截全部外部请求实测 —— Flutter app 完整启动
  (`#/home` 路由 + canvas 渲染 + vodozemac E2EE wasm 本地加载), 唯一外部请求是
  Flutter 引擎的字体回退 (失败静默切系统字体, 无白屏)

## 注意事项

- **Matrix 通信仍需联网**: 本方案只解决"界面加载慢"; 消息收发走你所选的 homeserver。
  建议配合国内自建/公共服务器 (`-homeserver` 预设), 否则收发仍慢。
- **字体**: 离线时 UI 使用系统字体 (中文 = 微软雅黑), 观感略有差异。
- **E2EE 数据**: 登录会话/密钥/消息缓存放浏览器本地存储; 换机器需走 FluffyChat 加密备份恢复。
- **更新**: 上游发新版本时重跑 `./prepare.sh <新版本>` + `./build.sh` 即可 (约几分钟)。

## 许可证

- 本项目整体: **GNU AGPL-3.0** (见 [LICENSE](LICENSE))
- 打包的 FluffyChat Web (v2.8.0): **AGPL-3.0**, 上游源码 https://github.com/krille-chan/fluffychat
- 依赖与第三方组件声明: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
- 对应源码义务: 本仓库公开全部自研代码 + 上游版本与可重现构建流程 (见上), 满足 AGPL-3.0 要求

## 发布到 GitHub

### 方式一: GitHub Actions 构建并发布 (推荐)

仓库已内置两个 workflow:

- **`Build & Release`** (手动触发): Actions 页面 → Run workflow → 输入 FluffyChat 版本
  (默认 v2.8.0)。CI 自动执行 `prepare.sh` + `build.sh` (交叉编译 Windows/Linux),
  冒烟测试后把 4 个二进制发布到 GitHub Releases (tag: `portable-<version>`)。
- **`CI`** (自动): 每次 push/PR 跑完整可重现构建 + 冒烟测试。

> 首次使用时确保仓库 Settings → Actions → General 允许 workflow 创建 release
> (默认 token 的 `contents: write` 权限)。

### 方式二: 本地构建

```bash
# 源码进 git (不含生成物)
git init && git add . && git commit -m "init"
git remote add origin git@github.com:<you>/portable-fluffychat.git
git push -u origin main

# 二进制通过 Releases 发布 (git 不跟踪)
gh release create portable-v2.8.0 dist/fluffychat-portable-win64-gui-v2.8.0.exe \
  dist/fluffychat-portable-win64-v2.8.0.exe dist/fluffychat-portable-win32-v2.8.0.exe \
  dist/fluffychat-portable-linux-x64 --title "FluffyChat Portable v2.8.0"
```

## 目录

```
main.go            Go 入口 (embed + HTTP server + 单实例 + config.json + 退出控制)
tray_windows.go    Windows 系统托盘 (systray)
tray_other.go      非 Windows 平台的信号处理
prepare.sh         下载/裁剪/改写官方 release → build/web_offline/ (生成物, 不入库)
build.sh           交叉编译 → dist/ (生成物, 不入库)
LICENSE            AGPL-3.0
THIRD_PARTY_NOTICES.md   第三方组件与合规声明
EVALUATION.md      方案评估报告
```
