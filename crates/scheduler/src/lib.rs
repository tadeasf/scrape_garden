//! scheduler: Cron-based job scheduling, config loading, and result persistence.

use chrono::{Datelike, Timelike, Utc};
use scraper_core::{ExtractionEngine, HttpClient};
use shared::{
    Config, DashboardMessage, JobStatus, LogEntry, ScrapeJob, ScrapeResult, SharedError,
};
use std::io::Write;
use thiserror::Error;

// ─── Error Types ───────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("config error: {0}")]
    Config(#[from] SharedError),

    #[error("job error for \"{0}\": {1}")]
    Job(String, String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("channel closed")]
    ChannelClosed,
}

// ─── Config Loader ─────────────────────────────────────────────────────────

/// Loads scraping configuration from a YAML file.
pub struct ConfigLoader;

impl ConfigLoader {
    /// Load config from file path.
    pub fn load(path: &str) -> Result<Config, SharedError> {
        Config::from_file(path)
    }

    /// Validate all jobs in the config.
    pub fn validate(config: &Config) -> Result<(), SchedulerError> {
        for job in &config.jobs {
            job.validate().map_err(|e| SchedulerError::Config(e))?;
        }
        Ok(())
    }
}

// ─── Result Writer ─────────────────────────────────────────────────────────

/// Writes scrape results to JSONL files.
pub struct ResultWriter;

impl ResultWriter {
    /// Append a single result to a JSONL file. Creates the file if it doesn't exist.
    pub fn write(result: &ScrapeResult, output_path: &str) -> Result<(), SchedulerError> {
        let line = serde_json::to_string(result)?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)?
            .write_all(format!("{line}\n").as_bytes())?;
        Ok(())
    }

    /// Write result using the job's configured output path, or the default.
    pub fn write_with_config(
        result: &ScrapeResult,
        job: &ScrapeJob,
        global_output_dir: Option<&str>,
    ) -> Result<(), SchedulerError> {
        let output_path = job
            .output
            .as_deref()
            .or(global_output_dir)
            .unwrap_or("results.jsonl");
        Self::write(result, output_path)
    }
}

// ─── Job Runner ────────────────────────────────────────────────────────────

/// Runs individual scrape jobs and reports results.
#[derive(Clone)]
pub struct JobRunner {
    log_tx: tokio::sync::mpsc::UnboundedSender<DashboardMessage>,
}

impl JobRunner {
    pub fn new(log_tx: tokio::sync::mpsc::UnboundedSender<DashboardMessage>) -> Self {
        Self { log_tx }
    }

    /// Run a single job with retries and write the result.
    pub async fn run_job(
        &self,
        job: &ScrapeJob,
        global_output_dir: Option<&str>,
    ) -> ScrapeResult {
        let log_tx = self.log_tx.clone();

        // Notify that the job is starting.
        let _ = log_tx.send(DashboardMessage::Log(LogEntry {
            timestamp: Utc::now(),
            level: "INFO".into(),
            message: format!("Starting job: {}", job.job_name),
        }));

        // Build the extraction engine with an HTTP client.
        let fetcher = HttpClient::new();
        let engine = ExtractionEngine::new(fetcher);

        // Run with retries.
        let result = engine.run_with_retries(job).await;

        // Write result to file.
        if let Err(e) = ResultWriter::write_with_config(&result, job, global_output_dir) {
            let _ = log_tx.send(DashboardMessage::Log(LogEntry {
                timestamp: Utc::now(),
                level: "ERROR".into(),
                message: format!("Failed to write results for {}: {}", job.job_name, e),
            }));
        }

        // Send result to dashboard.
        let _ = log_tx.send(DashboardMessage::Result(result.clone()));

        let status_str = match result.status {
            JobStatus::Done => "SUCCESS",
            JobStatus::Error => "FAILED",
            _ => "UNKNOWN",
        };

        let _ = log_tx.send(DashboardMessage::Log(LogEntry {
            timestamp: Utc::now(),
            level: if result.status == JobStatus::Done {
                "INFO"
            } else {
                "ERROR"
            }
            .into(),
            message: format!(
                "Job {} completed: {} (status: {})",
                job.job_name,
                job.url,
                status_str
            ),
        }));

        result
    }
}

// ─── Scheduler ─────────────────────────────────────────────────────────────

/// Manages scheduled job execution using tokio-cron-scheduler.
pub struct Scheduler {
    config: Config,
    log_tx: tokio::sync::mpsc::UnboundedSender<DashboardMessage>,
}

impl Scheduler {
    pub fn new(
        config: Config,
        log_tx: tokio::sync::mpsc::UnboundedSender<DashboardMessage>,
    ) -> Self {
        Self { config, log_tx }
    }

