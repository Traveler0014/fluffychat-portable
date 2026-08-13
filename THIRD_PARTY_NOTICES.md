# Third-Party Notices

本仓库是 FluffyChat Web 的离线封装（Tauri launcher + 打包），整体按 **GNU AGPL-3.0** 授权（见 LICENSE）。
以下为本项目使用/打包的第三方组件的许可声明。

## FluffyChat (AGPL-3.0)

- 项目: [krille-chan/fluffychat](https://github.com/krille-chan/fluffychat) — The cutest messenger in the Matrix network
- 许可: **GNU Affero General Public License v3.0** ([LICENSE](https://github.com/krille-chan/fluffychat/blob/main/LICENSE))
- 用途: 本项目打包其官方 Web 构建产物 (`fluffychat-web.tar.gz`), 版本 **v2.8.0** (build 3558)
- 对应源码: https://github.com/krille-chan/fluffychat (tag `v2.8.0`)
- 可重现构建: 本仓库 `prepare.sh` 下载官方 release 并裁剪/改写, 产出 `build/web_offline/`;
  本项目自身的 `src-tauri/src/main.rs` 等源码也是该衍生作品的对应源码组成部分
- 版权: © FluffyChat 贡献者 (Krille-chan et al.)

## Tauri / wry / tao (MIT OR Apache-2.0)

- 项目: [tauri-apps/tauri](https://github.com/tauri-apps/tauri) 及 tauri 运行时组件 (wry, tao, muda, tray-icon 等)
- 许可: MIT OR Apache-2.0（双许可）
- 用途: 桌面壳 (窗口 / WebView2 / WebKitGTK / 系统托盘)

## tauri-plugin-single-instance (MIT OR Apache-2.0)

- 项目: [tauri-apps/plugins-workspace](https://github.com/tauri-apps/plugins-workspace)
- 许可: MIT OR Apache-2.0
- 用途: 单实例（二次启动唤醒已有窗口）

## rust-embed (MIT)

- 项目: [pyros2097/rust-embed](https://github.com/pyros2097/rust-embed)
- 许可: MIT
- 用途: 编译期嵌入裁剪后的 FluffyChat Web 资源

## serde / serde_json (MIT OR Apache-2.0)

- 用途: 运行时 config.json 的序列化

## Microsoft Edge WebView2 (系统组件)

- Windows 上由操作系统提供 (Win10/11 普遍内置, 缺省时按微软官方分发条款安装)。
- 本项目不随包分发 WebView2 Runtime 本身。

## 字体 / 图标 (随 FluffyChat web 资源打包)

- Noto Sans SC / Noto Color Emoji: [SIL Open Font License 1.1](https://openfontlicense.org)
  (运行时按需从 Google Fonts 拉取, 离线时回退系统字体)
- Material Icons: Apache License 2.0
- Cupertino Icons: MIT License

## Rust 标准库 / 工具链 (MIT OR Apache-2.0)

Rust programming language, Copyright (c) The Rust Project Developers.

---

## 合规说明 (AGPL-3.0 §13 网络服务条款)

本项目分发的单二进制以内嵌 WebView 运行 FluffyChat（本地 fc:// 自定义协议，
等价于本地服务），属于 AGPL-3.0 第 13 条所定义的"通过网络提供服务"。
为满足对应源码 (Corresponding Source) 提供义务，本仓库公开了：

1. 全部自研代码 (`src-tauri/`, `prepare.sh`)
2. 上游对应源码来源与精确版本 (`FluffyChat v2.8.0`)
3. 可重现的构建流程 (见 README「从源码构建」)

任何用户均可据此获得并修改完整源码。
