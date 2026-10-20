// FluffyChat Portable —— Tauri 版: 把 FluffyChat Web 打包为离线单二进制桌面应用
//
// 与 Go 版 (embed + 本地 HTTP server + 浏览器) 的区别:
//   - 本机 WebView2/WebKitGTK 渲染, 独立窗口 + 系统托盘 + 单实例
//   - 前端资源经自定义协议 fc:// 提供 (等价于 Go 版的 HTTP server)
//   - 动态 config.json (等价 Go 版 -homeserver/-preset 参数)
//   - 弹窗 (内部浏览器) 能力补齐: 菜单栏 + Ctrl+L 取地址 + Ctrl+Shift+O 转系统
//     浏览器 + 弹窗内二级链接可打开; 主窗口 Ctrl+左键直接系统浏览器打开链接
//     (信令通道 __shell/*, 见 handle_shell_route)
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
use std::time::Duration;

use rust_embed::RustEmbed;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{DownloadEvent, NewWindowResponse, NewWindowFeatures, PageLoadEvent};
use tauri::{Manager, Runtime, UriSchemeContext, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// 裁剪后的 FluffyChat Web 静态资源 (由 prepare.sh 生成到 build/web_offline/)
/// 注意: 相对路径基于 $CARGO_MANIFEST_DIR = src-tauri/
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../build/web_offline/"]
struct WebAssets;

/// 运行时生成的 config.json 内容 (等价 Go 版 renderConfig)
static CONFIG_JSON: OnceLock<String> = OnceLock::new();

/// 用于给 window.open 弹出的子窗口生成唯一 label
static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

/// 内部浏览器弹窗的 label 前缀 (见 open_web_popup)
const POPUP_PREFIX: &str = "web-popup-";

/// 同时存活的弹窗上限。防页面 (广告/跳转链) 无限开窗 —— 多层嵌套弹窗
/// 很容易演化成“看不见也关不掉”的后台 webview, 也是用户遇到过的音频幽灵来源。
const MAX_POPUPS: usize = 8;

/// 弹窗销毁前的缓冲: 让 about:blank 真正开始装载, 旧文档 (音频/WS/定时器) 随之终止
const POPUP_TEARDOWN_DELAY: Duration = Duration::from_millis(200);

fn is_popup(label: &str) -> bool {
    label.starts_with(POPUP_PREFIX)
}

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
        .register_uri_scheme_protocol("fc", handle_protocol)
        .setup(|app| {
            setup_main_window(app)?;
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    // 关闭主窗口 → 隐藏到托盘 (托盘常驻, 对齐 Go 版语义)
                    api.prevent_close();
                    let _ = window.hide();
                } else if is_popup(window.label()) {
                    // 关闭弹窗 → 显式两级回收 (先断文档, 再销毁窗口), 见 teardown_popup
                    api.prevent_close();
                    let app = window.app_handle().clone();
                    teardown_popup(&app, window.label());
                }
            }
        })
        .on_menu_event(|app, event| {
            // 窗口菜单 (web-popup) 事件: id 形如 "<label>::<action>" (见 popup_menu);
            // 托盘菜单 id (open/quit) 不含 "::", 走 setup_tray 自己的回调
            let Some((label, action)) = event.id.as_ref().split_once("::") else {
                return;
            };
            let Some(w) = app.get_webview_window(label) else {
                return;
            };
            let js = match action {
                "copy-url" => "window.__fcCopyUrl && __fcCopyUrl()",
                "open-external" => "window.__fcOpenExt && __fcOpenExt()",
                "back" => "history.back()",
                "forward" => "history.forward()",
                "reload" => "location.reload()",
                _ => return,
            };
            let _ = w.eval(js);
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

/// fc:// 自定义协议处理器: 动态 config.json + 嵌入静态资源 + __shell/* 信令路由
fn handle_protocol<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let path = request.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // __shell/*: 页面 JS → 壳的原生命令通道 (见 handle_shell_route)
    if let Some(resp) = handle_shell_route(&ctx, path, &request) {
        return resp;
    }

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

    // 登录会话/E2EE 密钥/消息缓存都在 WebView 用户数据目录 (UDF, 见 resolve_data_dir);
    // 弹窗窗口必须与主窗口用同一 UDF (同一浏览器进程, 共享存储)。
    let data_dir = resolve_data_dir(app);
    let popup_data_dir = data_dir.clone();

    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("FluffyChat")
        .inner_size(480.0, 820.0)
        .min_inner_size(360.0, 600.0)
        .center()
        // Ctrl+左键聊天链接 → 直接系统浏览器打开 (见 MAIN_INIT_JS)
        .initialization_script(MAIN_INIT_JS)
        .on_download(make_download_handler())
        .on_new_window(move |url, features| {
            open_web_popup(&handle, &url, features, popup_data_dir.clone())
        });

    if let Some(dir) = data_dir {
        builder = builder.data_directory(dir);
    }

    builder.build()?;
    Ok(())
}

/// 解析 WebView 用户数据目录 (UDF):
///
/// - None: exe 所在目录可写 → 用 WebView2 默认 UDF (exe 旁 `<exe名>.exe.WebView2`),
///   便携语义, 原地替换 exe 不丢登录/E2EE 数据 (现有用户数据保持原位, 无需迁移);
/// - Some: exe 目录不可写 (如被放进 Program Files) → 回退到
///   %LOCALAPPDATA%/<identifier>, 否则 WebView2 会因无法创建 UDF (Access Denied) 启动失败。
///
/// 仅 Windows 需要: WebKitGTK 默认写用户 XDG 目录, WKWebView 不使用该配置。
fn resolve_data_dir(app: &tauri::App) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        // 探测写权限: 创建后删除临时探测文件
        let probe = exe_dir.join(format!(".fc-probe-{}", std::process::id()));
        let writable = match std::fs::File::create(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        };
        if writable {
            None
        } else {
            app.path().app_local_data_dir().ok()
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        None
    }
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
    data_dir: Option<PathBuf>,
) -> NewWindowResponse<tauri::Wry> {
    // 弹窗数量上限: 超出后改交系统浏览器, 避免页面无限嵌套开窗导致无法追踪的
    // 后台 webview (此时 Deny 会让 window.open 拿到 null, 与浏览器拦截语义一致)
    let live_popups = handle
        .webview_windows()
        .keys()
        .filter(|label| is_popup(label.as_str()))
        .count();
    if live_popups >= MAX_POPUPS {
        if matches!(url.scheme(), "http" | "https") {
            open_in_system_browser(url.as_str());
        }
        return NewWindowResponse::Deny;
    }

    let label = format!(
        "{POPUP_PREFIX}{}",
        NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed)
    );
    let initial_title = host_of(url).unwrap_or_else(|| "FluffyChat".into());

    // 原生菜单栏 (页面: 复制地址/外部打开/导航); 失败不阻塞弹窗本身
    let menu = popup_menu(handle, &label).ok();

    let mut builder = WebviewWindowBuilder::new(
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
    // 弹窗内页面能力补齐: Ctrl+L 复制/查看地址, Ctrl+Shift+O 转系统浏览器 (见 POPUP_INIT_JS)
    .initialization_script(popup_init_script())
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

    if let Some(menu) = menu {
        builder = builder.menu(menu);
    }

    // 与主窗口共用同一 UDF: 共享存储/cookie, 且 exe 目录不可写时弹窗才能创建成功
    let builder = if let Some(dir) = data_dir.clone() {
        builder.data_directory(dir)
    } else {
        builder
    };

    // 弹窗内的 target=_blank 二级链接: wry 对未挂 on_new_window 的 webview 一律
    // Deny (表现为点击无反应), 这里递归开新弹窗 (label 计数器保证唯一)
    let popup_handle = handle.clone();
    let builder = builder.on_new_window(move |url, features| {
        open_web_popup(&popup_handle, &url, features, data_dir.clone())
    });

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

// ---------------------------------------------------------------------------
// 弹窗生命周期回收
//
// 问题背景 (用户报告): 内部浏览器弹窗点叉关掉后, 后台似乎没回收 webview ——
// 极端时就变成“看不见、也关不掉”的幽灵页面 (例如一次跳转后自动播放了媒体)。
//
// WebView2 的渲染进程是否退出, 取决于 CoreWebView2Controller.Close() 何时执行
// (wry 在 webview 被 drop 时调用)。若窗口对象在某个环节被多持有一份引用, 或
// Close() 被推迟到 GC, 旧文档就仍然活着: 媒体继续出声、WebSocket/定时器继续跑。
//
// 所以不再依赖“关窗口 = 自动回收”这一单一链路, 改成显式三步回收:
//   1) navigate("about:blank") —— 用空文档替掉当前文档, 音频/视频/WS/定时器随
//      文档一起终止。即使 controller 回收被拖到最后, 也不会再有任何声音;
//   2) hide() —— 用户感知上就是已关闭 (不会出现点了叉还停留一小会儿的窗口);
//   3) 延时 destroy() —— 绕过 CloseRequested 直接销毁窗口 (不会递归到本函数),
//      让 runtime 从窗口表里移除 webview 并执行 controller.Close()。
//
// 如果 1/3 任一环节因平台原因失效, 页面已变空白且不可见, 托盘菜单
// 「关闭所有弹窗」可再兜底一次。
// ---------------------------------------------------------------------------

fn about_blank() -> tauri::Url {
    "about:blank".parse().expect("valid about:blank url")
}

/// 回收单个弹窗 (见上方说明)。可从任意线程调用: Tauri 的窗口操作都是消息式派发。
fn teardown_popup(app: &tauri::AppHandle, label: &str) {
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    let _ = win.navigate(about_blank());
    let _ = win.hide();

    let app = app.clone();
    let label = label.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(POPUP_TEARDOWN_DELAY);
        // 重试几轮: destroy 是消息式派发, 窗口表移除有延迟; 且 label 由原子计数器
        // 生成永不复用 —— 重试绝不会误伤后来新建的弹窗
        for _ in 0..3 {
            let Some(win) = app.get_webview_window(&label) else {
                return;
            };
            // destroy 不触发 CloseRequested → 不会与上面的 prevent_close 形成回环
            let _ = win.destroy();
            std::thread::sleep(Duration::from_millis(120));
        }
    });
}

