//! Process-wide counters exposed at `/metrics` in Prometheus text format.
//!
//! Atomics only — the handler path must never block on a lock.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    commands_total: AtomicU64,
    messages_total: AtomicU64,
    automod_actions: AtomicU64,
    links_threaded: AtomicU64,
    errors: AtomicU64,
    guilds: AtomicU64,
}

impl Metrics {
    pub fn inc_commands(&self) {
        self.commands_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_messages(&self) {
        self.messages_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_automod(&self) {
        self.automod_actions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_links_threaded(&self) {
        self.links_threaded.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_errors(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_guilds(&self, guilds: u64) {
        self.guilds.store(guilds, Ordering::Relaxed);
    }

    pub fn guilds(&self) -> u64 {
        self.guilds.load(Ordering::Relaxed)
    }

    pub fn commands_total(&self) -> u64 {
        self.commands_total.load(Ordering::Relaxed)
    }

    pub fn messages_total(&self) -> u64 {
        self.messages_total.load(Ordering::Relaxed)
    }

    pub fn automod_actions(&self) -> u64 {
        self.automod_actions.load(Ordering::Relaxed)
    }

    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Prometheus exposition text.
    pub fn render_prometheus(&self, uptime_secs: u64, db_ok: bool) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("# HELP cipherbot_commands_total Slash commands dispatched\n");
        out.push_str("# TYPE cipherbot_commands_total counter\n");
        out.push_str(&format!(
            "cipherbot_commands_total {}\n",
            self.commands_total.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP cipherbot_messages_total Guild messages processed\n");
        out.push_str("# TYPE cipherbot_messages_total counter\n");
        out.push_str(&format!(
            "cipherbot_messages_total {}\n",
            self.messages_total.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP cipherbot_automod_actions_total Automod actions taken\n");
        out.push_str("# TYPE cipherbot_automod_actions_total counter\n");
        out.push_str(&format!(
            "cipherbot_automod_actions_total {}\n",
            self.automod_actions.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP cipherbot_links_threaded_total Link threads created\n");
        out.push_str("# TYPE cipherbot_links_threaded_total counter\n");
        out.push_str(&format!(
            "cipherbot_links_threaded_total {}\n",
            self.links_threaded.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP cipherbot_errors_total Handler errors\n");
        out.push_str("# TYPE cipherbot_errors_total counter\n");
        out.push_str(&format!(
            "cipherbot_errors_total {}\n",
            self.errors.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP cipherbot_guilds Guilds the bot is connected to\n");
        out.push_str("# TYPE cipherbot_guilds gauge\n");
        out.push_str(&format!(
            "cipherbot_guilds {}\n",
            self.guilds.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP cipherbot_uptime_seconds Process uptime\n");
        out.push_str("# TYPE cipherbot_uptime_seconds gauge\n");
        out.push_str(&format!("cipherbot_uptime_seconds {uptime_secs}\n"));

        out.push_str("# HELP cipherbot_db_ok Database health probe (1 = ok)\n");
        out.push_str("# TYPE cipherbot_db_ok gauge\n");
        out.push_str(&format!("cipherbot_db_ok {}\n", if db_ok { 1 } else { 0 }));

        out.push_str("# HELP cipherbot_build_info Build version\n");
        out.push_str("# TYPE cipherbot_build_info gauge\n");
        out.push_str(&format!(
            "cipherbot_build_info{{version=\"{}\"}} 1\n",
            env!("CARGO_PKG_VERSION")
        ));

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_expected_metrics() {
        let metrics = Metrics::default();
        metrics.inc_commands();
        metrics.inc_commands();
        metrics.inc_messages();
        metrics.inc_automod();
        metrics.inc_links_threaded();
        metrics.inc_errors();
        metrics.set_guilds(3);

        let rendered = metrics.render_prometheus(42, true);
        assert!(rendered.contains("cipherbot_commands_total 2"));
        assert!(rendered.contains("cipherbot_messages_total 1"));
        assert!(rendered.contains("cipherbot_automod_actions_total 1"));
        assert!(rendered.contains("cipherbot_links_threaded_total 1"));
        assert!(rendered.contains("cipherbot_errors_total 1"));
        assert!(rendered.contains("cipherbot_guilds 3"));
        assert!(rendered.contains("cipherbot_uptime_seconds 42"));
        assert!(rendered.contains("cipherbot_db_ok 1"));
        assert!(rendered.contains("cipherbot_build_info{version="));
        assert!(rendered.contains("# TYPE cipherbot_commands_total counter"));
    }

    #[test]
    fn db_failure_is_reported() {
        let rendered = Metrics::default().render_prometheus(0, false);
        assert!(rendered.contains("cipherbot_db_ok 0"));
    }
}
