//! `dq` — the Reclaim command-line interface.

mod clean_command;
mod list_command;
mod restore_command;
mod scan_command;
mod store_util;

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
    /// Preview or execute quarantining Safe (and cheap Probably-safe)
    /// candidates. Never touches Unknown or expensive Probably-safe items.
    Clean {
        /// Directory to scan. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Only clean candidates in this tier.
        #[arg(long)]
        tier: Option<TierArg>,
        /// Show the plan; mutate nothing.
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt and quarantine immediately.
        #[arg(long)]
        yes: bool,
        /// Print the plan/result as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// State database path. Defaults to `%LOCALAPPDATA%\Reclaim\reclaim.db`.
        #[arg(long)]
        db: Option<PathBuf>,
        /// Quarantine staging root. Defaults to `%LOCALAPPDATA%\Reclaim\quarantine`.
        #[arg(long = "quarantine-dir")]
        quarantine_dir: Option<PathBuf>,
        /// Days before a quarantined item is eligible for purge. Clamped up to 30.
        #[arg(long = "grace-period-days")]
        grace_period_days: Option<u32>,
    },
    /// Restore quarantined items back to their original locations.
    Restore {
        /// The operation id to restore. Mutually exclusive with --last/--all.
        op_id: Option<i64>,
        /// Restore the most recently completed quarantine operation.
        #[arg(long)]
        last: bool,
        /// Restore everything currently in quarantine, across every operation.
        #[arg(long)]
        all: bool,
        /// Print the result as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// State database path. Defaults to `%LOCALAPPDATA%\Reclaim\reclaim.db`.
        #[arg(long)]
        db: Option<PathBuf>,
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
        Command::Clean {
            path,
            tier,
            dry_run,
            yes,
            json,
            db,
            quarantine_dir,
            grace_period_days,
        } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            let db = db.unwrap_or_else(scan_command::default_db_path);
            let quarantine_dir = quarantine_dir.unwrap_or_else(clean_command::default_quarantine_dir);
            let grace = grace_period_days
                .unwrap_or(reclaim_core::MIN_GRACE_PERIOD_DAYS)
                .max(reclaim_core::MIN_GRACE_PERIOD_DAYS);
            clean_command::run(
                &root,
                tier.map(Into::into),
                dry_run,
                yes,
                json,
                &db,
                &quarantine_dir,
                grace,
            );
        }
        Command::Restore { op_id, last, all, json, db } => {
            let db = db.unwrap_or_else(scan_command::default_db_path);
            let target = match (op_id, last, all) {
                (Some(id), false, false) => restore_command::RestoreTarget::OpId(id),
                (None, true, false) => restore_command::RestoreTarget::Last,
                (None, false, true) => restore_command::RestoreTarget::All,
                _ => {
                    eprintln!("specify exactly one of: <OP_ID>, --last, --all");
                    std::process::exit(2);
                }
            };
            restore_command::run(target, &db, json);
        }
    }
}
