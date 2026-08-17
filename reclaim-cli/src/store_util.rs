//! Shared `Store::open` error handling for the commands that touch the
//! quarantine manifest (`clean`, `restore`, `status`).

use std::path::Path;
use std::process::exit;

use reclaim_store::Store;

pub(crate) fn open_or_exit(db: &Path) -> Store {
    match Store::open(db) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("error: could not open state database at {}: {err}", db.display());
            exit(1);
        }
    }
}
