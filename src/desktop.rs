//! Windows desktop mode: the UI in a native window (WebView2), a notification-area icon, one instance.
//!
//! The HTTP server and the engine run on a background thread (tokio); the main thread owns the window
//! and the tray icon, as Windows requires.

use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::WindowExtWindows;
use tao::window::{Icon as WindowIcon, Window, WindowBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE, ICoreWebView2Environment, ICoreWebView2Environment5,
};
use webview2_com::{BrowserProcessExitedEventHandler, ProcessFailedEventHandler};
use windows_core::Interface;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, DestroyWindow, MB_ICONERROR, MB_ICONWARNING, MB_OK, MESSAGEBOX_STYLE,
    MessageBoxW, SetForegroundWindow,
};
use wry::{WebContext, WebView, WebViewExtWindows};

use crate::config::Settings;
use crate::engine::EngineHandle;
use crate::instance;
use crate::locale::pick;
use crate::recovery::{Decision, RebuildGate, RecoveryPolicy};
use crate::server::{self, StartError};

const WEBVIEW2_DOWNLOAD: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
/// The engine waits at most 5 s for the `stopped` announces: leave it a little more.
const QUIT_TIMEOUT: Duration = Duration::from_secs(6);
/// Building the webview is tried this many times: an overlay injected into WebView2 can crash it while it starts.
const BUILD_ATTEMPTS: u32 = 3;
const BUILD_RETRY_PAUSE: Duration = Duration::from_millis(500);
/// Display crashes recovered within `RECOVERY_WINDOW` before giving up (a crash loop is not recoverable).
const MAX_RECOVERIES: usize = 3;
const RECOVERY_WINDOW: Duration = Duration::from_secs(60);
/// How long a rebuild waits for WebView2 to report that the crashed browser process is gone.
const BROWSER_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Show,
    /// A WebView2 process of the webview number `generation` failed.
    WebViewFailed {
        generation: u64,
        kind: COREWEBVIEW2_PROCESS_FAILED_KIND,
    },
    /// The webview number `generation` can be built: the crashed browser process is gone, or waiting for it
    /// `timed_out`.
    RebuildReady {
        generation: u64,
        timed_out: bool,
    },
}

/// How to get the display back after a WebView2 process failure.
enum Repair {
    /// The browser process is gone: the webview is dead and must be built again.
    Recreate,
    /// Only the page's renderer is gone or stuck: the webview reloads it.
    Reload,
}

/// What the server thread reports once it is listening (or why it is not).
enum Started {
    Ready { engine: EngineHandle, url: String, runtime: tokio::runtime::Handle },
    Failed(StartError),
}

/// A blocking error dialog (there is no console in the GUI subsystem).
pub fn error_box(text: &str) {
    message_box(text, MB_ICONERROR);
}

/// A non-blocking warning dialog: shown from its own thread so the event loop (window, tray) keeps running.
fn warning_box(text: String) {
    std::thread::spawn(move || message_box(&text, MB_ICONWARNING));
}

fn message_box(text: &str, icon: MESSAGEBOX_STYLE) {
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("ratiod"));
    // SAFETY: both buffers are NUL-terminated and outlive the call; a null owner window is allowed.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | icon);
    }
}

