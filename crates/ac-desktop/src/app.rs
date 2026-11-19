use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use include_dir::{Dir, include_dir};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use wry::WebViewBuilder;

use crate::update::{self, Release};

include!("icon.rs");

/// How often a running app looks for a newer release.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

/// The built web interface, carried inside the program.
static WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../web/dist");

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "CrewConvo";

enum UserEvent {
    /// The server accepts requests at this address.
    Ready(String),
    Failed(String),
    /// The server and its database have shut down.
    Stopped,
    Show,
    Menu(String),
    /// A button on one of our own pages was pressed.
    Button(String),
    /// A check for a newer release ended. `asked`: the user asked for it from the tray menu.
    Checked { result: Result<Option<Release>, String>, asked: bool },
    /// The new program file is in place of this one; what is left is to restart.
    Installed,
    InstallFailed(String),
}

/// What the window shows, as far as updating is concerned. Buttons only count on the page
/// they belong to: the web interface (or anything inside it) cannot press them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    App,
    Offer,
    /// "Coding agents are running — update anyway?"
    Confirm,
    Installing,
    /// A message with a single way back.
    Notice,
}

/// `%LOCALAPPDATA%\CrewConvo`: settings (.env), data, logs.
fn app_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir())
        .join("CrewConvo")
}

fn set_default(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    if std::env::var_os(key).is_none_or(|v| v.is_empty()) {
        // Called before any other thread exists.
        unsafe { std::env::set_var(key, value) };
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn page(title: &str, body: &str, spinner: bool) -> String {
    let spin = if spinner { "<div class=\"spin\"></div>" } else { "" };
    format!(
        "<!doctype html><html lang=\"zh-Hant\"><meta charset=\"utf-8\"><style>\
         html,body{{height:100%;margin:0}}\
         body{{display:flex;align-items:center;justify-content:center;background:#f7f7f9;color:#1d1c1d;\
         font-family:'Segoe UI','Microsoft JhengHei',sans-serif}}\
         .box{{max-width:560px;padding:32px;text-align:center}}\
         h1{{font-size:22px;margin:16px 0 8px}}p{{color:#555;line-height:1.6;margin:6px 0;white-space:pre-wrap}}\
         .spin{{width:36px;height:36px;margin:0 auto;border:4px solid #dcdcec;border-top-color:#4f46e5;\
         border-radius:50%;animation:s 1s linear infinite}}@keyframes s{{to{{transform:rotate(360deg)}}}}\
         @media(prefers-color-scheme:dark){{body{{background:#1a1d21;color:#e8e8e8}}p{{color:#aaa}}}}\
         </style><div class=\"box\">{spin}<h1>{}</h1>{body}</div></html>",
        escape(title)
    )
}

/// One of our own pages with buttons: (what pressing it means, label, is the main one).
fn dialog(title: &str, body: &str, buttons: &[(&str, &str, bool)]) -> String {
    let mut html = String::from(
        "<style>.row{margin-top:22px;display:flex;gap:10px;justify-content:center;flex-wrap:wrap}\
         button{font:inherit;padding:9px 20px;border-radius:8px;border:1px solid #c9c9d6;background:transparent;\
         color:inherit;cursor:pointer}button.main{background:#4f46e5;border-color:#4f46e5;color:#fff}\
         a{color:#4f46e5;cursor:pointer;text-decoration:underline}.v{font-family:Consolas,monospace}</style>",
    );
    html.push_str(body);
    html.push_str("<div class=\"row\">");
    for (id, label, main) in buttons {
        let class = if *main { " class=\"main\"" } else { "" };
        html.push_str(&format!(
            "<button id=\"{id}\"{class} onclick=\"window.ipc.postMessage('{id}')\">{}</button>",
            escape(label)
        ));
    }
    html.push_str("</div>");
    page(title, &html, false)
}

fn version_label() -> String {
    match update::current() {
        Some(v) => format!("版本 {v}"),
        None => "開發版本（不檢查更新）".to_string(),
    }
}

fn offer_page(release: &Release, replaceable: bool) -> String {
    let current = escape(update::current().unwrap_or("?"));
    let new = escape(&release.version);
    let notes = "<p><a onclick=\"window.ipc.postMessage('update:notes')\">這個版本改了什麼</a></p>";
    if replaceable {
        let body = format!(
            "<p>目前版本 <span class=\"v\">{current}</span> → 新版本 <span class=\"v\">{new}</span></p>{notes}\
             <p>按「立即更新」會下載新版、驗證後取代目前的程式並重新啟動，資料會保留。不按就不會下載任何東西。</p>"
        );
        dialog("有新版本可以更新", &body, &[("update:install", "立即更新", true), ("update:later", "稍後", false)])
    } else {
        // The program sits somewhere it cannot be replaced from within (no write access).
        let body = format!(
            "<p>目前版本 <span class=\"v\">{current}</span> → 新版本 <span class=\"v\">{new}</span></p>{notes}\
             <p>CrewConvo 所在的資料夾無法寫入，所以不能自動更新。請按「前往下載」取得新版，結束 CrewConvo 後用新的檔案取代舊的。</p>"
        );
        dialog("有新版本可以更新", &body, &[("update:download", "前往下載", true), ("update:later", "稍後", false)])
    }
}

/// Starts the program file again after an update and lets it wait for us to exit.
fn start_updated(exe: &Path) -> std::io::Result<()> {
    std::process::Command::new(exe).arg("--updated").spawn().map(|_| ())
}

fn autostart_enabled() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_NAME))
        .is_ok()
}

