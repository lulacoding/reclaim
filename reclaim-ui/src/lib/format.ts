import type { Candidate, Tier } from "../api/types";

export const TIER_LABEL: Record<Tier, string> = {
  safe: "Safe",
  "probably-safe": "Probably safe",
  unknown: "Unknown",
  "never-touch": "Never touch",
};

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exp = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exp;
  const digits = exp === 0 ? 0 : value < 10 ? 2 : 1;
  return `${value.toFixed(digits)} ${units[exp]}`;
}

// Mirrors reclaim-ui/src-tauri/src/commands/clean.rs's is_selectable:
// Safe, or Probably-safe with a cheap rebuild, and only the Move method.
export function isSelectable(candidate: Candidate): boolean {
  const tierOk =
    candidate.tier === "safe" ||
    (candidate.tier === "probably-safe" && candidate.rebuild_cost === "cheap");
  return tierOk && candidate.method === "move";
}
