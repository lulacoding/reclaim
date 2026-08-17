//! `dq` — the Reclaim command-line interface.

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "dq",
    version,
    about = "Reclaim disk-space forensics + cleanup tool"
)]
struct Cli;

fn main() {
    let _cli = Cli::parse();
    println!("dq {}", env!("CARGO_PKG_VERSION"));
}