fn set_autostart(on: bool) -> std::io::Result<()> {
    let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if on {
        let exe = std::env::current_exe()?;
        key.set_value(RUN_NAME, &format!("\"{}\" --hidden", exe.display()))
    } else {
        match key.delete_value(RUN_NAME) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn open_in_shell(target: &std::ffi::OsStr) {
    let _ = std::process::Command::new("explorer").arg(target).spawn();
}

/// Unpacks the web interface next to the data, replacing the previous version's files.
fn unpack_web(dir: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    WEB.extract(dir)
}

fn local_url() -> String {
    let bind = std::env::var("BIND_ADDR").unwrap_or_default();
    let port = bind.rsplit(':').next().filter(|p| p.parse::<u16>().is_ok()).unwrap_or("8090").to_string();
    format!("http://127.0.0.1:{port}")
}

pub fn run() {
    let dir = app_dir();
    let _ = std::fs::create_dir_all(dir.join("logs"));
    let _ = std::env::set_current_dir(&dir);
    // Optional settings: %LOCALAPPDATA%\CrewConvo\.env (same names as the server's .env).
    let _ = dotenvy::from_path(dir.join(".env"));
    set_default("CREWCONVO_EDITION", "personal");
    set_default("DATA_DIR", dir.join("data"));
    set_default("WEB_DIST", dir.join("web"));
    set_default("WEBVIEW2_USER_DATA_FOLDER", dir.join("webview"));
    let hidden = std::env::args().any(|a| a == "--hidden");
    // Started by the previous version at the end of an update: it is still shutting down.
    let updated = std::env::args().any(|a| a == "--updated");

    // Only one copy runs the server; a second one just shows the first one's interface.
    let lock = File::options().create(true).read(true).write(true).truncate(false).open(dir.join("app.lock")).ok();
    let mut primary = lock.as_ref().is_some_and(|f| f.try_lock().is_ok());
    if updated && !primary {
        // Wait for the old program to let go (it stops the database first) instead of
        // becoming a mere window onto a server that is about to disappear.
        for _ in 0..120 {
            std::thread::sleep(Duration::from_millis(500));
            if lock.as_ref().is_some_and(|f| f.try_lock().is_ok()) {
                primary = true;
                break;
            }
        }
    }
    let exe = std::env::current_exe().ok();
    if primary && let Some(exe) = &exe {
        // The previous version's file, renamed aside by the update that started us.
        update::clean_up(exe);
    }

    if primary {
        if let Ok(log) = File::options().create(true).append(true).open(dir.join("logs").join("crewconvo.log")) {
            let filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn,tower_http=warn"));
            let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(log)).try_init();
        }
        if let Err(e) = unpack_web(&dir.join("web")) {
            tracing::error!(error = %e, "could not unpack the web interface");
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("CrewConvo")
        .with_inner_size(LogicalSize::new(1240.0, 820.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .with_window_icon(tao::window::Icon::from_rgba(icon_rgba(64), 64, 64).ok())
        .with_visible(!(hidden && primary))
        .build(&event_loop)
        .expect("create window");

    let loading = page(
        "正在啟動 CrewConvo…",
        "<p>第一次啟動要下載內建資料庫，大約需要一分鐘，之後幾秒就會好。</p>",
        true,
    );
    let buttons = event_loop.create_proxy();
    let builder = WebViewBuilder::new().with_ipc_handler(move |request: wry::http::Request<String>| {
        let _ = buttons.send_event(UserEvent::Button(request.body().clone()));
    });
    let builder = if primary { builder.with_html(loading) } else { builder.with_url(local_url()) };
    let webview = builder.build(&window).expect("create web view");

    if !primary {
        event_loop.run(move |event, _, control_flow| {
            let _keep = (&webview, &lock);
            *control_flow = ControlFlow::Wait;
            if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
                *control_flow = ControlFlow::Exit;
            }
        });
    }

    // The server (and the database it manages) runs on its own threads.
    let stop = Arc::new(tokio::sync::Notify::new());
    {
        let (stop, proxy) = (stop.clone(), event_loop.create_proxy());
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(anyhow::Error::from).and_then(
                |rt| {
                    let ready = proxy.clone();
                    rt.block_on(ac_server::serve_from_env(async move { stop.notified().await }, move |url| {
                        let _ = ready.send_event(UserEvent::Ready(url.to_string()));
                    }))
                },
            );
            if let Err(e) = result {
                tracing::error!(error = %format!("{e:#}"), "CrewConvo could not start");
                let _ = proxy.send_event(UserEvent::Failed(format!("{e:#}")));
            }
            let _ = proxy.send_event(UserEvent::Stopped);
        });
    }

    // System tray: the app keeps running there after the window is closed, so the coding agents
    // stay connected.
    let menu = Menu::new();
    let open_item = MenuItem::with_id("open", "開啟 CrewConvo", true, None);
    let browser_item = MenuItem::with_id("browser", "在瀏覽器開啟", true, None);
    let folder_item = MenuItem::with_id("folder", "開啟資料夾（設定、資料、記錄）", true, None);
    let autostart_item = CheckMenuItem::with_id("autostart", "開機時自動啟動", true, autostart_enabled(), None);
    let update_item = MenuItem::with_id("update", "檢查更新", true, None);
    let version_item = MenuItem::with_id("version", version_label(), false, None);
    let quit_item = MenuItem::with_id("quit", "結束 CrewConvo", true, None);
    let _ = menu.append_items(&[
        &open_item,
        &browser_item,
        &PredefinedMenuItem::separator(),
        &autostart_item,
        &folder_item,
        &PredefinedMenuItem::separator(),
        &update_item,
        &version_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ]);

    // Looking for a newer release: once the server is up, then every few hours, and when asked
    // from the menu. Failures are only logged — being offline is not worth a message.
    let (ask_check, check_requests) = std::sync::mpsc::channel::<bool>();
    {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            loop {
                // `true`: the user asked; a timeout: it is time for the regular check.
                let asked = match check_requests.recv_timeout(CHECK_EVERY) {
                    Ok(asked) => asked,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
                    Err(_) => return,
                };
                if !asked && !update::check_enabled() {
                    continue;
                }
                let result = rt.block_on(update::check());
                match &result {
                    Ok(Some(r)) => tracing::info!(version = %r.version, "a newer release is available"),
                    Ok(None) => tracing::info!("update check: this is the latest release"),
                    Err(e) => tracing::warn!(error = %e, "update check failed"),
                }
                let _ = proxy.send_event(UserEvent::Checked { result, asked });
            }
        });
    }
    let tray = TrayIconBuilder::new()
        .with_tooltip("CrewConvo")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).expect("tray icon"))
        .build();
    if let Err(e) = &tray {
        tracing::warn!(error = %e, "could not create the tray icon");
    }
    let has_tray = tray.is_ok();
    {
        let proxy = event_loop.create_proxy();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            let clicked = matches!(
                e,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            );
            if clicked {
                let _ = proxy.send_event(UserEvent::Show);
            }
        }));
        let proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(e.id.0.clone()));
        }));
    }

    let mut url: Option<String> = None;
    let mut quitting = false;
    let mut told_about_tray = false;
    let mut screen = Screen::App;
    // The newer release we know of, and the one the user said "later" to (until the next start).
    let mut offer: Option<Release> = None;
    let mut later: Option<String> = None;
    let mut first_check = true;
    // Opening the window looks for a newer release too (not more often than every few minutes),
    // and what that check finds is offered straight away, like the check at start.
    let mut last_check: Option<std::time::Instant> = None;
    let mut opened_check = false;
    // Set once the new program file is in place: started when the server has shut down.
    let restart: Arc<Mutex<Option<PathBuf>>> = Arc::default();
    // A page that must stay up after the server stopped (the restart failed).
    let hold = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let installer = event_loop.create_proxy();
    event_loop.run(move |event, _, control_flow| {
        let _keep = (&tray, &lock, &open_item, &browser_item, &folder_item, &quit_item, &update_item, &version_item);
        *control_flow = ControlFlow::Wait;
        let mut quit = || {
            if !quitting {
                quitting = true;
                window.set_visible(false);
                stop.notify_one();
                // Never hang on exit: the database gets a few seconds to stop cleanly.
                let (restart, hold) = (restart.clone(), hold.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(12));
                    if let Some(exe) = restart.lock().unwrap().take() {
                        let _ = start_updated(&exe);
                    }
                    if !hold.load(std::sync::atomic::Ordering::SeqCst) {
                        std::process::exit(0);
                    }
                });
            }
        };
        let bring_up = || {
            window.set_visible(true);
            window.set_minimized(false);
            window.set_focus();
        };
        // The offer is shown when the window is opened, not in the middle of what the user is doing.
        let offer_due = |offer: &Option<Release>, later: &Option<String>| {
            offer.as_ref().filter(|r| later.as_deref() != Some(r.version.as_str())).cloned()
        };
        let replaceable = exe.as_deref().is_some_and(update::can_replace);
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if has_tray && url.is_some() {
                    window.set_visible(false);
                    if !told_about_tray {
                        told_about_tray = true;
                        tracing::info!("window closed; still running in the system tray");
                    }
                } else {
                    quit();
                }
            }
            Event::UserEvent(UserEvent::Ready(u)) => {
                let _ = webview.load_url(&u);
                url = Some(u);
                // The check at start (a no-op for development builds and with UPDATE_CHECK=false).
                let _ = ask_check.send(false);
            }
            Event::UserEvent(UserEvent::Failed(msg)) => {
                let body = format!(
                    "<p>{}</p><p>詳細記錄：{}</p>",
                    escape(&msg),
                    escape(&app_dir().join("logs").join("crewconvo.log").display().to_string())
                );
                let _ = webview.load_html(&page("CrewConvo 無法啟動", &body, false));
                window.set_visible(true);
            }
            Event::UserEvent(UserEvent::Stopped) => {
                if quitting {
                    let pending = restart.lock().unwrap().take();
                    match pending.map(|exe| start_updated(&exe).map_err(|e| (exe, e))) {
                        Some(Err((exe, e))) => {
                            // The new file is in place but will not start: put the old one back.
                            tracing::error!(error = %e, "the updated program did not start");
                            let restored = update::restore(&exe);
                            hold.store(true, std::sync::atomic::Ordering::SeqCst);
                            let body = format!(
                                "<p>新版本無法啟動：{}</p><p>{}</p><p>請結束後重新開啟 CrewConvo。</p>",
                                escape(&e.to_string()),
                                if restored.is_ok() { "已經還原成原本的版本。" } else { "原本的程式檔在旁邊的 .old 檔案。" }
                            );
                            let _ = webview.load_html(&dialog("更新沒有完成", &body, &[("app:exit", "結束", true)]));
                            screen = Screen::Notice;
                            window.set_visible(true);
                        }
                        _ => *control_flow = ControlFlow::Exit,
                    }
                }
            }
            Event::UserEvent(UserEvent::Show) => {
                bring_up();
                if screen == Screen::App
                    && url.is_some()
                    && let Some(r) = offer_due(&offer, &later)
                {
                    let _ = webview.load_html(&offer_page(&r, replaceable));
                    screen = Screen::Offer;
                } else if url.is_some()
                    && offer.is_none()
                    && last_check.is_none_or(|t| t.elapsed() >= Duration::from_secs(300))
                {
                    last_check = Some(std::time::Instant::now());
                    opened_check = true;
                    let _ = ask_check.send(false);
                }
            }
            Event::UserEvent(UserEvent::Checked { result, asked }) => {
                last_check = Some(std::time::Instant::now());
                let at_start = (first_check || opened_check) && !asked;
                if !asked {
                    first_check = false;
                    opened_check = false;
                }
                match result {
                    Ok(Some(r)) => {
                        offer = Some(r.clone());
                        // Asked for, or the check right after opening the app: show it now.
                        // A later routine check waits until the window is opened again.
                        let now = asked || (at_start && window.is_visible() && offer_due(&offer, &later).is_some());
                        if now && screen == Screen::App && url.is_some() {
                            let _ = webview.load_html(&offer_page(&r, replaceable));
                            screen = Screen::Offer;
                            if asked {
                                bring_up();
                            }
                        }
                    }
                    Ok(None) | Err(_) if !asked || screen != Screen::App => {}
                    Ok(None) => {
                        let body = format!("<p>{}</p>", escape(&version_label()));
                        let _ = webview.load_html(&dialog("已經是最新版本", &body, &[("update:back", "返回", true)]));
                        screen = Screen::Notice;
                        bring_up();
                    }
                    Err(e) => {
                        let body = format!("<p>{}</p><p>請確認網路連線後再試一次。</p>", escape(&e));
                        let _ = webview.load_html(&dialog("無法檢查更新", &body, &[("update:back", "返回", true)]));
                        screen = Screen::Notice;
                        bring_up();
                    }
                }
            }
            Event::UserEvent(UserEvent::Button(id)) => {
                let back = |screen: &mut Screen| {
                    if let Some(u) = &url {
                        let _ = webview.load_url(u);
                    }
                    *screen = Screen::App;
                };
                let install = |screen: &mut Screen| {
                    let (Some(release), Some(exe)) = (offer.clone(), exe.clone()) else { return };
                    let body = format!("<p>正在下載並驗證 {}，完成後會自動重新啟動。</p>", escape(&release.version));
                    let _ = webview.load_html(&page("正在更新 CrewConvo…", &body, true));
                    *screen = Screen::Installing;
                    let proxy = installer.clone();
                    std::thread::spawn(move || {
                        let result = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| e.to_string())
                            .and_then(|rt| rt.block_on(update::install(&release, &exe)));
                        let _ = proxy.send_event(match result {
                            Ok(()) => UserEvent::Installed,
                            Err(e) => UserEvent::InstallFailed(e),
                        });
                    });
                };
                // A button only counts on the page that shows it.
                match (screen, id.as_str()) {
                    (Screen::Offer | Screen::Confirm, "update:later") => {
                        later = offer.as_ref().map(|r| r.version.clone());
                        back(&mut screen);
                    }
                    (Screen::Notice, "update:back") => back(&mut screen),
                    (Screen::Notice, "app:exit") => std::process::exit(0),
                    (Screen::Offer | Screen::Notice, "update:notes" | "update:download") => {
                        // Opened in the browser, and only when it really is a GitHub page.
                        if let Some(r) = offer.as_ref().filter(|r| update::url_allowed(&r.notes_url, false)) {
                            open_in_shell(r.notes_url.as_ref());
                        }
                    }
                    (Screen::Offer, "update:install") => {
                        let running = ac_server::local::active_runs();
                        if running > 0 {
                            let body = format!(
                                "<p>現在有 {running} 個 Coding Agent 正在執行。更新會重新啟動 CrewConvo，它們會被中止，做到一半的工作不會完成。</p>\
                                 <p>要現在更新，還是等它們做完？</p>"
                            );
                            let buttons =
                                [("update:install-anyway", "中止它們並更新", true), ("update:later", "稍後", false)];
                            let _ = webview.load_html(&dialog("Coding Agent 還在執行", &body, &buttons));
                            screen = Screen::Confirm;
                        } else {
                            install(&mut screen);
                        }
                    }
                    (Screen::Confirm, "update:install-anyway") => install(&mut screen),
                    _ => {}
                }
            }
            Event::UserEvent(UserEvent::Installed) => {
                tracing::info!("update installed; restarting");
                *restart.lock().unwrap() = exe.clone();
                let _ = webview.load_html(&page("正在重新啟動 CrewConvo…", "<p>新版本已經就緒。</p>", true));
                quit();
            }
            Event::UserEvent(UserEvent::InstallFailed(msg)) => {
                tracing::warn!(error = %msg, "update failed; the program was not changed");
                let body = format!(
                    "<p>{}</p><p>目前的版本沒有被更動，可以繼續使用。</p>\
                     <p>也可以<a onclick=\"window.ipc.postMessage('update:download')\">自己下載新版</a>。</p>",
                    escape(&msg)
                );
                let _ = webview.load_html(&dialog("更新沒有完成", &body, &[("update:back", "返回", true)]));
                screen = Screen::Notice;
            }
            Event::UserEvent(UserEvent::Menu(id)) => match id.as_str() {
                "open" => {
                    bring_up();
                    if screen == Screen::App
                        && url.is_some()
                        && let Some(r) = offer_due(&offer, &later)
                    {
                        let _ = webview.load_html(&offer_page(&r, replaceable));
                        screen = Screen::Offer;
                    }
                }
                "browser" => {
                    if let Some(u) = &url {
                        open_in_shell(u.as_ref());
                    }
                }
                "folder" => open_in_shell(app_dir().as_os_str()),
                "autostart" => {
                    let want = !autostart_enabled();
                    if let Err(e) = set_autostart(want) {
                        tracing::warn!(error = %e, "could not change the start-with-Windows setting");
                    }
                    autostart_item.set_checked(autostart_enabled());
                }
                "update" if screen == Screen::App && url.is_some() => {
                    if update::current().is_some() {
                        // The answer always gets a page, also "this is the latest".
                        later = None;
                        let _ = ask_check.send(true);
                    } else {
                        let body = "<p>這是自己編譯的開發版本，沒有版本號，所以不會檢查更新。</p>";
                        let _ = webview.load_html(&dialog("開發版本", body, &[("update:back", "返回", true)]));
                        screen = Screen::Notice;
                        bring_up();
                    }
                }
                "quit" => quit(),
                _ => {}
            },
            _ => {}
        }
    });
}
