//! Homebody desktop shell.
//!
//! Deliberately thin: it owns the windows and the menu-bar icon, and forwards everything else to
//! `homebody-core`. The UI calls one command, `homebody`, with `{cmd, args}` exactly as in docs/ipc.md, and
//! listens on one event channel, `homebody:event`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use homebody_core::{Engine, Event, Indicator};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::{Effect, EffectState, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

const GLANCE: (f64, f64) = (340.0, 480.0);
const PROMPT: (f64, f64) = (380.0, 128.0);

struct Shell {
    engine: Engine,
    /// When Glance last hid itself because it lost focus. A click on the menu-bar icon makes it
    /// lose focus first; without this, that same click would immediately reopen it.
    glance_hidden_at: Mutex<Option<Instant>>,
    indicator: Mutex<Option<Indicator>>,
}

#[tauri::command]
async fn homebody(app: AppHandle, window: WebviewWindow, shell: tauri::State<'_, Shell>, cmd: String, args: Option<Value>) -> Result<Value, String> {
    let args = args.unwrap_or(Value::Null);
    match cmd.as_str() {
        "open_main" => {
            show_main(&app, args.get("tab").and_then(Value::as_str));
            if window.label() == "glance" {
                let _ = window.hide();
            }
            Ok(Value::Null)
        }
        "hide_window" => {
            let _ = window.hide();
            Ok(Value::Null)
        }
        "quit" => {
            shell.engine.shutdown().await;
            app.exit(0);
            Ok(Value::Null)
        }
        "pick_folder" => tokio::task::spawn_blocking(|| pick("choose folder with prompt \"Choose a project folder to bring in\"")).await.map_err(|e| e.to_string())?,
        "pick_program" => tokio::task::spawn_blocking(|| pick("choose file with prompt \"Choose the program to protect\" default location (POSIX file \"/Applications\")")).await.map_err(|e| e.to_string())?,
        "install_and_restart" => {
            let res = shell.engine.dispatch("install_app", Value::Null).await?;
            let path = res.get("path").and_then(Value::as_str).unwrap_or("").to_string();
            // Start the installed copy once this one has gone, then stop cleanly.
            let _ = std::process::Command::new("/bin/sh").args(["-c", "sleep 2; exec /usr/bin/open -n \"$0\"", &path]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
            shell.engine.shutdown().await;
            app.exit(0);
            Ok(Value::Null)
        }
        "get_login_item" => Ok(serde_json::json!({ "on": login_item_path().map(|p| p.exists()).unwrap_or(false) })),
        "set_login_item" => {
            set_login_item(args.get("on").and_then(Value::as_bool).unwrap_or(false))?;
            Ok(serde_json::json!({ "on": login_item_path().map(|p| p.exists()).unwrap_or(false) }))
        }
        _ => shell.engine.dispatch(&cmd, args).await,
    }
}

/// The system's own chooser for a folder or a file. Returns null when the user cancels.
/// Uses the chooser built into macOS rather than a dialog library, to keep the app small.
fn pick(what: &str) -> Result<Value, String> {
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "tell me to activate", "-e", &format!("POSIX path of ({what})")])
        .output()
        .map_err(|e| format!("The chooser could not be opened: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        // -128 is "user cancelled".
        return if err.contains("-128") { Ok(Value::Null) } else { Err(format!("The chooser failed: {}", err.trim())) };
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let path = if path.len() > 1 { path.trim_end_matches('/').to_string() } else { path };
    Ok(if path.is_empty() { Value::Null } else { Value::String(path) })
}

const LOGIN_ITEM: &str = "app.homebody.mac";

fn login_item_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/LaunchAgents").join(format!("{LOGIN_ITEM}.plist")))
}

/// Open Homebody when the user logs in: a per-user launch entry pointing at this copy of the
/// app. No administrator rights are involved, and switching it off removes the file again.
fn set_login_item(on: bool) -> Result<(), String> {
    let path = login_item_path().ok_or("Your home folder could not be found.")?;
    if !on {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("The login entry could not be removed: {e}")),
        };
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy();
    if exe.contains('<') || exe.contains('&') {
        return Err("Move the app to a folder with a simpler name first.".into());
    }
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n  <key>Label</key><string>{LOGIN_ITEM}</string>\n  <key>ProgramArguments</key><array><string>{exe}</string><string>--hidden</string></array>\n  <key>RunAtLoad</key><true/>\n  <key>LimitLoadToSessionType</key><string>Aqua</string>\n  <key>ProcessType</key><string>Interactive</string>\n</dict></plist>\n"
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("The login entry could not be saved: {e}"))?;
    }
    std::fs::write(&path, plist).map_err(|e| format!("The login entry could not be saved: {e}"))
}

