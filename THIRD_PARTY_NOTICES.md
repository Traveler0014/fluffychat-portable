# Third-Party Notices

本仓库是 FluffyChat Web 的离线封装（launcher + 打包），整体按 **GNU AGPL-3.0** 授权（见 LICENSE）。
以下为本项目使用/打包的第三方组件的许可声明。

## FluffyChat (AGPL-3.0)

- 项目: [krille-chan/fluffychat](https://github.com/krille-chan/fluffychat) — The cutest messenger in the Matrix network
- 许可: **GNU Affero General Public License v3.0** ([LICENSE](https://github.com/krille-chan/fluffychat/blob/main/LICENSE))
- 用途: 本项目打包其官方 Web 构建产物 (`fluffychat-web.tar.gz`), 版本 **v2.8.0** (build 3558)
- 对应源码: https://github.com/krille-chan/fluffychat (tag `v2.8.0`)
- 可重现构建: 本仓库 `prepare.sh` 下载官方 release 并裁剪/改写, 产出 `build/web_offline/`;
  本项目自身的 `main.go` / `tray_*.go` 也是该衍生作品的对应源码组成部分
- 版权: © FluffyChat 贡献者 (Krille-chan et al.)

## getlantern/systray (Apache-2.0)

- 项目: [getlantern/systray](https://github.com/getlantern/systray)
- 许可: Apache License 2.0
- 用途: Windows 系统托盘 (仅 Windows 构建包含, 通过 build tag 隔离)

## 字体 / 图标 (随 FluffyChat web 资源打包)

- Noto Sans SC / Noto Color Emoji: [SIL Open Font License 1.1](https://openfontlicense.org)
  (运行时按需从 Google Fonts 拉取, 离线时回退系统字体)
- Material Icons: Apache License 2.0
- Cupertino Icons: MIT License

## Go 标准库 / 工具链 (BSD-3-Clause)

Go programming language, Copyright (c) The Go Authors. BSD-3-Clause.

---

## 合规说明 (AGPL-3.0 §13 网络服务条款)

本项目分发的单二进制以本地 HTTP 服务方式运行 FluffyChat (127.0.0.1)，
属于 AGPL-3.0 第 13 条所定义的"通过网络提供服务"。
为满足对应源码 (Corresponding Source) 提供义务，本仓库公开了：

1. 全部自研代码 (`main.go`, `tray_*.go`, `prepare.sh`, `build.sh`)
2. 上游对应源码来源与精确版本 (`FluffyChat v2.8.0`)
3. 可重现的构建流程 (见 README「从源码构建」)

任何用户均可据此获得并修改完整源码。
