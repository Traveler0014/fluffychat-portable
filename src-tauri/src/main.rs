// FluffyChat Portable —— Tauri 版: 把 FluffyChat Web 打包为离线单二进制桌面应用
//
// 与 Go 版 (embed + 本地 HTTP server + 浏览器) 的区别:
//   - 本机 WebView2/WebKitGTK 渲染, 独立窗口 + 系统托盘 + 单实例
//   - 前端资源经自定义协议 fc:// 提供 (等价于 Go 版的 HTTP server)
//   - 动态 config.json (等价 Go 版 -homeserver/-preset 参数)
//
// 用法:
//   fluffychat-portable                     # 默认 homeserver = matrix.org
//   fluffychat-portable -homeserver <url>   # 预设默认 Matrix 服务器
//   fluffychat-portable -preset <a,b,c>     # 登录页预设服务器列表
//
// 退出方式:
//   关闭窗口 = 隐藏到托盘 (常驻); 托盘图标 → 右键 → 退出
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::OnceLock;

use rust_embed::RustEmbed;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

/// 裁剪后的 FluffyChat Web 静态资源 (由 prepare.sh 生成到 build/web_offline/)
/// 注意: 相对路径基于 $CARGO_MANIFEST_DIR = src-tauri/
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../build/web_offline/"]
struct WebAssets;

/// 运行时生成的 config.json 内容 (等价 Go 版 renderConfig)
static CONFIG_JSON: OnceLock<String> = OnceLock::new();

fn main() {
    let (homeserver, preset) = parse_args();
    CONFIG_JSON
        .set(render_config(homeserver.as_deref(), preset.as_deref()))
        .ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 二次启动: 唤醒已有窗口
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .register_uri_scheme_protocol("fc", |_ctx, req| handle_protocol(req))
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭主窗口 → 隐藏到托盘 (托盘常驻, 对齐 Go 版语义)
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running FluffyChat Portable");
}

/// 解析 -homeserver / -preset 命令行参数 (兼容 -x 与 --x 两种写法)
fn parse_args() -> (Option<String>, Option<String>) {
    let mut homeserver = None;
    let mut preset = None;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-homeserver" | "--homeserver" => {
                if i + 1 < args.len() {
                    homeserver = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-preset" | "--preset" => {
                if i + 1 < args.len() {
                    preset = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (homeserver, preset)
}

/// 生成 FluffyChat 运行时配置 (对应上游 config.sample.json 字段)。
/// homeserver / preset 为空时不覆盖上游默认值。
fn render_config(homeserver: Option<&str>, preset: Option<&str>) -> String {
    let mut cfg = serde_json::json!({
        "applicationName": "FluffyChat",
        "defaultHomeserver": "matrix.org",
        "presetHomeserver": "",
        "welcomeText": "",
        "privacyUrl": "https://github.com/krille-chan/fluffychat/blob/main/PRIVACY.md",
        "audioRecordingNumChannels": 1,
        "audioRecordingAutoGain": true,
        "audioRecordingEchoCancel": false,
        "audioRecordingNoiseSuppress": true,
        "audioRecordingBitRate": 64000,
        "audioRecordingSamplingRate": 44100,
        "renderHtml": true,
        "fontSizeFactor": 1,
        "messagePreviewMaxLines": 128,
        "hideRedactedEvents": false,
        "hideUnknownEvents": true,
        "separateChatTypes": false,
        "autoplayImages": true,
        "sendTypingNotifications": true,
        "sendPublicReadReceipts": true,
        "swipeRightToLeftToReply": true,
        "sendOnEnter": false,
        "showPresences": true,
        "displayNavigationRail": false,
        "experimentalVoip": false,
        "shareKeysWith": "all",
        "noEncryptionWarningShown": false,
        "displayChatDetailsColumn": false,
        "colorSchemeSeedInt": 4283835834_u64,
        "enableSoftLogout": false,
    });

    if let Some(h) = homeserver {
        if !h.is_empty() {
            cfg["defaultHomeserver"] = serde_json::Value::String(h.to_string());
        }
    }
    let preset_joined = if let Some(p) = preset {
        p.split([',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",")
    } else {
        String::new()
    };
    if !preset_joined.is_empty() {
        cfg["presetHomeserver"] = serde_json::Value::String(preset_joined);
    } else if let Some(h) = homeserver {
        if !h.is_empty() {
            cfg["presetHomeserver"] = serde_json::Value::String(h.to_string());
        }
    }

    serde_json::to_string_pretty(&cfg).unwrap_or_else(|_| "{}".into())
}

/// fc:// 自定义协议处理器: 动态 config.json + 嵌入静态资源
fn handle_protocol(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = request.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if path == "config.json" {
        let body = CONFIG_JSON
            .get()
            .cloned()
            .unwrap_or_else(|| "{}".to_string())
            .into_bytes();
        return respond(200, "application/json; charset=utf-8", body);
    }

    match WebAssets::get(path) {
        Some(file) => respond(200, mime_type(path), file.data.to_vec()),
        None => respond(
            404,
            "text/plain",
            b"not found".to_vec(),
        ),
    }
}

fn respond(status: u16, mime: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(body)
        .unwrap()
}

/// 按扩展名返回 MIME (wasm 必须为 application/wasm, 否则 WebView 拒绝实例化)
fn mime_type(path: &str) -> &'static str {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "ogg" => "audio/ogg",
        "frag" => "text/plain",
        "dart" => "text/plain",
        "bin" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// 系统托盘: 左键唤醒窗口, 右键菜单 (显示/退出)
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开 FluffyChat", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    TrayIconBuilder::new()
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("window icon"),
        )
        .tooltip("FluffyChat Portable")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}
