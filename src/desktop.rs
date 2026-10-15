//! Windows desktop mode: the UI in a native window (WebView2), a notification-area icon, one instance.
//!
//! The HTTP server and the engine run on a background thread (tokio); the main thread owns the window
//! and the tray icon, as Windows requires.

use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::WindowExtWindows;
use tao::window::{Icon as WindowIcon, WindowBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, MB_ICONERROR, MB_OK, MessageBoxW, SetForegroundWindow,
};

use crate::config::Settings;
use crate::engine::EngineHandle;
use crate::instance;
use crate::server::{self, StartError};

const WEBVIEW2_DOWNLOAD: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
/// The engine waits at most 5 s for the `stopped` announces: leave it a little more.
const QUIT_TIMEOUT: Duration = Duration::from_secs(6);

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Show,
}

/// What the server thread reports once it is listening (or why it is not).
enum Started {
    Ready { engine: EngineHandle, url: String, runtime: tokio::runtime::Handle },
    Failed(StartError),
}

/// A blocking error dialog (there is no console in the GUI subsystem).
pub fn error_box(text: &str) {
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("ratiod"));
    // SAFETY: both buffers are NUL-terminated and outlive the call; a null owner window is allowed.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

pub fn run() -> ExitCode {
    let settings = match Settings::from_env() {
        Ok(settings) => settings,
        Err(error) => return fatal(&format!("Réglage invalide :\n\n{error:#}")),
    };
    if let Err(error) = std::fs::create_dir_all(&settings.data_dir) {
        return fatal(&format!(
            "Impossible d'utiliser le dossier de données {} :\n\n{error}",
            settings.data_dir.display()
        ));
    }
    // Checked before the log file is recreated, so that a second launch never wipes the first one's log.
    if let Err(error) = TcpListener::bind(settings.bind)
        && error.kind() == std::io::ErrorKind::AddrInUse
    {
        return address_in_use(&settings);
    }
    server::init_file_logging(&settings.data_dir);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (quit_tx, quit_rx) = tokio::sync::oneshot::channel::<()>();
    let server_settings = settings.clone();
    let show_proxy = Mutex::new(proxy.clone());
    let on_show: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if let Ok(proxy) = show_proxy.lock() {
            let _ = proxy.send_event(UserEvent::Show);
        }
    });
    std::thread::spawn(move || serve(server_settings, on_show, started_tx, quit_rx));

    let (engine, url, runtime) = match started_rx.recv() {
        Ok(Started::Ready { engine, url, runtime }) => (engine, url, runtime),
        Ok(Started::Failed(StartError::AddrInUse(_))) => return address_in_use(&settings),
        Ok(Started::Failed(error)) => {
            tracing::error!("{error}");
            return fatal(&format!("ratiod n'a pas pu démarrer :\n\n{error}"));
        }
        Err(_) => return fatal("ratiod n'a pas pu démarrer : le serveur s'est arrêté brutalement."),
    };

    let window = match WindowBuilder::new()
        .with_title("ratiod")
        .with_inner_size(LogicalSize::new(1100.0, 760.0))
        .with_min_inner_size(LogicalSize::new(420.0, 500.0))
        .with_window_icon(WindowIcon::from_rgba(crate::icon::rgba(64), 64, 64).ok())
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(error) => return fail(&runtime, &engine, &format!("Impossible de créer la fenêtre :\n\n{error}")),
    };
    // No drag-and-drop handler on purpose: wry would replace WebView2's own handling, which is what
    // delivers HTML5 `dragenter`/`drop` events (with the files) to the page.
    // WebView2 keeps its profile in the data folder, not next to the exe (which may be read-only).
    let mut web_context = wry::WebContext::new(Some(settings.data_dir.join("webview2")));
    let _webview = match wry::WebViewBuilder::new_with_web_context(&mut web_context).with_url(&url).build(&window) {
        Ok(webview) => webview,
        Err(error) => {
            tracing::error!("cannot create the webview: {error}");
            return fail(
                &runtime,
                &engine,
                &format!(
                    "ratiod a besoin de Microsoft WebView2 pour afficher son interface, et il est introuvable ou \
                 inutilisable.\n\nInstalle-le depuis :\n{WEBVIEW2_DOWNLOAD}\n\n({error})"
                ),
            );
        }
    };

    let open_item = MenuItem::new("Ouvrir ratiod", true, None);
    let quit_item = MenuItem::new("Quitter", true, None);
    let (open_id, quit_id) = (open_item.id().clone(), quit_item.id().clone());
    let menu = Menu::new();
    if let Err(error) = menu.append_items(&[&open_item, &quit_item]) {
        tracing::warn!("cannot build the tray menu: {error}");
    }
    let icon = match tray_icon::Icon::from_rgba(crate::icon::rgba(32), 32, 32) {
        Ok(icon) => icon,
        Err(error) => {
            return fail(&runtime, &engine, &format!("Icône de la zone de notification invalide :\n\n{error}"));
        }
    };
    let tray = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(icon)
        .with_tooltip("ratiod")
        .build()
    {
        Ok(tray) => tray,
        Err(error) => {
            return fail(
                &runtime,
                &engine,
                &format!("Impossible de créer l'icône de la zone de notification :\n\n{error}"),
            );
        }
    };

    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Tray(event));
    }));

    let mut tray = Some(tray);
    let mut quit_tx = Some(quit_tx);
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let show = || {
            window.set_visible(true);
            window.set_minimized(false);
            window.set_focus();
            // set_focus alone is often refused by Windows' foreground lock (the taskbar button just flashes).
            // SAFETY: the HWND belongs to the live window owned by this closure.
            unsafe {
                SetForegroundWindow(window.hwnd() as HWND);
            }
        };
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => window.set_visible(false),
            Event::UserEvent(UserEvent::Show) => show(),
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            })) => show(),
            Event::UserEvent(UserEvent::Menu(menu_event)) if menu_event.id == open_id => show(),
            Event::UserEvent(UserEvent::Menu(menu_event)) if menu_event.id == quit_id => {
                window.set_visible(false);
                if runtime.block_on(tokio::time::timeout(QUIT_TIMEOUT, engine.shutdown())).is_err() {
                    tracing::warn!("the engine did not stop in time");
                }
                tracing::info!("bye");
                // Removes the icon now instead of leaving a ghost one until the mouse passes over it.
                tray.take();
                if let Some(quit) = quit_tx.take() {
                    let _ = quit.send(());
                }
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}