pub fn run() -> ExitCode {
    let settings = match Settings::from_env() {
        Ok(settings) => settings,
        Err(error) => {
            return fatal(&pick(format!("Réglage invalide :\n\n{error:#}"), format!("Invalid setting:\n\n{error:#}")));
        }
    };
    if let Err(error) = std::fs::create_dir_all(&settings.data_dir) {
        let dir = settings.data_dir.display();
        return fatal(&pick(
            format!("Impossible d'utiliser le dossier de données {dir} :\n\n{error}"),
            format!("Cannot use the data folder {dir}:\n\n{error}"),
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
            return fatal(&pick(
                format!("ratiod n'a pas pu démarrer :\n\n{error}"),
                format!("ratiod could not start:\n\n{error}"),
            ));
        }
        Err(_) => {
            return fatal(&pick(
                "ratiod n'a pas pu démarrer : le serveur s'est arrêté brutalement.",
                "ratiod could not start: the server stopped unexpectedly.",
            ));
        }
    };

    let window = match WindowBuilder::new()
        .with_title("ratiod")
        .with_inner_size(LogicalSize::new(1100.0, 760.0))
        .with_min_inner_size(LogicalSize::new(420.0, 500.0))
        .with_window_icon(WindowIcon::from_rgba(crate::icon::rgba(64), 64, 64).ok())
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(error) => {
            return fail(
                &runtime,
                &engine,
                &pick(
                    format!("Impossible de créer la fenêtre :\n\n{error}"),
                    format!("Cannot create the window:\n\n{error}"),
                ),
            );
        }
    };
    // WebView2 keeps its profile in the data folder, not next to the exe (which may be read-only).
    // On Windows the context only carries the data folder: it is reused by every (re)built webview.
    let mut web_context = WebContext::new(Some(settings.data_dir.join("webview2")));
    let mut generation = 0;
    let mut webview = match build_webview(&window, &mut web_context, &url, &proxy, generation) {
        Ok(webview) => Some(webview),
        Err(error) => {
            tracing::error!("cannot create the webview: {error}");
            return fail(
                &runtime,
                &engine,
                &pick(
                    format!(
                        "ratiod a besoin de Microsoft WebView2 pour afficher son interface, et il est introuvable ou \
                         inutilisable.\n\nInstalle-le depuis :\n{WEBVIEW2_DOWNLOAD}\n\n({error})"
                    ),
                    format!(
                        "ratiod needs Microsoft WebView2 to display its interface, and it is missing or unusable.\n\n\
                         Install it from:\n{WEBVIEW2_DOWNLOAD}\n\n({error})"
                    ),
                ),
            );
        }
    };

    let open_item = MenuItem::new(pick("Ouvrir ratiod", "Open ratiod"), true, None);
    let quit_item = MenuItem::new(pick("Quitter", "Quit"), true, None);
    let (open_id, quit_id) = (open_item.id().clone(), quit_item.id().clone());
    let menu = Menu::new();
    if let Err(error) = menu.append_items(&[&open_item, &quit_item]) {
        tracing::warn!("cannot build the tray menu: {error}");
    }
    let icon = match tray_icon::Icon::from_rgba(crate::icon::rgba(32), 32, 32) {
        Ok(icon) => icon,
        Err(error) => {
            return fail(
                &runtime,
                &engine,
                &pick(
                    format!("Icône de la zone de notification invalide :\n\n{error}"),
                    format!("Invalid notification area icon:\n\n{error}"),
                ),
            );
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
                &pick(
                    format!("Impossible de créer l'icône de la zone de notification :\n\n{error}"),
                    format!("Cannot create the notification area icon:\n\n{error}"),
                ),
            );
        }
    };

    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));
    let tray_proxy = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = tray_proxy.send_event(UserEvent::Tray(event));
    }));

    let mut tray = Some(tray);
    let mut quit_tx = Some(quit_tx);
    let mut recovery = RecoveryPolicy::new(MAX_RECOVERIES, RECOVERY_WINDOW);
    let mut rebuild = RebuildGate::default();
    // Kept alive until the rebuild, so that its `BrowserProcessExited` event can come.
    let mut old_environment: Option<ICoreWebView2Environment> = None;
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
            // Events of a webview already replaced are stale.
            Event::UserEvent(UserEvent::WebViewFailed { generation: failed, kind }) if failed == generation => {
                let Some(repair) = repair_for(kind) else {
                    tracing::warn!("a WebView2 process failed (kind {}), WebView2 restarts it itself", kind.0);
                    return;
                };
                let decision = recovery.on_failure(Instant::now());
                tracing::warn!(
                    "the display failed (WebView2 process failure kind {}), {} failure(s) in the last {} s",
                    kind.0,
                    recovery.failures(),
                    RECOVERY_WINDOW.as_secs()
                );
                if decision == Decision::GiveUp {
                    // Stops listening to this webview: no further recovery, no repeated dialog. The dead webview
                    // is closed so the window does not keep it (the dialog explains the empty window).
                    generation += 1;
                    if let Some(dead) = webview.take() {
                        close_webview(dead);
                    }
                    give_up(&url);
                    return;
                }
                match repair {
                    Repair::Reload => {
                        if let Some(webview) = &webview
                            && let Err(error) = webview.load_url(&url)
                        {
                            tracing::warn!("cannot reload the page: {error}");
                        }
                    }
                    Repair::Recreate => {
                        // A new webview built on the same data folder before the old browser process is fully gone
                        // attaches to it and dies with it (one crash would count twice): Microsoft's pattern is to
                        // close the old webview and rebuild once its environment reports the browser process exited.
                        generation += 1;
                        rebuild.arm(generation);
                        if let Some(dead) = webview.take() {
                            old_environment = await_browser_exit(&dead, &proxy, generation);
                            close_webview(dead);
                        }
                        let timeout_proxy = proxy.clone();
                        let id = generation;
                        std::thread::spawn(move || {
                            std::thread::sleep(BROWSER_EXIT_TIMEOUT);
                            let _ =
                                timeout_proxy.send_event(UserEvent::RebuildReady { generation: id, timed_out: true });
                        });
                    }
                }
            }
            Event::UserEvent(UserEvent::RebuildReady { generation: id, timed_out }) if rebuild.fire(id) => {
                if timed_out {
                    tracing::warn!("WebView2 did not report the end of its browser process, rebuilding anyway");
                }
                drop(old_environment.take());
                match build_webview(&window, &mut web_context, &url, &proxy, generation) {
                    Ok(rebuilt) => {
                        tracing::info!("the display has been recreated");
                        webview = Some(rebuilt);
                    }
                    Err(error) => {
                        tracing::error!("cannot recreate the webview: {error}");
                        give_up(&url);
                    }
                }
            }
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

