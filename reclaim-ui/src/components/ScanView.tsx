import { useState } from "react";
import { listCandidates } from "../api/commands";
import type { Candidate } from "../api/types";
import { TIER_LABEL, formatBytes, isSelectable } from "../lib/format";
import "./ScanView.css";

type ScanViewProps = {
  root: string;
  onRootChange: (root: string) => void;
  candidates: Candidate[];
  onCandidates: (candidates: Candidate[]) => void;
  selected: Set<string>;
  onToggleSelected: (path: string) => void;
  onReview: () => void;
};

export default function ScanView({
  root,
  onRootChange,
  candidates,
  onCandidates,
  selected,
  onToggleSelected,
  onReview,
}: ScanViewProps) {
  const [pathInput, setPathInput] = useState(root);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const runScan = async () => {
    setScanning(true);
    setError(null);
    try {
      const report = await listCandidates(pathInput || undefined);
      onCandidates([
        ...report.safe,
        ...report.probably_safe,
        ...report.unknown,
        ...report.never_touch,
      ]);
      onRootChange(report.root);
    } catch (err) {
      setError(String(err));
    } finally {
      setScanning(false);
    }
  };

  const selectedBytes = candidates
    .filter((c) => selected.has(c.path))
    .reduce((sum, c) => sum + c.dedup_size, 0);

  return (
    <div className="scan-view">
      <div className="scan-header">
        <h2>Scan &amp; clean</h2>
        <div className="scan-controls">
          <input
            type="text"
            className="scan-path-input"
            placeholder="Folder to scan (defaults to current directory)"
            value={pathInput}
            onChange={(e) => setPathInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && runScan()}
          />
          <button
            type="button"
            className="btn btn-primary"
            onClick={runScan}
            disabled={scanning}
          >
            {scanning ? "Scanning…" : "Scan"}
          </button>
        </div>
        {error && <p className="scan-error">{error}</p>}
      </div>

      {candidates.length === 0 && !scanning && !error && (
        <p className="scan-empty">Scan a folder to see cleanup candidates.</p>
      )}

      {candidates.length > 0 && (
        <>
          <div className="scan-list">
            {candidates.map((c) => {
              const selectable = isSelectable(c);
              const checked = selected.has(c.path);
              return (
                <label
                  key={c.path}
                  className={selectable ? "scan-row" : "scan-row scan-row-disabled"}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={!selectable}
                    onChange={() => onToggleSelected(c.path)}
                  />
                  <span className={`tier-badge tier-badge-${c.tier}`}>
                    {TIER_LABEL[c.tier]}
                  </span>
                  <span className="scan-row-path" title={c.path}>
                    {c.path}
                  </span>
                  <span className="scan-row-size">{formatBytes(c.dedup_size)}</span>
                </label>
              );
            })}
          </div>

          <div className="scan-footer">
            <div className="scan-footer-total">
              <strong>{selected.size}</strong> item{selected.size === 1 ? "" : "s"} selected
              {" · "}
              <strong>{formatBytes(selectedBytes)}</strong> pending reclaim
            </div>
            <button
              type="button"
              className="btn btn-primary"
              disabled={selected.size === 0}
              onClick={onReview}
            >
              Review selection →
            </button>
          </div>
        </>
      )}
    </div>
  );
}