/// 关闭所有残留弹窗 (托盘菜单入口; 也是“后台幽灵 webview”的人工兜底)
fn close_all_popups(app: &tauri::AppHandle) {
    let labels: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|label| is_popup(label.as_str()))
        .cloned()
        .collect();
    for label in labels {
        teardown_popup(app, &label);
    }
}

/// 弹窗的原生菜单栏: 给"地址可获取/可外部打开"提供可发现的鼠标入口。
/// 菜单项 id 编码 "<窗口label>::<action>", 全局 on_menu_event 按此路由回对应弹窗
/// (MenuEvent 不携带窗口信息); 动作全部 eval 到注入脚本/页面 API 上执行。
///
/// 菜单文字中的快捷键仅为提示, 不注册真 accelerator —— webview 聚焦时原生
/// accelerator 不可靠, 且与注入 JS 双重处理会触发两次; 键盘路径由 POPUP_INIT_JS
/// 接管。Alt+←/→、Ctrl+R/F5 为 WebView2 原生 accelerator, 亦无需处理。
fn popup_menu(handle: &tauri::AppHandle, label: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let item = |action: &str, text: &str| {
        MenuItem::with_id(handle, format!("{label}::{action}"), text, true, None::<&str>)
    };
    let copy = item("copy-url", "复制页面地址\tCtrl+L")?;
    let open_ext = item("open-external", "在系统浏览器打开\tCtrl+Shift+O")?;
    let sep = PredefinedMenuItem::separator(handle)?;
    let back = item("back", "后退\tAlt+←")?;
    let forward = item("forward", "前进\tAlt+→")?;
    let reload = item("reload", "重新加载\tCtrl+R")?;

    let page = Submenu::with_id(handle, "page", "页面(&P)", true)?;
    page.append_items(&[&copy, &open_ext, &sep, &back, &forward, &reload])?;
    Menu::with_items(handle, &[&page])
}

