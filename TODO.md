# Project Specification: scrape-garden 🌿

## 1. Overview
Build `scrape-garden`, a high-performance, Rust-native web scraping framework. Unlike traditional Python scrapers (like Scrapy), this tool will be designed for low latency and minimal resource usage. It features a background scheduling engine, dual-mode fetching (HTTP + Headless Browser), structured data extraction, and an interactive TUI dashboard.

## 2. Tech Stack
*   **Language:** Rust (Edition 2021/2024)
*   **Async Runtime:** `tokio`
*   **HTTP Client:** `reqwest` (fast, blocking/non-blocking)
*   **Browser Automation:** `playwright` (Rust binding for Playwright protocol) or `headless_chrome`
*   **Serialization:** `serde`, `serde_yaml` (for config files)
*   **TUI:** `ratatui` + `crossterm`
*   **Scheduler:** `tokio-cron-scheduler`
*   **Storage:** `sqlite` (via `sqlx` or `rusqlite`)

## 3. Architecture & Directory Structure
The project will be a single binary (`scrape-garden`) that runs two main components:
1.  **Background Daemon:** Handles scheduling, fetching, and saving data.
2.  **Foreground CLI/TUI:** Connects to the daemon (via IPC or shared state) to display live stats.

**Proposed Directory Structure:**
```text
scrape-garden/
├── Cargo.toml
├── config.yaml          # Global config (defaults, storage paths)
├── crates/
│   ├── scraper-core     # The engine: Fetchers, Parsers, Extractors
│   ├── scheduler        # Cron jobs and job management
│   ├── tui              # Terminal UI components
│   └── shared           # Shared types (Jobs, Results, Config)
├── examples/            # Example scraping recipes
└── src/main.rs          # Entry point
```

---

## 4. Implementation Phases

### Phase 1: Foundation & HTTP Engine
**Goal:** Set up the project structure and build a basic scraper that fetches HTML over HTTP.

*   **Setup:** Initialize Cargo workspace. Add dependencies (`reqwest`, `tokio`, `scraper` for parsing).
*   **Structs:** Define `ScrapeJob` (URL, Method, Headers) and `ScrapeResult`.
*   **Fetcher:** Implement an `HttpClient` struct using `reqwest` that returns the raw HTML string.
*   **Parser:** Use the `scraper` crate to implement basic CSS selector extraction from the HTML.

### Phase 2: Headless Browser Integration (The "Greenhouse")
**Goal:** Add support for JavaScript-heavy sites using Playwright.

*   **Dependency:** Integrate the `playwright` crate.
*   **Browser Client:** Create a `BrowserClient` that manages a persistent browser instance (lazy initialization).
*   **Navigation:** Implement methods to navigate to URLs, wait for network idle or specific selectors (`wait_for_selector`).
*   **Extraction:** Allow extracting text or inner HTML from the DOM after JS execution.

### Phase 3: Structured Extraction & Storage
**Goal:** Define how data is extracted and saved.

*   **Config Schema:** Design a YAML schema for "Recipes."
    ```yaml
    job_name: "tech_news"
    url: "https://example.com/news"
    schedule: "0 */6 * * *" # every 6 hours
    extraction:
      - selector: ".article-title"
        name: "title"
      - selector: ".date"
        name: "date"
    output: "data.jsonl"
    ```
*   **Storage:** Implement a simple writer that saves results to JSONL (JSON Lines) or SQLite.

### Phase 4: Scheduling Engine
**Goal:** Automate the scraping process.

*   **Scheduler:** Integrate `tokio-cron-scheduler`.
*   **Job Runner:** A loop that reads the config, parses the cron expressions, and triggers the appropriate Fetcher (HTTP vs Browser) when a job is due.
*   **Concurrency:** Use `tokio::spawn` to run multiple jobs in parallel without blocking the main thread.

### Phase 5: TUI Dashboard
**Goal:** Visualize what is happening.

*   **UI Framework:** Set up `ratatui`.
*   **Widgets:**
    *   **Job List:** Table showing Job Name, Status (Running/Done/Error), and Last Run time.
    *   **Log Stream:** A scrolling text area showing real-time output from the scheduler.
    *   **Stats:** Total requests processed, errors count.
*   **Interactivity:** Ability to click a job to see its recent history or logs.

---

## 5. Instructions for the Agent
1.  **Start with Phase 1.** Do not move to browser support until basic HTTP scraping works and tests pass.
2.  **Focus on Abstraction.** Create a trait `Fetcher` that both `HttpClient` and `BrowserClient` implement. This allows the scheduler to treat them identically.
3.  **Error Handling.** Use the `thiserror` crate for clean error types (e.g., `FetchError`, `ParseError`).
4.  **No "God Objects".** Keep the logic modular. The scheduler should just say "Run this job," not know *how* it runs.

