//! scraper-core: The scraping engine — HTTP fetching, HTML parsing, and structured extraction.

use scraper::{Html, Selector};
use shared::{ExtractionRule, ScrapeJob, ScrapeResult};
use thiserror::Error;

// ─── Error Types ───────────────────────────────────────────────────────────

/// Errors from fetching / HTTP operations.
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("request timeout after {0}ms")]
    Timeout(u64),

    #[error("invalid response status {0}: {1}")]
    Status(u16, String),

    #[error("network error: {0}")]
    Network(String),
}

/// Errors from parsing / extraction operations.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid CSS selector \"{0}\": {1}")]
    InvalidSelector(String, String),

    #[error("extraction failed for rule \"{0}\": {1}")]
    Extraction(String, String),

    #[error("no elements matched selector \"{0}\"")]
    NoMatch(String),
}

/// Unified scrape error.
#[derive(Debug, Error)]
pub enum ScrapeError {
    #[error("fetch error: {0}")]
    Fetch(#[from] FetchError),

    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
}

// ─── Fetcher Trait ─────────────────────────────────────────────────────────

/// Abstraction over different fetcher implementations (HTTP, Browser, etc.).
pub trait Fetcher: Send + Sync + 'static {
    /// Fetch the raw HTML from the job's URL.
    async fn fetch_html(&self, job: &ScrapeJob) -> Result<String, FetchError>;

    /// Full pipeline: fetch + parse + extract structured data.
    async fn scrape(&self, job: &ScrapeJob) -> ScrapeResult {
        let html = match self.fetch_html(job).await {
            Ok(html) => html,
            Err(e) => {
                return ScrapeResult::failure(&job.job_name, &job.url, e.to_string());
            }
        };

        match self.extract(&job.job_name, &job.url, &html, &job.extraction) {
            Ok(data) => ScrapeResult::success(&job.job_name, &job.url, data),
            Err(e) => ScrapeResult::failure(&job.job_name, &job.url, e.to_string()),
        }
    }

    /// Extract structured data using the given extraction rules.
    fn extract(
        &self,
        _job_name: &str,
        _url: &str,
        html: &str,
        rules: &[ExtractionRule],
    ) -> Result<serde_json::Value, ParseError> {
        let parsed = Html::parse_document(html);
        let mut records: Vec<serde_json::Value> = Vec::new();

        for rule in rules {
            let selector = Selector::parse(&rule.selector).map_err(|e| {
                ParseError::InvalidSelector(rule.selector.clone(), e.to_string())
            })?;

            let elements: Vec<_> = parsed.select(&selector).collect();

            if elements.is_empty() {
                // Skip rules with no matches rather than failing.
                continue;
            }

            let items: Vec<serde_json::Value> = elements
                .iter()
                .map(|el| {
                    if let Some(attr) = &rule.attribute {
                        el.value()
                            .attr(attr)
                            .map(|v| serde_json::Value::String(v.to_string()))
                            .unwrap_or_default()
                    } else {
                        serde_json::Value::String(el.text().collect::<String>())
                    }
                })
                .collect();

            records.push(serde_json::json!({
                rule.name.clone(): if items.len() == 1 { items.into_iter().next().unwrap() } else { serde_json::Value::Array(items) }
            }));
        }

        // Merge all records into a single object.
        let mut merged: serde_json::Map<String, serde_json::Value> =
            serde_json::Map::new();
        for record in records {
            if let serde_json::Value::Object(map) = record {
                for (k, v) in map {
                    merged.insert(k, v);
                }
            }
        }

        Ok(serde_json::Value::Object(merged))
    }
}

// ─── HTTP Client ───────────────────────────────────────────────────────────

