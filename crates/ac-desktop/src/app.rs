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

include!("icon.rs");

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

    // Only one copy runs the server; a second one just shows the first one's interface.
    let lock = File::options().create(true).read(true).write(true).truncate(false).open(dir.join("app.lock")).ok();
    let primary = lock.as_ref().is_some_and(|f| f.try_lock().is_ok());

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
    let builder = WebViewBuilder::new();
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
    let quit_item = MenuItem::with_id("quit", "結束 CrewConvo", true, None);
    let _ = menu.append_items(&[
        &open_item,
        &browser_item,
        &PredefinedMenuItem::separator(),
        &autostart_item,
        &folder_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ]);
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
    event_loop.run(move |event, _, control_flow| {
        let _keep = (&tray, &lock, &open_item, &browser_item, &folder_item, &quit_item);
        *control_flow = ControlFlow::Wait;
        let mut quit = || {
            if !quitting {
                quitting = true;
                window.set_visible(false);
                stop.notify_one();
                // Never hang on exit: the database gets a few seconds to stop cleanly.
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_secs(12));
                    std::process::exit(0);
                });
            }
        };
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
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Show) => {
                window.set_visible(true);
                window.set_minimized(false);
                window.set_focus();
            }
            Event::UserEvent(UserEvent::Menu(id)) => match id.as_str() {
                "open" => {
                    window.set_visible(true);
                    window.set_minimized(false);
                    window.set_focus();
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
                "quit" => quit(),
                _ => {}
            },
            _ => {}
        }
    });
}
