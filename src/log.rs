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
                // lock_log_dirはpoisonを無視して回復するため、LOG_DIR保持中に
                // 一度panicが起きても、以後ずっとログが出なくなることはない。
                let d = $crate::lock_log_dir();
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
    };
}