/// Default HTTP fetcher using `reqwest`.
pub struct HttpClient {
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("scrape-garden/0.1.0")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    /// Create with a custom user-agent.
    pub fn with_user_agent(user_agent: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(user_agent)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build reqwest client"),
        }
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher for HttpClient {
    async fn fetch_html(&self, job: &ScrapeJob) -> Result<String, FetchError> {
        let mut builder = self.client.get(&job.url);

        // Apply custom headers.
        for (key, value) in &job.headers {
            builder = builder.header(key, value);
        }

        let response = builder.send().await?;
        let status = response.status();

        if !status.is_success() {
            return Err(FetchError::Status(
                status.as_u16(),
                format!("Error fetching {}: {}", job.url, status),
            ));
        }

        let body = response.text().await?;
        Ok(body)
    }
}

// ─── HTML Parser ───────────────────────────────────────────────────────────

/// Utility functions for HTML parsing.
pub struct HtmlParser;

impl HtmlParser {
    /// Parse raw HTML and return a `Html` document.
    pub fn parse(html: &str) -> Html {
        Html::parse_document(html)
    }

    /// Extract text content matching a CSS selector.
    pub fn extract_text(html: &Html, selector: &str) -> Result<Vec<String>, ParseError> {
        let sel = Selector::parse(selector).map_err(|e| {
            ParseError::InvalidSelector(selector.to_string(), e.to_string())
        })?;
        Ok(html.select(&sel).map(|el| el.text().collect()).collect())
    }

    /// Extract attribute values matching a CSS selector.
    pub fn extract_attr(
        html: &Html,
        selector: &str,
        attr: &str,
    ) -> Result<Vec<String>, ParseError> {
        let sel = Selector::parse(selector).map_err(|e| {
            ParseError::InvalidSelector(selector.to_string(), e.to_string())
        })?;
        Ok(html
            .select(&sel)
            .filter_map(|el| el.value().attr(attr).map(|v| v.to_string()))
            .collect())
    }

    /// Extract inner HTML of elements matching a selector.
    pub fn extract_html(html: &Html, selector: &str) -> Result<Vec<String>, ParseError> {
        let sel = Selector::parse(selector).map_err(|e| {
            ParseError::InvalidSelector(selector.to_string(), e.to_string())
        })?;
        Ok(html.select(&sel).map(|el| el.html()).collect())
    }
}

// ─── Extraction Engine ─────────────────────────────────────────────────────

/// High-level API that combines a Fetcher with extraction rules.
pub struct ExtractionEngine<F: Fetcher> {
    fetcher: F,
}

impl<F: Fetcher> ExtractionEngine<F> {
    pub fn new(fetcher: F) -> Self {
        Self { fetcher }
    }

    /// Run a job and return the structured result.
    pub async fn run(&self, job: &ScrapeJob) -> ScrapeResult {
        self.fetcher.scrape(job).await
    }