/// 文件下载处理 (on_download), 主窗口与弹窗共用。
///
/// wry 的默认行为是"允许下载但完全静默": 无进度、无完成提示, 文件悄悄落在
/// WebView 默认下载目录, 用户感知上就是"点了保存没反应"。这里统一改为:
///
/// 1. 保存到系统"下载"目录, 同名文件自动追加序号 (photo.png → photo (1).png)
/// 2. 下载完成后在文件管理器中定位该文件, 作为明确的完成反馈
///
/// 注 1: FluffyChat Web 保存文件 = 取回字节 → blob URL → `<a download>` 点击,
/// 各 WebView 均将此作为标准下载事件处理, 建议文件名即 download 属性值。
/// 注 2: 不弹"另存为"对话框: on_download 在主线程同步回调, 必须当场给出
/// 允许/拒绝 (tauri-plugin-dialog 的 blocking API 明确禁止主线程使用, 会死锁;
/// blob URL 也无法由 Rust 侧接手重新下载), 因此固定保存到下载目录。
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
    // Windows: 必须用 raw_arg 手工拼 `explorer /select,"路径"`。普通 .arg() 会按 CRT 规则
    // 给含空格的参数整体加引号 —— 重名自动加序号的 "photo (1).png" 必含空格 —— explorer
    // 解析不了 "/select,C:\a b.png", 会退回打开默认文件夹 (如 Documents) 而非下载目录。
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if path.is_file() {
            let _ = std::process::Command::new("explorer.exe")
                .raw_arg(format!("/select,\"{}\"", path.display()))
                .spawn();
        } else if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
            // 防御: 文件已不在 (极端情况) 时至少打开其所在目录
            let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
        }
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

