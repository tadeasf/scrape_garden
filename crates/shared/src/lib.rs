//! Shared types used across all scrape-garden crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

// ─── Error Types ───────────────────────────────────────────────────────────

/// Errors originating from shared-type validation or serialization.
#[derive(Debug, Error)]
pub enum SharedError {
    #[error("invalid URL \"{0}\": {1}")]
    InvalidUrl(String, String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_yaml::Error),

    #[error("deserialization error: {0}")]
    Deserialization(#[from] serde_json::Error),

    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ─── Fetcher Type ──────────────────────────────────────────────────────────

/// Which fetcher implementation to use for a job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FetcherType {
    Http,
    Browser,
}

impl Default for FetcherType {
    fn default() -> Self {
        Self::Http
    }
}

// ─── Extraction Rule ───────────────────────────────────────────────────────

/// A single CSS-selector-based extraction rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionRule {
    /// CSS selector to match elements.
    pub selector: String,
    /// Field name in the output JSON.
    pub name: String,
    /// Optional: which attribute to extract (e.g. "href", "src").
    /// If None, the element's text content is used.
    #[serde(default)]
    pub attribute: Option<String>,
}

// ─── Scrape Job ────────────────────────────────────────────────────────────

/// A scraping job defined in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeJob {
    /// Human-readable name for this job.
    pub job_name: String,
    /// URL to scrape.
    pub url: String,
    /// Cron schedule expression (e.g. `"0 */6 * * *"`).
    #[serde(default)]
    pub schedule: Option<String>,
    /// Which fetcher to use.
    #[serde(default)]
    pub fetcher: FetcherType,
    /// Optional HTTP headers.
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    /// Extraction rules.
    #[serde(default)]
    pub extraction: Vec<ExtractionRule>,
    /// Output file path for results (JSONL).
    #[serde(default)]
    pub output: Option<String>,
    /// Optional: number of retries on failure.
    #[serde(default = "default_retries")]
    pub retries: u32,
}

fn default_retries() -> u32 {
    3
}

impl ScrapeJob {
    /// Validate the job's URL.
    pub fn validate(&self) -> Result<(), SharedError> {
        let parsed: Url = self
            .url
            .parse()
            .map_err(|e: url::ParseError| SharedError::InvalidUrl(self.url.clone(), e.to_string()))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(SharedError::InvalidUrl(
                self.url.clone(),
                "only http and https schemes are supported".to_string(),
            ));
        }
        Ok(())
    }
}

// ─── Scrape Result ─────────────────────────────────────────────────────────

/// Status of a single scrape run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Error,
}

/// Result of a single scrape run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeResult {
    /// Name of the job that produced this result.
    pub job_name: String,
    /// URL that was scraped.
    pub url: String,
    /// Timestamp of the scrape.
    pub timestamp: DateTime<Utc>,
    /// Status of this run.
    pub status: JobStatus,
    /// Extracted key-value data (empty on error).
    #[serde(default)]
    pub data: serde_json::Value,
    /// Error message if the run failed.
    #[serde(default)]
    pub error: Option<String>,
}

impl ScrapeResult {
    pub fn success(job_name: impl Into<String>, url: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            job_name: job_name.into(),
            url: url.into(),
            timestamp: Utc::now(),
            status: JobStatus::Done,
            data,
            error: None,
        }
    }

    pub fn failure(job_name: impl Into<String>, url: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            job_name: job_name.into(),
            url: url.into(),
            timestamp: Utc::now(),
            status: JobStatus::Error,
            data: serde_json::Value::Null,
            error: Some(error.into()),
        }
    }
}

// ─── Global Config ─────────────────────────────────────────────────────────

/// Top-level configuration file schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Optional: global output directory (each job can override).
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Optional: global user-agent for HTTP requests.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// List of scrape jobs.
    pub jobs: Vec<ScrapeJob>,
}

impl Config {
    /// Load config from a YAML file.
    pub fn from_file(path: &str) -> Result<Self, SharedError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Save config to a YAML file.
    pub fn to_file(&self, path: &str) -> Result<(), SharedError> {
        let content = serde_yaml::to_string(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

// ─── Dashboard State (shared between daemon and TUI) ───────────────────────

/// Shared state for the TUI dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardState {
    /// Latest result for each job.
    pub job_statuses: Vec<JobStatusEntry>,
    /// Recent log lines.
    pub logs: Vec<LogEntry>,
    /// Aggregate statistics.
    pub stats: Stats,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            job_statuses: Vec::new(),
            logs: Vec::new(),
            stats: Stats::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatusEntry {
    pub job_name: String,
    pub status: JobStatus,
    pub last_run: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stats {
    pub total_runs: u64,
    pub successful_runs: u64,
    pub failed_runs: u64,
}

// ─── Dashboard Commands (TUI → Daemon) ─────────────────────────────────────

/// Commands the TUI can send to the daemon.
#[derive(Debug, Clone)]
pub enum DashboardCommand {
    /// Refresh the current state.
    Refresh,
    /// Clear all logs.
    ClearLogs,
    /// Request a specific job to run now.
    RunNow(String),
    /// Quit the TUI.
    Quit,
}

/// Messages the daemon sends to the TUI.
#[derive(Debug, Clone)]
pub enum DashboardMessage {
    /// Updated dashboard state.
    State(DashboardState),
    /// A new log entry.
    Log(LogEntry),
    /// A new scrape result.
    Result(ScrapeResult),
}
