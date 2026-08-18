// Thin wrappers around the `list`/`clean_dry_run`/`clean` Tauri IPC
// commands (reclaim-ui/src-tauri/src/commands/{list,clean}.rs). Argument
// keys are the literal Rust parameter names -- these commands aren't
// annotated with `rename_all = "camelCase"`, so invoke() must match exactly.

import { invoke } from "@tauri-apps/api/core";
import type { CleanExecution, CleanPlan, ListReport } from "./types";

export function listCandidates(path?: string): Promise<ListReport> {
  return invoke("list", { path });
}

export function cleanDryRun(path: string, paths: string[]): Promise<CleanPlan> {
  return invoke("clean_dry_run", { path, paths });
}

export function runClean(path: string, paths: string[]): Promise<CleanExecution> {
  return invoke("clean", { path, paths });
}
