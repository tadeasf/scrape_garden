# scrape-garden 🌿

A high-performance, Rust-native web scraping framework designed for low latency and minimal resource usage.

## Features

- **Fast HTTP Fetching** — Built on `reqwest` for high-performance HTTP requests
- **Headless Browser Support** — Extensible `Fetcher` trait for browser automation (Playwright integration planned)
- **Structured Extraction** — CSS selector-based extraction rules with attribute support
- **Cron Scheduling** — Built-in cron-based job scheduling engine
- **Interactive TUI** — Real-time terminal dashboard with job status, logs, and statistics
- **JSONL Output** — Results saved as JSON Lines for easy downstream processing
- **Configurable** — YAML-based configuration with retry logic and custom headers

## Installation

```bash
# Clone the repository
git clone https://github.com/tadeasf/scrape-garden.git
cd scrape-garden

# Build
cargo build --release

# Or install globally
cargo install --path .
```

## Quick Start

### 1. Create a configuration file

```yaml
# config.yaml
output_dir: "data/"
user_agent: "scrape-garden/0.1.0"

jobs:
  - job_name: "hacker_news"
    url: "https://news.ycombinator.com/"
    schedule: "0 */6 * * *"
    fetcher: http
    extraction:
      - selector: ".titleline > a"
        name: "title"
      - selector: ".age a"
        name: "time"
    output: "data/hacker_news.jsonl"
    retries: 3
```

### 2. Run all jobs once

```bash
scrape-garden --config config.yaml run
```

### 3. Run as a daemon with TUI dashboard

```bash
scrape-garden --config config.yaml daemon
```

Or simply (default is daemon mode):

```bash
scrape-garden --config config.yaml
```

### 4. Run TUI only (standalone mode)

```bash
scrape-garden --config config.yaml tui
```

## Configuration Reference

### Global Settings

| Field         | Type   | Description                          | Default    |
|---------------|--------|--------------------------------------|------------|
| `output_dir`  | string | Global output directory for results  | `""`       |
| `user_agent`  | string | Default User-Agent for HTTP requests | `""`       |

### Job Settings

| Field        | Type   | Description                          | Default  |
|--------------|--------|--------------------------------------|----------|
| `job_name`   | string | Unique name for the job              | required |
| `url`        | string | URL to scrape                        | required |
| `schedule`   | string | Cron expression (5-field)            | `""` (run once) |
| `fetcher`    | string | `http` or `browser`                  | `http`   |
| `headers`    | map    | Custom HTTP headers                  | `{}`     |
| `extraction` | array  | CSS selector extraction rules        | `[]`     |
| `output`     | string | Output file path (JSONL)             | `results.jsonl` |
| `retries`    | number | Number of retries on failure         | `3`      |

### Extraction Rules

Each extraction rule defines a CSS selector and output field name:

```yaml
extraction:
  - selector: ".article-title"
    name: "title"           # Extracts text content
  - selector: ".article a"
    name: "link"
    attribute: "href"       # Extracts the href attribute
```

## Cron Scheduling

The scheduler supports standard 5-field cron expressions:

```
* * * * *
│ │ │ │ │
│ │ │ │ └─ Day of week (0-6, Sunday=0)
│ │ │ └─── Month (1-12)
│ │ └───── Day of month (1-31)
│ └─────── Hour (0-23)
└───────── Minute (0-59)
```

Examples:
- `0 */6 * * *` — Every 6 hours at minute 0
- `0 0 * * *` — Daily at midnight
- `*/15 * * * *` — Every 15 minutes
- `30 2 * * 1` — Mondays at 2:30 AM

## TUI Controls

| Key       | Action              |
|-----------|---------------------|
| `↑` / `k` | Move up in job list |
| `↓` / `j` | Move down in job list |
| `q`       | Quit                |
| `r`       | Refresh             |
| `c`       | Clear logs          |

## Architecture

```
scrape-garden/
├── Cargo.toml              # Workspace + binary config
├── config.yaml             # Default configuration
├── crates/
│   ├── shared/             # Shared types (Jobs, Results, Config)
│   ├── scraper-core/       # HTTP fetcher, HTML parser, extraction engine
│   ├── scheduler/          # Cron scheduling, config loading, result writing
│   └── tui/                # Interactive terminal dashboard
├── examples/               # Example configuration files
└── data/                   # Output directory (created at runtime)
```

### Crate Responsibilities

- **shared** — All data types, error definitions, and dashboard state
- **scraper-core** — `Fetcher` trait, `HttpClient`, `HtmlParser`, `ExtractionEngine`
- **scheduler** — `ConfigLoader`, `Scheduler`, `JobRunner`, `ResultWriter`
- **tui** — `ratatui`-based dashboard with job tables, log stream, and stats

## Extending

### Adding a New Fetcher

Implement the `Fetcher` trait from `scraper-core`:

```rust
use scraper_core::{Fetcher, ScrapeJob, ScrapeResult, FetchError};

struct MyCustomFetcher;

impl Fetcher for MyCustomFetcher {
    async fn fetch_html(&self, job: &ScrapeJob) -> Result<String, FetchError> {
        // Your custom fetching logic
        todo!()
    }
}
```

Then use it in your scheduler with `ExtractionEngine::new(MyCustomFetcher)`.

### Adding Browser Support

The `Fetcher` trait is designed to be implemented for browser automation. Add the `playwright` crate to `scraper-core` and implement `BrowserClient`:

```rust
struct BrowserClient {
    browser: playwright::Browser,
}

impl Fetcher for BrowserClient {
    async fn fetch_html(&self, job: &ScrapeJob) -> Result<String, FetchError> {
        // Navigate, wait for JS, extract DOM
        todo!()
    }
}
```

## License

MIT
