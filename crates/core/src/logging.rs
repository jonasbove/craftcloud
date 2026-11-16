//! The filter every CraftCloud logger goes through: the HTTP library's own lines stop at debug.
//!
//! At trace level `ureq-proto` logs the raw bytes of each request and answer, which hold the
//! `Authorization` header, cookies and sign-in bodies (refresh tokens). `RUST_LOG=trace` (or
//! `RUST_LOG=ureq_proto=trace`) must not put those in a log, so [`WireCapped`] drops anything
//! below debug from these targets whatever the inner logger lets through. Their debug lines
//! (method, address, status) stay.

use log::{Level, Log, Metadata, Record};

/// Log targets (and their submodules) that write request and answer bytes at trace level.
pub const WIRE_TARGETS: [&str; 2] = ["ureq", "ureq_proto"];

/// The most these targets may log, even when the inner logger asks for more.
pub const WIRE_MAX: Level = Level::Debug;

/// Whether a line at `level` from `target` may be logged at all.
pub fn allowed(target: &str, level: Level) -> bool {
    level <= WIRE_MAX || !WIRE_TARGETS.iter().any(|t| target.strip_prefix(t).is_some_and(|rest| rest.is_empty() || rest.starts_with("::")))
}

/// A logger that passes every line to `L` except the trace lines of [`WIRE_TARGETS`].
pub struct WireCapped<L>(pub L);

impl<L: Log> Log for WireCapped<L> {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        allowed(metadata.target(), metadata.level()) && self.0.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if allowed(record.target(), record.level()) {
            self.0.log(record);
        }
    }

    fn flush(&self) {
        self.0.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use log::{Level, Log, Metadata, Record};

    use super::{WireCapped, allowed};

    #[test]
    fn the_http_library_stops_at_debug() {
        for target in ["ureq", "ureq::run", "ureq_proto", "ureq_proto::util"] {
            assert!(!allowed(target, Level::Trace), "{target}");
            assert!(allowed(target, Level::Debug) && allowed(target, Level::Error), "{target}");
        }
    }

    #[test]
    fn other_targets_keep_their_trace_lines() {
        for target in ["craftcloud_native::sync", "ureqx", "ureq_protocol", "my::ureq", "rustls::client"] {
            assert!(allowed(target, Level::Trace), "{target}");
        }
    }

    struct Seen(Mutex<Vec<String>>);
    impl Log for Seen {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &Record<'_>) {
            self.0.lock().unwrap().push(format!("{} {}", record.target(), record.args()));
        }
        fn flush(&self) {}
    }

    #[test]
    fn the_wrapper_drops_only_the_wire_bytes() {
        let logger = WireCapped(Seen(Mutex::new(Vec::new())));
        let line = |target: &'static str, level: Level| {
            let meta = Metadata::builder().target(target).level(level).build();
            let enabled = logger.enabled(&meta);
            logger.log(&Record::builder().metadata(meta).args(format_args!("x")).build());
            enabled
        };
        assert!(!line("ureq_proto::util", Level::Trace));
        assert!(line("ureq_proto::util", Level::Debug));
        assert!(line("craftcloud", Level::Trace));
        assert_eq!(*logger.0.0.lock().unwrap(), ["ureq_proto::util x", "craftcloud x"]);
    }
}
