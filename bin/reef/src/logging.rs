//! Logging: structured JSON to stderr by default with the keys
//! `time`, `level`, `msg` plus the event's fields; `text` for humans; an optional daily rolling
//! file (14 kept) in addition to stderr; the level from `[log].level` unless `RUST_LOG` is set.

use reef_api::config::LogConfig;
use std::fmt;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

/// Install the global subscriber. The returned guard flushes the log file on drop; keep it
/// alive for the life of the process.
pub fn init(log: &LogConfig) -> anyhow::Result<Option<WorkerGuard>> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{},tower_http=warn,sqlx=warn,russh=warn,hyper=warn,h2=warn,rustls=warn", log.level)));
    let json = log.format != "text";
    let (file_writer, guard) = match log.file.as_str() {
        "" => (None, None),
        path => {
            let p = std::path::Path::new(path);
            let dir = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .ok_or_else(|| anyhow::anyhow!("log.file must be an absolute path, got {path:?}"))?;
            let prefix = p.file_stem().and_then(|s| s.to_str()).unwrap_or("reef").to_string();
            let suffix = p.extension().and_then(|s| s.to_str()).unwrap_or("log").to_string();
            std::fs::create_dir_all(dir)?;
            let appender = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .max_log_files(14)
                .filename_prefix(prefix)
                .filename_suffix(suffix)
                .build(dir)?;
            let (nb, guard) = tracing_appender::non_blocking(appender);
            (Some(nb), Some(guard))
        }
    };
    let registry = tracing_subscriber::registry().with(filter);
    if json {
        let stderr = tracing_subscriber::fmt::layer().event_format(Json).with_writer(std::io::stderr);
        let file = file_writer.map(|w| tracing_subscriber::fmt::layer().event_format(Json).with_ansi(false).with_writer(w));
        registry.with(stderr).with(file).init();
    } else {
        let stderr = tracing_subscriber::fmt::layer().with_target(false).with_writer(std::io::stderr);
        let file = file_writer.map(|w| tracing_subscriber::fmt::layer().with_target(false).with_ansi(false).with_writer(w));
        registry.with(stderr).with(file).init();
    }
    Ok(guard)
}

/// Minimal stderr logging for the git hook entry point (no configuration is loaded there).
pub fn init_plain(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false).with_writer(std::io::stderr))
        .init();
}

/// `{"time":"...","level":"info","msg":"...",<fields>}` one per line (exactly these keys
/// first, then the event's fields). Debug and trace events also carry `target` (the module),
/// which only helps while debugging.
struct Json;

impl<S, N> FormatEvent<S, N> for Json
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, _ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        let mut map = serde_json::Map::new();
        let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default();
        map.insert("time".into(), serde_json::Value::String(now));
        map.insert("level".into(), serde_json::Value::String(event.metadata().level().as_str().to_ascii_lowercase()));
        let mut fields = Fields { map: serde_json::Map::new(), msg: None };
        event.record(&mut fields);
        map.insert("msg".into(), serde_json::Value::String(fields.msg.unwrap_or_default()));
        if *event.metadata().level() >= tracing::Level::DEBUG {
            map.insert("target".into(), serde_json::Value::String(event.metadata().target().to_string()));
        }
        for (k, v) in fields.map {
            map.insert(k, v);
        }
        writeln!(writer, "{}", serde_json::Value::Object(map))
    }
}

struct Fields {
    map: serde_json::Map<String, serde_json::Value>,
    msg: Option<String>,
}

impl Fields {
    fn put(&mut self, field: &Field, value: serde_json::Value) {
        if field.name() == "message" {
            self.msg = Some(match value {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            });
        } else {
            self.map.insert(field.name().to_string(), value);
        }
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, serde_json::Value::String(format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, serde_json::Value::String(value.to_string()));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, serde_json::Value::from(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, serde_json::Value::from(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, serde_json::Value::from(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, serde_json::Value::Bool(value));
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, serde_json::Value::String(value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// `time`, `level`, `msg`, then the fields; no `target` above debug.
    #[test]
    fn json_lines_have_exactly_these_keys() {
        let buf = Buf::default();
        let w = buf.clone();
        let sub = tracing_subscriber::registry()
            .with(EnvFilter::new("debug"))
            .with(tracing_subscriber::fmt::layer().event_format(Json).with_writer(move || w.clone()));
        tracing::subscriber::with_default(sub, || {
            tracing::info!(method = "GET", status = 200u64, "request");
            tracing::debug!(detail = "x", "probe");
        });
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let keys = |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&lines[0]), ["time", "level", "msg", "method", "status"], "{text}");
        assert_eq!(lines[0]["level"], "info");
        assert_eq!(lines[0]["msg"], "request");
        assert!(lines[0].get("target").is_none());
        assert_eq!(lines[1]["level"], "debug");
        assert!(lines[1].get("target").is_some(), "kept at debug");
        assert!(text.lines().next().unwrap().starts_with(r#"{"time":"#), "time, level, msg lead: {text}");
    }
}
