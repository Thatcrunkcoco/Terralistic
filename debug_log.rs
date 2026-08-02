use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

/// Whether the `--debug` flag was passed on the command line.
pub fn requested(args: &[String]) -> bool {
    args.iter().any(|a| a == "debug" || a == "--debug")
}

/// Returns the directory where debug logs are written (project-relative).
pub fn log_dir() -> PathBuf {
    PathBuf::from("logs")
}

/// Holds the non-blocking file writer guard so the log file stays open for the
/// lifetime of the process.
pub struct DebugLog {
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Initializes debug logging and crash capture when `--debug` was requested.
///
/// When disabled this does nothing and returns `None`, so normal builds have no
/// logging overhead and write nothing. When enabled it writes structured logs to
/// `logs/terralistic.log` and installs a panic hook that records backtraces.
pub fn init(args: &[String]) -> Option<DebugLog> {
    if !requested(args) {
        return None;
    }

    let dir = log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("Failed to create debug log directory at {}", dir.display());
        return None;
    }

    let file_appender = tracing_appender::rolling::never(&dir, "terralistic.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);

    // Keep our own code at TRACE but mute third-party crates (message_io, mio,
    // arboard, sdl2) that otherwise flood the log with their internals, so the
    // debug log stays focused on our sync/native code.
    let filter = EnvFilter::new("terralistic=trace,message_io=warn,mio=warn,arboard=warn,sdl2=warn,sdl2_sys=warn");

    tracing_subscriber::fmt()
        .with_writer(writer)
        // include file/line and context in the log lines for correlation
        .with_target(true)
        .with_env_filter(filter)
        .init();

    // Capture panics (and their backtraces) instead of losing them at exit.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = backtrace::Backtrace::new();
        tracing::error!("PANIC: {}\n{:?}", info, backtrace);
        previous(info);
    }));

    Some(DebugLog { _guard: guard })
}
