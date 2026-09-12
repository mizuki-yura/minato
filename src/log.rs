// ── ログユーティリティ ────────────────────────────────────

macro_rules! append_log {
    ($msg:expr) => {
        {
            #[cfg(debug_assertions)]
            let should_log = true;
            #[cfg(not(debug_assertions))]
            let should_log = $crate::DEBUG_LOG.load(std::sync::atomic::Ordering::Relaxed);

            if should_log {
                use std::io::Write;
                if let Ok(d) = $crate::LOG_DIR.lock() {
                    if !d.as_os_str().is_empty() {
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(d.join("minato_load.log"))
                        {
                            let _ = writeln!(f, "{}", $msg);
                        }
                    }
                }
            }
        }
    };
}