// ---------------------------------------------------------------------------
// __shell/* 信令通道: 页面 JS → 壳的原生能力 (复制地址/外部打开/关窗)
//
// 弹窗加载的是第三方 https 页面, 向 fc:// (Windows 上实为 http://fc.localhost) 发
// fetch 属 mixed content 会被拦, 唯一可靠通道是顶层导航 (location.href); 主窗口
// (自家 origin) 则直接用相对路径 fetch。__shell/open 的 url 参数仅白名单 http/https,
// 防止恶意页面借壳触发任意系统协议处理器 (ms-msdt:/file: 等)。
// ---------------------------------------------------------------------------

/// 弹窗初始化脚本: 注入到每个 web-popup 窗口的每一页面。
/// - Ctrl+L: 复制当前页地址 (失败也以 toast 展示完整地址, 保证地址总是可获取)
/// - Ctrl+Shift+O: 当前页转交系统浏览器打开
///
/// 其余导航键 (Alt+←/→、Ctrl+R/F5) 交给 WebView2 原生 accelerator, 不在 JS 重复处理
/// (避免与原生行为叠加双次后退/刷新)。
const POPUP_INIT_JS: &str = r#"(function () {
  var SHELL = '__SHELL_BASE__';

  function toast(msg) {
    try {
      var d = document.createElement('div');
      d.textContent = msg;
      var s = d.style;
      s.position = 'fixed'; s.left = '50%'; s.bottom = '24px';
      s.transform = 'translateX(-50%)'; s.maxWidth = '80vw';
      s.overflowWrap = 'anywhere'; s.background = 'rgba(0,0,0,.78)';
      s.color = '#fff'; s.font = '13px/1.6 system-ui,sans-serif';
      s.padding = '6px 14px'; s.borderRadius = '6px'; s.zIndex = '2147483647';
      (document.body || document.documentElement).appendChild(d);
      setTimeout(function () { d.remove(); }, 4000);
    } catch (e) {}
  }

  function copyText(text) {
    var p;
    try {
      // 非安全上下文 (http 第三方页) 下 navigator.clipboard 为 undefined, 走 catch
      p = navigator.clipboard.writeText(text).then(function () { return true; }, function () { return false; });
    } catch (e) {
      p = Promise.resolve(false);
    }
    return p.then(function (ok) {
      if (ok) return true;
      try {
        var t = document.createElement('textarea');
        t.value = text;
        t.style.position = 'fixed'; t.style.opacity = '0';
        (document.body || document.documentElement).appendChild(t);
        t.focus(); t.select();
        ok = document.execCommand('copy');
        t.remove();
        return !!ok;
      } catch (e) { return false; }
    });
  }

  window.__fcCopyUrl = function () {
    copyText(location.href).then(function (ok) {
      toast(ok ? '地址已复制: ' + location.href : '复制失败, 当前地址: ' + location.href);
    });
  };

  window.__fcOpenExt = function () {
    location.href = SHELL + '/__shell/open?url=' + encodeURIComponent(location.href);
  };

  window.addEventListener('keydown', function (e) {
    var k = (e.key || '').toLowerCase();
    var mod = e.ctrlKey || e.metaKey;
    if (mod && k === 'l') { e.preventDefault(); window.__fcCopyUrl(); }
    else if (mod && e.shiftKey && k === 'o') { e.preventDefault(); window.__fcOpenExt(); }
  }, true);
})();
"#;

