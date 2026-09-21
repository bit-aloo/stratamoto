use libafl::monitors::{Monitor, stats::ClientStatsManager};
use libafl_bolts::ClientId;

/// The causes findings are filed under, as the crash cause feedback names them.
pub const CAUSES: &[&str] = &[
    "setupconnection",
    "crash",
    "harnessintegrity",
    "infrastructure",
    "other",
    "timeout",
];

#[derive(Clone, Debug)]
pub struct GlobalMonitor<F>
where
    F: FnMut(&str),
{
    total_execs: u64,
    corpus_size: u64,
    log_fn: F,
}

impl<F> GlobalMonitor<F>
where
    F: FnMut(&str),
{
    pub fn new(log_fn: F) -> Self {
        Self {
            total_execs: 0,
            corpus_size: 0,
            log_fn,
        }
    }
}

impl<F> Monitor for GlobalMonitor<F>
where
    F: FnMut(&str),
{
    fn display(
        &mut self,
        client_stats_manager: &mut ClientStatsManager,
        event_msg: &str,
        _sender_id: ClientId,
    ) -> Result<(), libafl::Error> {
        let stat = |name: &str, default: &str| {
            client_stats_manager
                .aggregated()
                .get(name)
                .map_or(default.to_string(), std::string::ToString::to_string)
        };
        let trace = stat("trace", "0%");
        let stability = stat("stability", "100%");
        let causes: Vec<(&str, String)> = CAUSES
            .iter()
            .map(|cause| (*cause, stat(cause, "0")))
            .filter(|(_, count)| count != "0")
            .collect();

        let global_stats = client_stats_manager.global_stats();

        let event = match event_msg {
            "UserStats" => {
                let mut out = None;
                if global_stats.total_execs == 0
                    || global_stats.total_execs > self.total_execs
                    || global_stats.corpus_size > self.corpus_size
                {
                    self.total_execs = global_stats.total_execs.max(1);
                    self.corpus_size = global_stats.corpus_size;
                    out = Some("📊");
                }
                out
            }
            "Client Heartbeat" => Some("💗"),
            "Broker Heartbeat" => Some("💓"),
            "Objective" => {
                let bugs = ["🪲", "🐛", "🐞", "🪰", "🦗", "🦋"];
                Some(bugs[global_stats.run_time.subsec_nanos() as usize % bugs.len()])
            }
            "Testcase" => None,
            _ => Some(event_msg),
        };

        let bugs_str = if global_stats.objective_size > 0 && !causes.is_empty() {
            let details: Vec<String> = causes
                .iter()
                .map(|(cause, count)| format!("{count} {cause}"))
                .collect();
            format!(
                "bugs: {} ({})",
                global_stats.objective_size,
                details.join(", ")
            )
        } else {
            format!("bugs: {}", global_stats.objective_size)
        };

        if let Some(event) = event {
            let fmt = format!(
                "{} time: {} (x{}) execs: {} cov: {} corpus: {} exec/sec: {} stability: {} {}",
                event,
                global_stats.run_time_pretty,
                global_stats.client_stats_count,
                global_stats.total_execs,
                trace,
                global_stats.corpus_size,
                global_stats.execs_per_sec_pretty,
                stability,
                bugs_str
            );
            (self.log_fn)(&fmt);
        }

        Ok(())
    }
}
