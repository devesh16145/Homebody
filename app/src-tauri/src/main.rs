//! Sandbox-ai desktop shell.
//!
//! Deliberately thin: it owns the windows and the menu-bar icon, and forwards everything else to
//! `sai-core`. The UI calls one command, `sai`, with `{cmd, args}` exactly as in docs/ipc.md, and
//! listens on one event channel, `sai:event`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use sai_core::{Engine, Event, Indicator};
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
async fn sai(app: AppHandle, window: WebviewWindow, shell: tauri::State<'_, Shell>, cmd: String, args: Option<Value>) -> Result<Value, String> {
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
        _ => shell.engine.dispatch(&cmd, args).await,
    }
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
        let _ = app.emit("sai:event", Event::Navigate { tab: tab.to_string() });
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
    let main = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("main.html".into())).title("Sandbox-ai").inner_size(980.0, 680.0).min_inner_size(760.0, 520.0).visible(false);
    #[cfg(target_os = "macos")]
    let main = main.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
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

    let glance = WebviewWindowBuilder::new(app, "glance", WebviewUrl::App("glance.html".into()))
        .title("Sandbox-ai")
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
        .title("Sandbox-ai")
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
    let open = MenuItem::with_id(app, "open", "Open Sandbox-ai", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Sandbox-ai", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id("tray")
        .icon(tray_icon(Indicator::Attention))
        .icon_as_template(true)
        .tooltip("Sandbox-ai")
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
                        let _ = tray.set_tooltip(Some(format!("Sandbox-ai: {}", s.sentence)));
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
            let _ = app.emit("sai:event", &event);
        }
    });
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![sai])
        .setup(|app| {
            let handle = app.handle().clone();
            // One engine per user: if `sai daemon` or another copy of the app is running, bow out.
            if tauri::async_runtime::block_on(sai_core::control::is_running()) {
                eprintln!("Sandbox-ai is already running.");
                std::process::exit(0);
            }
            let engine = tauri::async_runtime::block_on(Engine::start())?;
            app.manage(Shell { engine: engine.clone(), glance_hidden_at: Mutex::new(None), indicator: Mutex::new(None) });

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            build_windows(&handle)?;
            build_tray(&handle)?;
            pump_events(handle.clone(), engine.clone());

            let control_engine = engine.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = sai_core::control::serve(control_engine).await {
                    eprintln!("sandbox-ai: control socket: {e}");
                }
            });

            // First run, or protection needs attention: open the main window so it is not missed.
            let first = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(2500)).await;
                let status = first.state::<Shell>().engine.status();
                if status.setup_needed || std::env::var("SAI_SHOW_MAIN").is_ok() {
                    show_main(&first, None);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Sandbox-ai")
        .run(|app, event| {
            // Keep running in the menu bar when the last window is hidden.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            if let tauri::RunEvent::Exit = event {
                let _ = std::fs::remove_file(sai_core::paths::control_socket());
                let _ = app;
            }
        });
}