fn tray_icon(ind: Indicator) -> Image<'static> {
    let bytes: &'static [u8] = match ind {
        Indicator::Private => include_bytes!("../icons/tray-private.png"),
        Indicator::Watching => include_bytes!("../icons/tray-watching.png"),
        Indicator::Asking => include_bytes!("../icons/tray-asking.png"),
        Indicator::Blocked => include_bytes!("../icons/tray-blocked.png"),
        Indicator::Off => include_bytes!("../icons/tray-off.png"),
        Indicator::Attention => include_bytes!("../icons/tray-attention.png"),
    };
    Image::from_bytes(bytes).expect("tray icon")
}

fn show_main(app: &AppHandle, tab: Option<&str>) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    if let Some(tab) = tab {
        let _ = app.emit("homebody:event", Event::Navigate { tab: tab.to_string() });
    }
}

fn toggle_glance(app: &AppHandle, icon: tauri::Rect) {
    let Some(w) = app.get_webview_window("glance") else { return };
    let shell = app.state::<Shell>();
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    if shell.glance_hidden_at.lock().unwrap().map(|t| t.elapsed() < Duration::from_millis(300)).unwrap_or(false) {
        return;
    }
    let scale = w.scale_factor().unwrap_or(2.0);
    let pos = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let width = GLANCE.0 * scale;
    let mut x = pos.x + size.width / 2.0 - width / 2.0;
    let y = pos.y + size.height + 6.0 * scale;
    if let Ok(Some(m)) = w.current_monitor().or_else(|_| app.primary_monitor()) {
        let (left, right) = (m.position().x as f64, m.position().x as f64 + m.size().width as f64);
        x = x.clamp(left + 8.0 * scale, right - width - 8.0 * scale);
    }
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.show();
    let _ = w.set_focus();
}

fn show_prompt(app: &AppHandle) {
    let Some(w) = app.get_webview_window("prompt") else { return };
    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let x = m.position().x as f64 + m.size().width as f64 - (PROMPT.0 + 16.0) * scale;
        let y = m.position().y as f64 + 40.0 * scale;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
    let _ = w.show();
}

fn build_windows(app: &AppHandle) -> tauri::Result<()> {
    let main = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("home.html".into())).title("Homebody").inner_size(980.0, 680.0).min_inner_size(760.0, 520.0).visible(false);
    #[cfg(target_os = "macos")]
    // The window buttons sit inside the floating sidebar (8 pt from the window edge, rounded 18):
    // 14 pt in from its left and 16 pt down from its top, clear of its rounded corner.
    let main = main.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true).traffic_light_position(tauri::LogicalPosition::new(22.0, 24.0));
    let main = main.build()?;
    // Closing the main window hides it: the app lives in the menu bar.
    let handle = app.clone();
    let main_for_event = main.clone();
    main.on_window_event(move |e| {
        if let WindowEvent::CloseRequested { api, .. } = e {
            api.prevent_close();
            let _ = main_for_event.hide();
            #[cfg(target_os = "macos")]
            let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    });

    let glance = WebviewWindowBuilder::new(app, "glance", WebviewUrl::App("menubar.html".into()))
        .title("Homebody")
        .inner_size(GLANCE.0, GLANCE.1)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .effects(EffectsBuilder::new().effect(Effect::Popover).state(EffectState::Active).radius(14.0).build())
        .build()?;
    let handle = app.clone();
    let glance_for_event = glance.clone();
    glance.on_window_event(move |e| {
        if let WindowEvent::Focused(false) = e {
            let _ = glance_for_event.hide();
            *handle.state::<Shell>().glance_hidden_at.lock().unwrap() = Some(Instant::now());
        }
    });

    WebviewWindowBuilder::new(app, "prompt", WebviewUrl::App("prompt.html".into()))
        .title("Homebody")
        .inner_size(PROMPT.0, PROMPT.1)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .effects(EffectsBuilder::new().effect(Effect::HudWindow).state(EffectState::Active).radius(16.0).build())
        .build()?;
    Ok(())
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Homebody", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Homebody", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id("tray")
        .icon(tray_icon(Indicator::Attention))
        .icon_as_template(true)
        .tooltip("Homebody")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app, None),
            "quit" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    app.state::<Shell>().engine.shutdown().await;
                    app.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                toggle_glance(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    Ok(())
}

/// Forward core events to every window, and keep the native bits (icon, prompt window) in step.
fn pump_events(app: AppHandle, engine: Engine) {
    tauri::async_runtime::spawn(async move {
        let mut rx = engine.subscribe();
        loop {
            let event = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            match &event {
                Event::Status(s) => {
                    let shell = app.state::<Shell>();
                    let mut current = shell.indicator.lock().unwrap();
                    if *current != Some(s.indicator) {
                        *current = Some(s.indicator);
                        if let Some(tray) = app.tray_by_id("tray") {
                            let _ = tray.set_icon(Some(tray_icon(s.indicator)));
                            let _ = tray.set_icon_as_template(true);
                        }
                    }
                    if let Some(tray) = app.tray_by_id("tray") {
                        let _ = tray.set_tooltip(Some(format!("Homebody: {}", s.sentence)));
                    }
                }
                Event::Ask(_) => show_prompt(&app),
                Event::Show { window, tab } => {
                    match window.as_str() {
                        "glance" => {
                            if let Some(rect) = app.tray_by_id("tray").and_then(|t| t.rect().ok().flatten()) {
                                toggle_glance(&app, rect);
                            }
                        }
                        "prompt" => show_prompt(&app),
                        _ => show_main(&app, tab.as_deref()),
                    }
                    continue;
                }
                _ => {}
            }
            let _ = app.emit("homebody:event", &event);
        }
    });
}

