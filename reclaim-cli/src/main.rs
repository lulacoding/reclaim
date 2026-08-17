//! `dq` — the Reclaim command-line interface.

mod clean_command;
mod list_command;
mod scan_command;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use reclaim_core::Tier;

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
    /// Scan + classify a directory and list candidates grouped by tier.
    /// Strictly read-only: never mutates the filesystem or the store.
    List {
        /// Directory to scan. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Only show candidates in this tier.
        #[arg(long)]
        tier: Option<TierArg>,
        /// Print the report as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum TierArg {
    Safe,
    ProbablySafe,
    Unknown,
    NeverTouch,
}

impl From<TierArg> for Tier {
    fn from(arg: TierArg) -> Self {
        match arg {
            TierArg::Safe => Tier::Safe,
            TierArg::ProbablySafe => Tier::ProbablySafe,
            TierArg::Unknown => Tier::Unknown,
            TierArg::NeverTouch => Tier::NeverTouch,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { path, json, db } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            let db = db.unwrap_or_else(scan_command::default_db_path);
            scan_command::run(&root, json, &db);
        }
        Command::List { path, tier, json } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            list_command::run(&root, tier.map(Into::into), json);
        }
    }
}
