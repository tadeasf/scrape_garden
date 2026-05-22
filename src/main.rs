//! scrape-garden: A high-performance, Rust-native web scraping framework.

use clap::{Parser, Subcommand};
use scheduler::run_all;
use shared::DashboardMessage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tui::run_tui;

#[derive(Parser, Debug)]
#[command(name = "scrape-garden", about = "A high-performance, Rust-native web scraping framework 🌿")]
struct Cli {
    /// Path to the configuration file (YAML).
    #[arg(short, long, default_value = "config.yaml")]
    config: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the scheduler daemon (with optional TUI).
    Daemon,
    /// Run all jobs once (one-shot mode).
    Run,
    /// Run the TUI dashboard only (connects to daemon).
    Tui,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run) => {
            // One-shot mode: run all jobs immediately.
            run_all(&cli.config).await?;
        }
        Some(Commands::Tui) => {
            // TUI-only mode: need a channel for messages.
            // In practice, this would connect to a running daemon via IPC.
            // For now, we create a local channel and run the TUI.
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<DashboardMessage>();
            // Send a welcome message.
            let _ = tx.send(DashboardMessage::Log(shared::LogEntry {
                timestamp: chrono::Utc::now(),
                level: "INFO".into(),
                message: "TUI running in standalone mode. No daemon connected.".into(),
            }));
            run_tui(rx).await?;
        }
        Some(Commands::Daemon) | None => {
            // Default: run daemon + TUI together.
            let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel::<DashboardMessage>();

            // Load config.
            let config = scheduler::ConfigLoader::load(&cli.config)?;
            scheduler::ConfigLoader::validate(&config)?;

            println!("🌿 scrape-garden v{}", env!("CARGO_PKG_VERSION"));
            println!("   Config: {}", cli.config);
            println!("   Jobs: {}", config.jobs.len());
            println!();

            // Spawn the scheduler.
            let scheduler_handle = tokio::spawn(async move {
                let scheduler = scheduler::Scheduler::new(config, log_tx);
                if let Err(e) = scheduler.run().await {
                    eprintln!("❌ Scheduler error: {}", e);
                }
            });

            // Spawn the TUI.
            let tui_handle = tokio::spawn(async move {
                if let Err(e) = run_tui(log_rx).await {
                    eprintln!("❌ TUI error: {}", e);
                }
            });

            // Wait for either to finish.
            let running = Arc::new(AtomicBool::new(true));
            let running_clone = running.clone();
            let ctrl_c = tokio::signal::ctrl_c();
            tokio::spawn(async move {
                let _ = ctrl_c.await;
                running_clone.store(false, Ordering::SeqCst);
            });

            while running.load(Ordering::SeqCst) {
                tokio::select! {
                    _ = scheduler_handle => {
                        println!("\n🌿 Scheduler finished.");
                        break;
                    }
                    _ = tui_handle => {
                        println!("\n🌿 TUI exited.");
                        break;
                    }
                }
            }

            println!("🌿 scrape-garden shutting down...");
        }
    }

    Ok(())
}