/// Tell the user something from a process that has no window of its own.
fn alert(message: &str) {
    let text: String = message.chars().filter(|c| !c.is_control()).map(|c| if c == '"' || c == '\\' { '\'' } else { c }).collect();
    let _ = std::process::Command::new("/usr/bin/osascript").args(["-e", &format!("display alert \"Homebody\" message \"{text}\"")]).output();
}

/// An answer that means "not ready yet, try again in a moment" rather than "no".
fn not_ready(error: &str) -> bool {
    error.contains("still being checked") || error.contains("not running")
}

fn main() {
    // A launcher ("<App> (protected)" in the Dock) starts us with: --open-protected <app id>.
    let args: Vec<String> = std::env::args().collect();
    let wanted = args.iter().position(|a| a == "--open-protected").and_then(|i| args.get(i + 1)).cloned();
    if let Some(id) = &wanted {
        if tauri::async_runtime::block_on(homebody_core::control::is_running()) {
            // Homebody is already running: hand the request over and leave.
            tauri::async_runtime::block_on(async {
                let mut last = String::new();
                for _ in 0..40 {
                    match homebody_core::control::call("app_open", serde_json::json!({ "appId": id })).await {
                        Ok(_) => return,
                        Err(e) if not_ready(&e) => last = e,
                        Err(e) => return alert(&e),
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                alert(&last);
            });
            return;
        }
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![homebody])
        .setup(move |app| {
            let handle = app.handle().clone();
            // One engine per user: if `homebody daemon` or another copy of the app is running, bow out.
            if tauri::async_runtime::block_on(homebody_core::control::is_running()) {
                eprintln!("Homebody is already running.");
                std::process::exit(0);
            }
            let engine = tauri::async_runtime::block_on(Engine::start())?;
            app.manage(Shell { engine: engine.clone(), glance_hidden_at: Mutex::new(None), indicator: Mutex::new(None) });
            if let Ok(exe) = std::env::current_exe() {
                engine.set_opener(&exe.to_string_lossy());
                // "Open at login" follows the app when it has moved, and moves to the new name
                // from the entry written before the app was called Homebody.
                let legacy = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/LaunchAgents/com.sandboxai.app.plist"));
                let had_legacy = legacy.as_ref().map(|p| std::fs::remove_file(p).is_ok()).unwrap_or(false);
                if had_legacy || login_item_path().map(|p| p.exists()).unwrap_or(false) {
                    let _ = set_login_item(true);
                }
            }
            // Started by a launcher while not running yet: start up as usual, then open the app.
            if let Some(id) = wanted.clone() {
                let e = engine.clone();
                tauri::async_runtime::spawn(async move {
                    let mut last = String::new();
                    for _ in 0..40 {
                        match e.dispatch("app_open", serde_json::json!({ "appId": id })).await {
                            Ok(_) => return,
                            Err(err) if not_ready(&err) => last = err,
                            Err(err) => return alert(&err),
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    alert(&last);
                });
            }

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            build_windows(&handle)?;
            build_tray(&handle)?;
            pump_events(handle.clone(), engine.clone());

            let control_engine = engine.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = homebody_core::control::serve(control_engine).await {
                    eprintln!("homebody: control socket: {e}");
                }
            });

            // Asked to stop from outside (kill, logout, shutdown): stop protected apps we started
            // and leave nothing behind, exactly as Quit does.
            let stopping = handle.clone();
            tauri::async_runtime::spawn(async move {
                use tokio::signal::unix::{signal, SignalKind};
                let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup())) else { return };
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                    _ = hup.recv() => {}
                }
                stopping.state::<Shell>().engine.shutdown().await;
                stopping.exit(0);
            });

            // Opening the app shows its window: someone who opens it wants to see it. The quiet
            // starts are the ones nobody asked for by hand: at login, or through a launcher.
            // Either way the window opens when protection needs attention, so that is not missed.
            let quiet = std::env::args().any(|a| a == "--hidden") || wanted.is_some();
            let first = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(if quiet { 2500 } else { 600 })).await;
                let status = first.state::<Shell>().engine.status();
                if !quiet || status.setup_needed || std::env::var("HOMEBODY_SHOW_MAIN").is_ok() {
                    show_main(&first, None);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Homebody")
        .run(|app, event| {
            // Keep running in the menu bar when the last window is hidden.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            // Opened again while already running (from Applications, Spotlight or the Dock):
            // there is nothing to start, so show the window instead of doing nothing visible.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &event {
                show_main(app, None);
            }
            if let tauri::RunEvent::Exit = event {
                let _ = std::fs::remove_file(homebody_core::paths::control_socket());
                let _ = app;
            }
        });
}
