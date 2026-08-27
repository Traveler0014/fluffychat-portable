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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use rust_embed::RustEmbed;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{DownloadEvent, NewWindowResponse, NewWindowFeatures, PageLoadEvent};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// 裁剪后的 FluffyChat Web 静态资源 (由 prepare.sh 生成到 build/web_offline/)
/// 注意: 相对路径基于 $CARGO_MANIFEST_DIR = src-tauri/
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../build/web_offline/"]
struct WebAssets;

/// 运行时生成的 config.json 内容 (等价 Go 版 renderConfig)
static CONFIG_JSON: OnceLock<String> = OnceLock::new();

/// 用于给 window.open 弹出的子窗口生成唯一 label
static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

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
            setup_main_window(app)?;
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

/// 创建主窗口 (含 window.open 弹窗处理与文件下载处理)
fn setup_main_window(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle().clone();
    let url: tauri::Url = "fc://localhost/".parse().expect("valid fc url");

    WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("FluffyChat")
        .inner_size(480.0, 820.0)
        .min_inner_size(360.0, 600.0)
        .center()
        .on_download(make_download_handler())
        .on_new_window(move |url, features| open_web_popup(&handle, &url, features))
        .build()?;
    Ok(())
}

/// window.open 的通用子窗口: SSO/OIDC 登录、聊天中的外部链接、文件预览等,
/// 不只是登录场景。Tauri 默认 Deny window.open, 这里创建真正的子窗口:
/// WebView2 的 SetNewWindow 会自动建立 window.opener 关系并导航到目标 url
/// (SSO 场景下 auth.html 完成后经 postMessage 回传 loginToken)。
///
/// 标题不写死: 初始显示目标站点域名, 页面加载后跟随 document.title (同浏览器),
/// 用户始终能看出弹窗里是哪个站点 —— SSO 输入凭据时这也是防钓鱼的关键信息。
fn open_web_popup(
    handle: &tauri::AppHandle,
    url: &tauri::Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<tauri::Wry> {
    let label = format!(
        "web-popup-{}",
        NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed)
    );
    let initial_title = host_of(url).unwrap_or_else(|| "FluffyChat".into());

    let builder = WebviewWindowBuilder::new(
        handle,
        &label,
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .inner_size(480.0, 640.0)
    .window_features(features)
    .title(&initial_title)
    // 页面 document.title 变化 → 同步为窗口标题
    .on_document_title_changed(|window, title| {
        let _ = window.set_title(&title);
    })
    // 导航开始时先显示目标域名: 多跳转的 OIDC 流程中, 避免新页面加载完成前
    // 窗口仍停留在上一跳的标题
    .on_page_load(|window, payload| {
        if matches!(payload.event(), PageLoadEvent::Started) {
            if let Some(host) = host_of(payload.url()) {
                let _ = window.set_title(&host);
            }
        }
    })
    // 弹窗里的网页同样可能触发下载 (如页面上的文件链接)
    .on_download(make_download_handler());

    match builder.build() {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => NewWindowResponse::Deny,
    }
}

fn host_of(url: &tauri::Url) -> Option<String> {
    url.host_str()
        .map(str::to_string)
        .filter(|host| !host.is_empty())
}

/// 文件下载处理 (on_download), 主窗口与弹窗共用。
///
/// wry 的默认行为是"允许下载但完全静默": 无进度、无完成提示, 文件悄悄落在
/// WebView 默认下载目录, 用户感知上就是"点了保存没反应"。这里统一改为:
///
/// 1. 保存到系统"下载"目录, 同名文件自动追加序号 (photo.png → photo (1).png)
/// 2. 下载完成后在文件管理器中定位该文件, 作为明确的完成反馈
///
/// 注: FluffyChat Web 保存文件 = 取回字节 → blob URL → `<a download>` 点击,
/// 各 WebView 均将此作为标准下载事件处理, 建议文件名即 download 属性值。
fn make_download_handler(
) -> impl Fn(tauri::Webview<tauri::Wry>, DownloadEvent<'_>) -> bool + Send + Sync + 'static {
    |webview, event| match event {
        DownloadEvent::Requested { url, destination } => {
            let dir = webview
                .path()
                .download_dir()
                .unwrap_or_else(|_| std::env::temp_dir());
            let name = destination
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| file_name_from_url(&url));
            *destination = unique_destination(&dir, &name);
            // 返回 true = 允许下载
            true
        }
        DownloadEvent::Finished { path, success, .. } => {
            if success {
                if let Some(path) = path {
                    reveal_in_file_manager(&path);
                }
            }
            true
        }
        _ => true,
    }
}

/// 计算下载目标路径: 剥离建议文件名中的路径分隔符 (防逃逸下载目录),
/// 已存在同名文件时追加序号 (对齐浏览器行为)。
fn unique_destination(dir: &Path, raw_name: &str) -> PathBuf {
    let name: String = raw_name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            _ => c,
        })
        .collect();
    let name = name.trim();
    if name.is_empty() {
        return dir.join("download");
    }

    let path = dir.join(name);
    if !path.exists() {
        return path;
    }

    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    (1..=999)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(path)
}

/// 兜底文件名: webview 未给出建议文件名时取 URL 最后一段
fn file_name_from_url(url: &tauri::Url) -> String {
    url.path_segments()
        .and_then(|mut segs| segs.next_back())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "download".into())
}

/// 在系统文件管理器中定位并显示文件 (下载完成的可见反馈)
fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.as_os_str().to_string_lossy()))
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
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
