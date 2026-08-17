//! `dq` — the Reclaim command-line interface.

mod scan_command;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "dq",
    version,
    about = "Reclaim disk-space forensics + cleanup tool"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a directory and report size totals. Read-only.
    Scan {
        /// Directory to scan. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Print the summary as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// State database path. Defaults to `%LOCALAPPDATA%\Reclaim\reclaim.db`.
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { path, json, db } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            let db = db.unwrap_or_else(scan_command::default_db_path);
            scan_command::run(&root, json, &db);
        }
    }
}