    /// Start the scheduler — runs all jobs according to their cron schedules.
    /// If a job has no schedule, it runs once immediately.
    pub async fn run(&self) -> Result<(), SchedulerError> {
        let job_runner = JobRunner::new(self.log_tx.clone());
        let global_output_dir = self.config.output_dir.as_deref();

        let mut handles = Vec::new();

        for job in &self.config.jobs {
            let job_runner = job_runner.clone();
            let global_output_dir = global_output_dir.map(|s| s.to_string());
            let job = job.clone();

            let handle = if let Some(ref schedule) = job.schedule {
                // Schedule with cron expression.
                let schedule = schedule.clone();
                self.schedule_cron_job(schedule, job, job_runner, global_output_dir)
                    .await?
            } else {
                // Run once immediately.
                tokio::spawn(async move {
                    let _ = job_runner.run_job(&job, global_output_dir.as_deref()).await;
                })
            };
            handles.push(handle);
        }

        // Wait for all jobs (this will block indefinitely for cron jobs,
        // which is the desired behavior for a daemon).
        // In practice, the daemon runs forever.
        if let Some(handle) = handles.into_iter().next() {
            let _ = handle.await;
        }

        Ok(())
    }

    async fn schedule_cron_job(
        &self,
        schedule: String,
        job: ScrapeJob,
        job_runner: JobRunner,
        global_output_dir: Option<String>,
    ) -> Result<tokio::task::JoinHandle<()>, SchedulerError> {
        let log_tx = self.log_tx.clone();

        // Log the schedule.
        let _ = log_tx.send(DashboardMessage::Log(LogEntry {
            timestamp: Utc::now(),
            level: "INFO".into(),
            message: format!("Scheduled job '{}': cron '{}'", job.job_name, schedule),
        }));

        // For now, run the job once immediately then sleep until the next scheduled time.
        // A full cron-scheduler integration would use the tokio-cron-scheduler crate,
        // but for simplicity we implement a basic scheduler here.
        let job_runner2 = job_runner.clone();
        let job2 = job.clone();
        let output_dir2 = global_output_dir.clone();
        let schedule2 = schedule.clone();

        Ok(tokio::spawn(async move {
            // Run once immediately.
            let _ = job_runner2.run_job(&job2, output_dir2.as_deref()).await;

            // Parse the cron expression and sleep until next run.
            // Simple cron parser for common patterns.
            loop {
                let next_run = match parse_next_cron_run(&schedule2) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = log_tx.send(DashboardMessage::Log(LogEntry {
                            timestamp: Utc::now(),
                            level: "ERROR".into(),
                            message: format!("Failed to parse cron '{}': {}", schedule2, e),
                        }));
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        continue;
                    }
                };

                let delay = next_run.signed_duration_since(Utc::now()).to_std();
                match delay {
                    Ok(d) if d.as_secs() > 0 => {
                        tokio::time::sleep(d).await;
                    }
                    _ => {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }

                let _ = job_runner.run_job(&job, global_output_dir.as_deref()).await;
            }
        }))
    }
}

// ─── Simple Cron Parser ────────────────────────────────────────────────────

/// Parse a basic cron expression (minute hour day-of-month month day-of-week)
/// and return the next run time.
/// Supports: `*`, numeric values, `*/N` step patterns.
fn parse_next_cron_run(expression: &str) -> Result<chrono::DateTime<Utc>, String> {
    let parts: Vec<&str> = expression.trim().split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!(
            "expected 5 fields in cron expression, got {}",
            parts.len()
        ));
    }

    let now = Utc::now();
    let mut next = now + chrono::Duration::seconds(1);

    // Normalize to start of next minute.
    next = next.with_second(0).unwrap().with_nanosecond(0).unwrap();

    let minute_expr = parse_cron_field(&parts[0], 0, 59)?;
    let hour_expr = parse_cron_field(&parts[1], 0, 23)?;
    let dom_expr = parse_cron_field(&parts[2], 1, 31)?;
    let month_expr = parse_cron_field(&parts[3], 1, 12)?;
    let dow_expr = parse_cron_field(&parts[4], 0, 6)?;

    // Find next matching time (simple forward search, up to 366 days).
    for _ in 0..525960 {
        // Max ~1 year of minutes
        if month_expr.contains(next.month() as u32)
            && dow_expr.contains(next.weekday().num_days_from_sunday())
            && dom_expr.contains(next.day() as u32)
            && hour_expr.contains(next.hour() as u32)
            && minute_expr.contains(next.minute() as u32)
        {
            return Ok(next);
        }
        next += chrono::Duration::minutes(1);
    }

    Err("could not find next run time within 1 year".into())
}