---

# Implementation Plan

## Step 1: Initialize Cargo workspace and shared crate
- [ ] Create workspace `Cargo.toml` with 4 member crates
- [ ] Create `crates/shared/Cargo.toml` (serde, serde_yaml, thiserror, chrono, url, serde_json)
- [ ] Create `crates/shared/src/lib.rs` with all shared types:
  - `ScrapeJob` (name, url, schedule, extraction rules, output, fetcher type)
  - `ExtractionRule` (selector, name)
  - `ScrapeResult` (job_name, url, timestamp, status, data, error)
  - `JobStatus` enum (Pending, Running, Done, Error)
  - `FetcherType` enum (Http, Browser)
  - `Config` (global settings + list of jobs)
  - Error types: `SharedError`
- [x] Verify: `cargo build --package shared`

## Step 2: Build scraper-core crate (HTTP fetcher + parser)
- [ ] Create `crates/scraper-core/Cargo.toml` (reqwest, scraper, tokio, shared)
- [ ] Create `crates/scraper-core/src/lib.rs`:
  - `Fetcher` trait: `async fn fetch(&self, job: &ScrapeJob) -> Result<ScrapeResult, FetchError>`
  - `HttpClient` struct implementing Fetcher with reqwest
  - `HtmlParser` struct with CSS selector extraction
  - `ExtractionEngine` — combines fetcher + parser + extraction rules → structured data
  - Error types: `FetchError`, `ParseError`
- [x] Write unit test for HttpClient against a real URL
- [x] Verify: `cargo build --package scraper-core`

## Step 3: Build scheduler crate
- [ ] Create `crates/scheduler/Cargo.toml` (tokio-cron-scheduler, tokio, shared, scraper-core, serde_yaml)
- [ ] Create `crates/scheduler/src/lib.rs`:
  - `ConfigLoader` — reads YAML config file, returns `Config`
  - `Scheduler` — wraps tokio-cron-scheduler, registers jobs from config
  - `JobRunner` — spawns tasks that call ExtractionEngine per job
  - `ResultWriter` — saves ScrapeResult to JSONL files
  - `run_daemon` — main async entry point for the background daemon
- [x] Verify: `cargo build --package scheduler`

## Step 4: Build TUI crate
- [ ] Create `crates/tui/Cargo.toml` (ratatui, crossterm, shared, tokio)
- [ ] Create `crates/tui/src/lib.rs`:
  - `TuiApp` — main application state (job list, logs, stats)
  - `render_jobs_table` — displays jobs with status
  - `render_log_stream` — scrolling log area
  - `render_stats` — request counts, error counts
  - `run_tui` — main async entry point
  - Shared state channel for daemon ↔ TUI communication (mpsc)
- [x] Verify: `cargo build --package tui`

## Step 5: Build main binary
- [ ] Create `src/main.rs`:
  - Parse CLI args: `--daemon` flag or `--tui` flag
  - If `--daemon`: spawn scheduler daemon + TUI reader
  - If `--tui`: run TUI dashboard
  - Default: run both daemon + TUI together
- [x] Create `config.yaml` with a sample job
- [x] Verify: `cargo build --bin scrape-garden`

## Step 6: Integration test & polish
- [x] Test full pipeline: config → scheduler → HTTP fetch → parse → JSONL output
- [x] Add `examples/` directory with sample configs
- [x] Add `README.md` with usage instructions
- [x] Verify: `cargo build` and `cargo test` (9 tests pass)

---

# Execution Log

## Step 1: ✅ DONE — Workspace + shared crate
Created workspace, shared crate with all types and error definitions. Verified: `cargo build --package shared`.

## Step 2: ✅ DONE — scraper-core crate
Implemented Fetcher trait, HttpClient, HtmlParser, ExtractionEngine. All 5 tests pass.

## Step 3: ✅ DONE — scheduler crate
Implemented ConfigLoader, Scheduler, JobRunner, ResultWriter, run_daemon. All 4 tests pass.

## Step 4: ✅ DONE — tui crate
Implemented TUI with jobs table, log stream, stats, and shared state.

## Step 5: ✅ DONE — main binary + config
Built main entry point with --daemon / --tui / --run CLI modes. Config YAML with sample jobs.

## Step 6: ✅ DONE — Integration test & polish
Full pipeline verified: config → scheduler → HTTP fetch → parse → JSONL output.
Added examples/ with sample configs and README.md with usage instructions.
All 9 tests pass (5 scraper-core + 4 scheduler).