    /// Run a job with retries.
    pub async fn run_with_retries(&self, job: &ScrapeJob) -> ScrapeResult {
        let max_retries = job.retries;
        let mut last_error: Option<ScrapeError> = None;

        for attempt in 0..=max_retries {
            let result = self.fetcher.scrape(job).await;
            if result.status == shared::JobStatus::Done {
                return result;
            }
            last_error = Some(ScrapeError::Parse(ParseError::Extraction(
                job.job_name.clone(),
                result.error.clone().unwrap_or_default(),
            )));
            if attempt < max_retries {
                // Simple exponential back-off: 1s, 2s, 4s...
                let delay = 2u64.pow(attempt);
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
        }

        // All retries exhausted.
        ScrapeResult::failure(
            &job.job_name,
            &job.url,
            last_error.map(|e| e.to_string()).unwrap_or_else(|| "unknown error".into()),
        )
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shared::ExtractionRule;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    // ── Offline test helpers ────────────────────────────────────────────────

    /// A fetcher that returns fixed HTML, for deterministic offline pipeline tests.
    struct StaticFetcher {
        html: String,
    }

    impl Fetcher for StaticFetcher {
        async fn fetch_html(&self, _job: &ScrapeJob) -> Result<String, FetchError> {
            Ok(self.html.clone())
        }
    }

    /// A fetcher that fails `fail_times` then returns empty HTML, to exercise retries.
    struct FlakyFetcher {
        fail_times: u32,
        calls: Arc<AtomicU32>,
    }

    impl Fetcher for FlakyFetcher {
        async fn fetch_html(&self, _job: &ScrapeJob) -> Result<String, FetchError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_times {
                Err(FetchError::Network(format!("synthetic failure #{n}")))
            } else {
                // Empty doc: no extraction rules → success with empty data.
                Ok("<html></html>".to_string())
            }
        }
    }

    fn job_with(html_url: &str, rules: Vec<ExtractionRule>, retries: u32) -> ScrapeJob {
        ScrapeJob {
            job_name: "t".into(),
            url: html_url.into(),
            schedule: None,
            fetcher: shared::FetcherType::Http,
            headers: Default::default(),
            extraction: rules,
            output: None,
            retries,
        }
    }

    fn rule(selector: &str, name: &str, attr: Option<&str>) -> ExtractionRule {
        ExtractionRule {
            selector: selector.into(),
            name: name.into(),
            attribute: attr.map(|s| s.into()),
        }
    }

    // ── extract() behavior ──────────────────────────────────────────────────

    #[test]
    fn extract_single_match_is_scalar() {
        let f = StaticFetcher { html: String::new() };
        let html = r#"<div class="x">only</div>"#;
        let data = f.extract("j", "u", html, &[rule(".x", "x", None)]).unwrap();
        assert_eq!(data["x"], serde_json::json!("only"));
    }

    #[test]
    fn extract_multiple_matches_is_array() {
        let f = StaticFetcher { html: String::new() };
        let html = r#"<a class="x">1</a><a class="x">2</a>"#;
        let data = f.extract("j", "u", html, &[rule(".x", "x", None)]).unwrap();
        assert_eq!(data["x"], serde_json::json!(["1", "2"]));
    }

    #[test]
    fn extract_attribute_present_and_missing() {
        let f = StaticFetcher { html: String::new() };
        // First link has href, second does not. A missing attribute yields JSON
        // `null` (from `unwrap_or_default()` on the Option<Value>), not "".
        let html = r#"<a class="l" href="/p1">a</a><a class="l">b</a>"#;
        let data = f
            .extract("j", "u", html, &[rule(".l", "link", Some("href"))])
            .unwrap();
        assert_eq!(data["link"], serde_json::json!(["/p1", null]));
    }

    #[test]
    fn extract_no_match_rule_is_skipped() {
        let f = StaticFetcher { html: String::new() };
        let html = r#"<p>hi</p>"#;
        let data = f
            .extract("j", "u", html, &[rule(".missing", "gone", None)])
            .unwrap();
        assert!(data.get("gone").is_none());
        assert_eq!(data, serde_json::json!({}));
    }

    #[test]
    fn extract_multiple_rules_merge_into_one_object() {
        let f = StaticFetcher { html: String::new() };
        let html = r#"<h1 class="t">Title</h1><span class="a" data-id="9">x</span>"#;
        let data = f
            .extract(
                "j",
                "u",
                html,
                &[
                    rule(".t", "title", None),
                    rule(".a", "id", Some("data-id")),
                ],
            )
            .unwrap();
        assert_eq!(data["title"], serde_json::json!("Title"));
        assert_eq!(data["id"], serde_json::json!("9"));
    }

    #[test]
    fn extract_invalid_selector_errors() {
        let f = StaticFetcher { html: String::new() };
        let err = f
            .extract("j", "u", "<p>x</p>", &[rule("p => span", "bad", None)])
            .unwrap_err();
        assert!(matches!(err, ParseError::InvalidSelector(_, _)));
    }

    // ── Full pipeline + retries (offline, time paused) ──────────────────────

    #[tokio::test]
    async fn scrape_pipeline_success_offline() {
        let f = StaticFetcher {
            html: r#"<a class="x">hello</a>"#.into(),
        };
        let engine = ExtractionEngine::new(f);
        let job = job_with("https://example.com", vec![rule(".x", "x", None)], 0);
        let result = engine.run(&job).await;
        assert_eq!(result.status, shared::JobStatus::Done);
        assert_eq!(result.data["x"], serde_json::json!("hello"));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_succeed_after_transient_failures() {
        let calls = Arc::new(AtomicU32::new(0));
        let f = FlakyFetcher {
            fail_times: 2,
            calls: calls.clone(),
        };
        let engine = ExtractionEngine::new(f);
        // retries=3 → up to 4 attempts; fails twice then succeeds on the 3rd.
        let job = job_with("https://example.com", Vec::new(), 3);
        let result = engine.run_with_retries(&job).await;
        assert_eq!(result.status, shared::JobStatus::Done);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_exhausted_returns_error() {
        let calls = Arc::new(AtomicU32::new(0));
        let f = FlakyFetcher {
            fail_times: 100, // always fails within the budget
            calls: calls.clone(),
        };
        let engine = ExtractionEngine::new(f);
        let job = job_with("https://example.com", Vec::new(), 2);
        let result = engine.run_with_retries(&job).await;
        assert_eq!(result.status, shared::JobStatus::Error);
        assert!(result.error.is_some());
        // retries=2 → exactly 3 attempts.
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_http_client_fetch() {
        let client = HttpClient::new();
        let job = ScrapeJob {
            job_name: "test".into(),
            url: "https://httpbin.org/html".into(),
            schedule: None,
            fetcher: shared::FetcherType::Http,
            headers: Default::default(),
            extraction: Vec::new(),
            output: None,
            retries: 0,
        };
        let html = client.fetch_html(&job).await;
        assert!(html.is_ok(), "HTTP fetch should succeed");
    }

    #[tokio::test]
    async fn test_extraction_engine() {
        let client = HttpClient::new();
        let engine = ExtractionEngine::new(client);

        // Scrape a page with known structure.
        let job = ScrapeJob {
            job_name: "news_test".into(),
            url: "https://news.ycombinator.com/".into(),
            schedule: None,
            fetcher: shared::FetcherType::Http,
            headers: Default::default(),
            extraction: vec![
                ExtractionRule {
                    selector: ".titleline > a".into(),
                    name: "title".into(),
                    attribute: None,
                },
                ExtractionRule {
                    selector: ".age a".into(),
                    name: "time".into(),
                    attribute: None,
                },
            ],
            output: None,
            retries: 0,
        };

        let result = engine.run(&job).await;
        assert_eq!(result.status, shared::JobStatus::Done);
        assert!(result.data.get("title").is_some());
    }

    #[test]
    fn test_html_parser_extract_text() {
        let html = r#"<html><body><h1 class="title">Hello</h1><h1 class="title">World</h1></body></html>"#;
        let doc = HtmlParser::parse(html);
        let results = HtmlParser::extract_text(&doc, "h1.title").unwrap();
        assert_eq!(results, vec!["Hello", "World"]);
    }

    #[test]
    fn test_html_parser_extract_attr() {
        let html = r#"<html><body><a href="/page1">Link1</a><a href="/page2">Link2</a></body></html>"#;
        let doc = HtmlParser::parse(html);
        let results = HtmlParser::extract_attr(&doc, "a", "href").unwrap();
        assert_eq!(results, vec!["/page1", "/page2"]);
    }

    #[test]
    fn test_invalid_selector() {
        let html = r#"<html><body><p>test</p></body></html>"#;
        let doc = HtmlParser::parse(html);
        // "=>" is not valid CSS selector syntax
        let result = HtmlParser::extract_text(&doc, "p => span");
        assert!(result.is_err());
    }
}