fn parse_cron_field(field: &str, min: u32, max: u32) -> Result<CronField, String> {
    if field == "*" {
        return Ok(CronField::All);
    }

    if let Some(n) = field.strip_prefix("*/") {
        let step = n.parse::<u32>().map_err(|_| format!("invalid step in '{field}'"))?;
        if step == 0 {
            return Err(format!("step cannot be 0 in '{field}'"));
        }
        return Ok(CronField::Step(min, max, step));
    }

    // Handle comma-separated values or ranges.
    let mut values = std::collections::HashSet::new();
    for part in field.split(',') {
        if let Some(range) = part.strip_prefix("RANGE:") {
            // Not supporting complex ranges for now.
        } else if let Some((a, b)) = part.split_once('-') {
            let start = a.parse::<u32>().map_err(|_| format!("invalid range start in '{part}'"))?;
            let end = b.parse::<u32>().map_err(|_| format!("invalid range end in '{part}'"))?;
            if start < min || end > max {
                return Err(format!("range {start}-{end} out of bounds [{min}-{max}]"));
            }
            for v in start..=end {
                values.insert(v);
            }
        } else {
            let v = part.parse::<u32>().map_err(|_| format!("invalid value '{part}'"))?;
            if v < min || v > max {
                return Err(format!("value {v} out of bounds [{min}-{max}]"));
            }
            values.insert(v);
        }
    }

    if values.is_empty() {
        Err(format!("empty values in field '{field}'"))
    } else {
        Ok(CronField::Values(values))
    }
}

enum CronField {
    All,
    Step(u32, u32, u32), // min, max, step
    Values(std::collections::HashSet<u32>),
}

impl CronField {
    fn contains(&self, value: u32) -> bool {
        match self {
            CronField::All => true,
            CronField::Step(min, max, step) => {
                (value >= *min && value <= *max) && ((value - min) % step == 0)
            }
            CronField::Values(vals) => vals.contains(&value),
        }
    }
}

// ─── Daemon Entry Point ────────────────────────────────────────────────────

/// Run the full scrape-garden daemon.
pub async fn run_daemon(config_path: &str) -> Result<(), SchedulerError> {
    let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<DashboardMessage>();

    // Load and validate config.
    let config = ConfigLoader::load(config_path)?;
    ConfigLoader::validate(&config)?;

    println!("🌿 scrape-garden daemon starting...");
    println!("   Config: {}", config_path);
    println!("   Jobs: {}", config.jobs.len());
    println!();

    // Start the scheduler.
    let scheduler = Scheduler::new(config, log_tx);

    // Spawn a task to forward log messages to stdout.
    tokio::spawn(async move {
        while let Some(msg) = log_rx.recv().await {
            match msg {
                DashboardMessage::Log(entry) => {
                    println!("[{}] [{}] {}", entry.timestamp, entry.level, entry.message);
                }
                DashboardMessage::Result(result) => {
                    println!(
                        "  📊 Result for '{}': status={:?}, data_keys={}",
                        result.job_name,
                        result.status,
                        if result.data.is_object() {
                            result.data.as_object().unwrap().len()
                        } else {
                            0
                        }
                    );
                }
                DashboardMessage::State(_) => {}
            }
        }
    });

    // Run the scheduler (blocks until interrupted).
    scheduler.run().await
}

// ─── Manual Run (for testing / CLI one-shot) ───────────────────────────────

/// Run all jobs in the config immediately (no scheduling).
pub async fn run_all(config_path: &str) -> Result<(), SchedulerError> {
    let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<DashboardMessage>();

    let config = ConfigLoader::load(config_path)?;
    ConfigLoader::validate(&config)?;

    let job_runner = JobRunner::new(log_tx);
    let global_output_dir = config.output_dir.as_deref();

    println!("🌿 scrape-garden — running all jobs...");

    let mut handles = Vec::new();
    for job in config.jobs {
        let job_runner = job_runner.clone();
        let global_output_dir = global_output_dir.map(|s| s.to_string());
        handles.push(tokio::spawn(async move {
            let _ = job_runner.run_job(&job, global_output_dir.as_deref()).await;
        }));
    }

    // Spawn log forwarder.
    tokio::spawn(async move {
        while let Some(msg) = log_rx.recv().await {
            match msg {
                DashboardMessage::Log(entry) => {
                    println!("[{}] [{}] {}", entry.timestamp, entry.level, entry.message);
                }
                DashboardMessage::Result(result) => {
                    println!(
                        "  📊 Result for '{}': status={:?}",
                        result.job_name, result.status
                    );
                }
                DashboardMessage::State(_) => {}
            }
        }
    });

    for handle in handles {
        let _ = handle.await;
    }

    println!("✅ All jobs completed.");
    Ok(())
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_parse_every_minute() {
        let next = parse_next_cron_run("* * * * *");
        assert!(next.is_ok());
        let next = next.unwrap();
        // Next run should be within 2 minutes from now.
        let diff = (next - Utc::now()).num_seconds().unsigned_abs();
        assert!(diff < 120, "next run {} should be within 2 min of now {}", next, Utc::now());
    }

    #[test]
    fn test_cron_parse_every_6_hours() {
        let next = parse_next_cron_run("0 */6 * * *");
        assert!(next.is_ok());
        let next = next.unwrap();
        // Next run should be within 7 hours from now.
        let diff = (next - Utc::now()).num_hours().unsigned_abs();
        assert!(diff < 7, "next run {} should be within 7 hours of now {}", next, Utc::now());
    }

    #[test]
    fn test_cron_parse_invalid() {
        let result = parse_next_cron_run("bad cron");
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_parse_step() {
        let next = parse_next_cron_run("*/15 * * * *");
        assert!(next.is_ok());
    }
}