/// Builds the webview in `window` and watches its WebView2 processes, retrying a few times: an overlay
/// injected into WebView2 can crash it while it starts.
fn build_webview(
    window: &Window,
    context: &mut WebContext,
    url: &str,
    proxy: &EventLoopProxy<UserEvent>,
    generation: u64,
) -> wry::Result<WebView> {
    let mut attempt = 1;
    loop {
        // No drag-and-drop handler on purpose: wry would replace WebView2's own handling, which is what
        // delivers HTML5 `dragenter`/`drop` events (with the files) to the page.
        match wry::WebViewBuilder::new_with_web_context(context).with_url(url).build(window) {
            Ok(webview) => {
                watch_failures(&webview, proxy.clone(), generation);
                return Ok(webview);
            }
            Err(error) if attempt < BUILD_ATTEMPTS => {
                tracing::warn!("cannot create the webview (attempt {attempt}/{BUILD_ATTEMPTS}): {error}");
                std::thread::sleep(BUILD_RETRY_PAUSE);
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Reports the failures of the webview's WebView2 processes to the event loop, which does the recovery.
fn watch_failures(webview: &WebView, proxy: EventLoopProxy<UserEvent>, generation: u64) {
    let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        // SAFETY: `kind` is a valid out pointer for the duration of the call.
        unsafe { args.ProcessFailedKind(&mut kind)? };
        let _ = proxy.send_event(UserEvent::WebViewFailed { generation, kind });
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: the handler is a live COM object and `token` a valid out pointer; the registration ends
    // with the webview (its controller is closed when it is dropped).
    if let Err(error) = unsafe { webview.webview().add_ProcessFailed(&handler, &mut token) } {
        tracing::warn!("cannot watch the WebView2 processes: {error}");
    }
}

/// Asks the dead webview's environment to report when its browser process is completely gone (it does
/// once the webview is closed), as the rebuild `generation`. Returns the environment, which must be
/// kept alive until then; `None` when the runtime is too old for this event (the rebuild timeout applies).
fn await_browser_exit(
    webview: &WebView,
    proxy: &EventLoopProxy<UserEvent>,
    generation: u64,
) -> Option<ICoreWebView2Environment> {
    let environment = webview.environment();
    let environment5 = match environment.cast::<ICoreWebView2Environment5>() {
        Ok(environment5) => environment5,
        Err(error) => {
            tracing::warn!("cannot wait for the end of the WebView2 browser process: {error}");
            return None;
        }
    };
    let proxy = proxy.clone();
    let handler = BrowserProcessExitedEventHandler::create(Box::new(move |_, _| {
        let _ = proxy.send_event(UserEvent::RebuildReady { generation, timed_out: false });
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: the handler is a live COM object and `token` a valid out pointer.
    if let Err(error) = unsafe { environment5.add_BrowserProcessExited(&handler, &mut token) } {
        tracing::warn!("cannot wait for the end of the WebView2 browser process: {error}");
        return None;
    }
    Some(environment)
}

/// Closes a dead webview. wry destroys its container window only for child webviews: it is destroyed
/// here, or every recreation would leave an empty one behind in the main window.
fn close_webview(webview: WebView) {
    let container = webview.hwnd().0 as HWND;
    drop(webview);
    // SAFETY: the container window was created on this thread by wry and nothing uses it any more.
    unsafe {
        DestroyWindow(container);
    }
}

/// The repair a WebView2 process failure needs, `None` for the processes WebView2 restarts itself
/// (GPU, utility, sandbox helper...).
fn repair_for(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> Option<Repair> {
    match kind {
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => Some(Repair::Recreate),
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
        | COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED
        | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => Some(Repair::Reload),
        _ => None,
    }
}

/// The display keeps crashing: stop recovering it and point the user to the browser (seeding goes on).
fn give_up(url: &str) {
    tracing::error!("the display crashed too many times, no more recovery (the interface stays reachable at {url})");
    warning_box(pick(
        format!(
            "L'affichage de ratiod a planté plusieurs fois de suite. C'est souvent causé par un logiciel d'overlay \
             (RivaTuner / MSI Afterburner, Discord…) qui s'injecte dans Microsoft WebView2.\n\nLe seed continue. Tu \
             peux ouvrir l'interface dans ton navigateur : {url}\n\nPour corriger : dans RivaTuner Statistics Server, \
             ajoute msedgewebview2.exe avec « Application detection level » sur « None », puis relance ratiod."
        ),
        format!(
            "ratiod's display crashed several times in a row. This is often caused by an overlay program (RivaTuner / \
             MSI Afterburner, Discord…) that injects itself into Microsoft WebView2.\n\nSeeding continues. You can \
             open the interface in your browser: {url}\n\nTo fix it: in RivaTuner Statistics Server, add \
             msedgewebview2.exe with “Application detection level” set to “None”, then restart ratiod."
        ),
    ));
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
    let port = settings.bind.port();
    fatal(&pick(
        format!(
            "Le port {port} est déjà utilisé par un autre programme.\n\nFerme ce programme, ou choisis une autre adresse \
             avec la variable d'environnement RATIOD_BIND."
        ),
        format!(
            "Port {port} is already in use by another program.\n\nClose that program, or pick another address \
             with the RATIOD_BIND environment variable."
        ),
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
