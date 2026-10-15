// Windows: no console window, ever. Logs go to `<data>\ratiod.log`, blocking errors to a message box.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

use anyhow::Context;
use ratiod::config::Settings;
use ratiod::server;

fn main() -> ExitCode {
    let headless = std::env::args().skip(1).any(|arg| arg == "--headless");
    #[cfg(windows)]
    if !headless {
        return ratiod::desktop::run();
    }
    // `--headless` only matters on Windows: other platforms always run as a server.
    let _ = headless;
    match run_server() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error:#}");
            #[cfg(windows)]
            ratiod::desktop::error_box(&ratiod::locale::pick(
                format!("ratiod n'a pas pu démarrer :\n\n{error:#}"),
                format!("ratiod could not start:\n\n{error:#}"),
            ));
            ExitCode::FAILURE
        }
    }
}

/// Server mode: no window, until Ctrl+C / SIGTERM.
fn run_server() -> anyhow::Result<()> {
    #[cfg(not(windows))]
    server::init_stderr_logging();
    let settings = Settings::from_env()?;
    #[cfg(windows)]
    {
        std::fs::create_dir_all(&settings.data_dir)
            .with_context(|| format!("cannot create {}", settings.data_dir.display()))?;
        server::init_file_logging(&settings.data_dir);
    }
    let runtime = tokio::runtime::Runtime::new().context("cannot start the async runtime")?;
    runtime.block_on(async {
        let mut running = server::start(&settings, None).await?;
        if settings.open_browser {
            open_browser(&running.url);
        }
        tokio::select! {
            result = &mut running.server => result.context("HTTP server crashed")?.context("HTTP server failed")?,
            () = shutdown_signal() => {}
        }
        running.engine.shutdown().await;
        tracing::info!("bye");
        Ok(())
    })
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutting down, sending stopped announces");
}

fn open_browser(url: &str) {
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Err(error) = command.spawn() {
        tracing::warn!("cannot open the browser ({error}); open {url} yourself");
    }
}