/// __shell/open 成功页: 明示已转交系统浏览器, 并把地址展示出来 (可选中复制)
const SHELL_OPENED_PAGE: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>已在系统浏览器打开</title>
<style>
  body { margin:0; min-height:100vh; display:flex; align-items:center; justify-content:center;
         font:14px/1.7 system-ui,"Segoe UI","Microsoft YaHei",sans-serif;
         background:#fafafa; color:#24292f; }
  @media (prefers-color-scheme: dark) { body { background:#0d1117; color:#e6edf3; } }
  .card { max-width:560px; margin:24px; padding:24px 28px; border:1px solid #d0d7de; border-radius:12px; }
  .ok { font-size:15px; font-weight:600; }
  .url { margin:14px 0 18px; padding:8px 12px; background:rgba(125,125,125,.12);
         border-radius:6px; font:12px/1.6 ui-monospace,Consolas,monospace;
         word-break:break-all; cursor:text; }
  button { padding:6px 18px; font:13px system-ui; border:1px solid #d0d7de; border-radius:6px;
           background:#f6f8fa; color:inherit; cursor:pointer; }
  @media (prefers-color-scheme: dark) { button { background:#21262d; border-color:#30363d; } }
</style>
</head>
<body>
<div class="card">
  <div class="ok">✓ 已在系统浏览器中打开</div>
  <div class="url" id="u" title="点击全选">{URL}</div>
  <button onclick="closeWin()">关闭本窗口</button>
</div>
<script>
  var u = document.getElementById('u');
  u.addEventListener('click', function () {
    var r = document.createRange(); r.selectNodeContents(u);
    var s = window.getSelection(); s.removeAllRanges(); s.addRange(r);
  });
  // 弹窗由 window.open 而来, window.close() 通常有效;
  // 兼若仍存活则走信令由壳关闭 (非脚本打开的窗口 window.close 无效)
  function closeWin() {
    try { window.close(); } catch (e) {}
    setTimeout(function () { location.href = '/__shell/close'; }, 150);
  }
</script>
</body>
</html>
"#;

/// __shell/open 失败页 (参数缺失/非 http(s) scheme)
const SHELL_OPEN_FAILED_PAGE: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>无法打开</title>
<style>
  body { margin:0; min-height:100vh; display:flex; align-items:center; justify-content:center;
         font:14px/1.7 system-ui,"Segoe UI","Microsoft YaHei",sans-serif;
         background:#fafafa; color:#24292f; }
  @media (prefers-color-scheme: dark) { body { background:#0d1117; color:#e6edf3; } }
  .card { max-width:560px; margin:24px; padding:24px 28px; border:1px solid #d0d7de; border-radius:12px; }
  .url { margin-top:14px; padding:8px 12px; background:rgba(125,125,125,.12);
         border-radius:6px; font:12px/1.6 ui-monospace,Consolas,monospace;
         word-break:break-all; cursor:text; }
</style>
</head>
<body>
<div class="card">
  <div>无法在外部打开该地址：仅支持 http/https 链接</div>
  <div class="url">{URL}</div>
</div>
</body>
</html>
"#;

/// __shell/close 应答页: 窗口即将关闭的过渡页 (顺带 window.close 双保险)
const SHELL_CLOSING_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>关闭中…</title>
<body style="font:13px system-ui">正在关闭窗口…</body>
<script>try { window.close(); } catch (e) {}</script>
"#;

/// __shell/* 信令路由 (匹配则返回 Some):
/// - __shell/open?url=… : 转交系统浏览器打开, 返回确认页
///   (弹窗: 顶层导航到达, 页面被替换为确认页本身就是操作反馈;
///   主窗口: fetch 调用, 确认页仅作为响应体不会被展示)
/// - __shell/close : 关闭发起请求的 webview 窗口
fn handle_shell_route<R: Runtime>(
    ctx: &UriSchemeContext<'_, R>,
    path: &str,
    request: &Request<Vec<u8>>,
) -> Option<Response<Vec<u8>>> {
    match path {
        "__shell/open" => {
            let raw = query_param(request.uri().query().unwrap_or_default(), "url");
            let target = raw
                .as_deref()
                .and_then(|s| tauri::Url::parse(s).ok())
                .filter(|u| matches!(u.scheme(), "http" | "https"));
            match target {
                Some(u) => {
                    open_in_system_browser(u.as_str());
                    Some(respond(
                        200,
                        "text/html; charset=utf-8",
                        shell_page(SHELL_OPENED_PAGE, u.as_str()).into_bytes(),
                    ))
                }
                None => Some(respond(
                    200,
                    "text/html; charset=utf-8",
                    shell_page(SHELL_OPEN_FAILED_PAGE, raw.as_deref().unwrap_or("(空)")).into_bytes(),
                )),
            }
        }
        "__shell/close" => {
            let label = ctx.webview_label().to_string();
            let app = ctx.app_handle().clone();
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(w) = app2.get_webview_window(&label) {
                    let _ = w.close();
                }
            });
            Some(respond(
                200,
                "text/html; charset=utf-8",
                SHELL_CLOSING_PAGE.as_bytes().to_vec(),
            ))
        }
        _ => None,
    }
}

/// 主窗口初始化脚本 (仅主窗口, 不注入弹窗):
/// 按住 Ctrl 点聊天链接 → 直接用系统浏览器打开 (绕过内部弹窗)。
///
/// 实现: 在 pointerdown 瞬间快照修饰键 (capture 阶段, 先于 Flutter 手势竞技场,
/// 无竞态), window.open 时消费快照 —— 从物理点击到 url_launcher 调 window.open
/// 间有异步延迟, 届时同步事件早已不可读; 读即清避免残留影响后续键盘激活。
///
/// - 返回 null 无副作用: url_launcher 的 web 实现不检查 window.open 返回值
/// - 信令用相对路径 fetch (主窗口 origin 即 fc://localhost, 恒同源; Windows 上
///   实为 http://fc.localhost); 失败退化为 Image 像素信标 (不依赖 CORS)
/// - SSO 登录链接 (/_matrix/.../login/sso) 豁免: 转外部浏览器会断登录回跳
const MAIN_INIT_JS: &str = r#"(function () {
  var forceExternal = false;
  function snap(e) { forceExternal = !!(e.ctrlKey || e.metaKey); }
  window.addEventListener('pointerdown', snap, true);
  window.addEventListener('mousedown', snap, true);

  var SSO = /\/_matrix\/.*\/login\/sso/;
  var origOpen = window.open.bind(window);
  window.open = function (url, target, features) {
    var u = String(url || '');
    if (forceExternal && /^https?:/i.test(u) && !SSO.test(u)) {
      forceExternal = false;
      var signal = '/__shell/open?url=' + encodeURIComponent(u);
      try {
        fetch(signal, { cache: 'no-store' }).catch(function () {
          var i = new Image(); i.src = signal;
        });
      } catch (e) {
        var i = new Image(); i.src = signal;
      }
      return null;
    }
    return origOpen(url, target, features);
  };
})();
"#;

/// 弹窗初始化脚本实例化: 按平台替换信令地址前缀
fn popup_init_script() -> String {
    POPUP_INIT_JS.replace("__SHELL_BASE__", shell_base())
}

/// fc:// 的真实访问前缀。
/// WebView2 不支持真正的自定义 scheme, wry 用 http://{scheme}.localhost 的
/// workaround 转发 (custom_protocol_workaround.rs); 弹窗 (第三方页面) 发信令
/// 必须用平台对应的真实地址, 否则 Windows 上顶层导航到 fc:// 会被拒。
/// 注: 不能开 useHttpsScheme —— origin 变 https 会丢存量 localStorage/IndexedDB。
fn shell_base() -> &'static str {
    if cfg!(target_os = "windows") {
        "http://fc.localhost"
    } else {
        "fc://localhost"
    }
}

/// 系统默认浏览器打开 URL (调用处已白名单 http/https)
#[cfg(target_os = "windows")]
fn open_in_system_browser(url: &str) {
    // ShellExecuteW 走系统关联 (默认浏览器), 与 explorer.exe 拼参数不同,
    // 对任意 URL 形态都可靠
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            lpverb: *const u16,
            lpfile: *const u16,
            lpparameters: *const u16,
            lpdirectory: *const u16,
            nshow: i32,
        ) -> isize;
    }
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let verb = wide("open");
    let file = wide(url);
    unsafe {
        ShellExecuteW(0, verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
    }
}
#[cfg(target_os = "linux")]
fn open_in_system_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}
#[cfg(target_os = "macos")]
fn open_in_system_browser(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// 把模板页的 {URL} 占位符替换为转义后的地址
fn shell_page(tpl: &str, url: &str) -> String {
    tpl.replace("{URL}", &html_escape(url))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
}

/// 提取查询参数 (application/x-www-form-urlencoded 风格: '+' = 空格, %XX 解码)
fn query_param(query: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    query
        .split('&')
        .find_map(|p| p.strip_prefix(&prefix))
        .map(percent_decode)
}

fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => match (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            ) {
                (Some(hi), Some(lo)) => {
                    out.push(hi * 16 + lo);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 系统托盘: 左键唤醒窗口, 右键菜单 (显示 / 关闭所有弹窗 / 退出)
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开 FluffyChat", true, None::<&str>)?;
    // 弹窗回收兜底入口: 弹窗是后开的独立窗口, 一旦躲到别的窗口后面/失去可见性,
    // 用户没有别的入口能关掉它们 (主窗口的隐藏逻辑不影响弹窗)
    let close_popups =
        MenuItem::with_id(app, "close-popups", "关闭所有弹窗", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &close_popups, &sep, &quit])?;

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
            "close-popups" => close_all_popups(app),
            "quit" => {
                // 退出前先回收弹窗 (显式关闭 WebView2 controller), 再延时退出;
                // 直接 exit 会让弹窗的 renderer 在进程收尾阶段被粗暴终止
                close_all_popups(app);
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(POPUP_TEARDOWN_DELAY);
                    app.exit(0);
                });
            }
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
