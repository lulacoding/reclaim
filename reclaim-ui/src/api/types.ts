// JSON shapes mirroring reclaim-ui/src-tauri/src/commands/{list,clean}.rs
// field-for-field, which in turn mirror docs/reference/CLI.md.

export type Tier = "safe" | "probably-safe" | "unknown" | "never-touch";
export type RebuildCost = "cheap" | "expensive";
export type ReclaimMethod = "move" | { command: string[] };

export interface Candidate {
  path: string;
  logical_size: number;
  dedup_size: number;
  tier: Tier;
  method: ReclaimMethod | null;
  rebuild_cost: RebuildCost | null;
  rule_id: string | null;
}

export interface ListReport {
  root: string;
  safe: Candidate[];
  probably_safe: Candidate[];
  unknown: Candidate[];
  never_touch: Candidate[];
}

export interface CleanItem {
  path: string;
  logical_size: number;
  dedup_size: number;
  tier: Tier;
  rule_id: string | null;
}

export interface StorePlanItem {
  tool: string;
  action: string;
  commands: string[];
  store_dir: string | null;
  tier: Tier;
  rebuild_cost: RebuildCost;
  logical_size: number;
  dedup_size: number;
}

export interface CleanPlan {
  root: string;
  items: CleanItem[];
  planned_bytes: number;
  store_items: StorePlanItem[];
  store_estimated_bytes: number;
}

export interface QuarantinedResult {
  path: string;
  quarantine_path: string;
  logical_size: number;
  dedup_size: number;
}

export interface CleanFailure {
  path: string;
  error: string;
}

export interface StoreCommandOutcome {
  tool: string;
  succeeded: boolean;
  reclaimed_bytes: number;
  steps: string[];
}

export interface CleanExecution {
  quarantined: QuarantinedResult[];
  pending_reclaim_bytes: number;
  store_commands: StoreCommandOutcome[];
  reclaimed_bytes: number;
  failures: CleanFailure[];
}