/// Server thread: runs the tokio runtime and keeps the server alive until the main thread asks to quit.
fn serve(
    settings: Settings,
    on_show: Arc<dyn Fn() + Send + Sync>,
    started: std::sync::mpsc::Sender<Started>,
    quit: tokio::sync::oneshot::Receiver<()>,
) {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = started.send(Started::Failed(StartError::Other(anyhow::Error::new(error))));
            return;
        }
    };
    let handle = runtime.handle().clone();
    runtime.block_on(async move {
        match server::start(&settings, Some(on_show)).await {
            Ok(running) => {
                let _ = started.send(Started::Ready {
                    engine: running.engine.clone(),
                    url: running.url.clone(),
                    runtime: handle,
                });
                let mut running = running;
                tokio::select! {
                    _ = quit => {}
                    result = &mut running.server => match result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => tracing::error!("HTTP server failed: {error}"),
                        Err(error) => tracing::error!("HTTP server crashed: {error}"),
                    },
                }
                drop(running);
            }
            Err(error) => {
                let _ = started.send(Started::Failed(error));
            }
        }
    });
}

/// The UI port is taken: a running ratiod gets its window back, anything else is an error.
fn address_in_use(settings: &Settings) -> ExitCode {
    let addr = server::loopback_addr(settings.bind);
    // This process is the foreground one: let the running instance take the foreground in turn.
    // SAFETY: plain Win32 call without pointers.
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
    if instance::notify_running(addr) {
        return ExitCode::SUCCESS;
    }
    fatal(&format!(
        "Le port {} est déjà utilisé par un autre programme.\n\nFerme ce programme, ou choisis une autre adresse \
         avec la variable d'environnement RATIOD_BIND.",
        settings.bind.port()
    ))
}

/// Fatal error once the server runs: stop the engine first so the trackers get their `stopped` announces.
fn fail(runtime: &tokio::runtime::Handle, engine: &EngineHandle, text: &str) -> ExitCode {
    if runtime.block_on(tokio::time::timeout(QUIT_TIMEOUT, engine.shutdown())).is_err() {
        tracing::warn!("the engine did not stop in time");
    }
    fatal(text)
}

fn fatal(text: &str) -> ExitCode {
    error_box(text);
    ExitCode::FAILURE
}
