import { useEffect, useState } from "react";
import { cleanDryRun, runClean } from "../api/commands";
import type { CleanExecution, CleanPlan } from "../api/types";
import { TIER_LABEL, formatBytes } from "../lib/format";
import "./ReviewView.css";

type ReviewViewProps = {
  root: string;
  selected: Set<string>;
  onBack: () => void;
  onDone: () => void;
};

type ConfirmStage = "idle" | "armed" | "running" | "done";

// How long the "Confirm: move N items?" arming state stays up before it
// quietly disarms -- deliberate friction (a second, explicit click),
// without a scary modal dialog.
const ARM_TIMEOUT_MS = 6000;

export default function ReviewView({ root, selected, onBack, onDone }: ReviewViewProps) {
  const [plan, setPlan] = useState<CleanPlan | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [stage, setStage] = useState<ConfirmStage>("idle");
  const [result, setResult] = useState<CleanExecution | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    cleanDryRun(root, [...selected])
      .then((p) => {
        if (!cancelled) setPlan(p);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [root]);

  useEffect(() => {
    if (stage !== "armed") return;
    const timer = window.setTimeout(() => setStage("idle"), ARM_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [stage]);

  const confirm = async () => {
    if (stage !== "armed") {
      setStage("armed");
      return;
    }
    setStage("running");
    setError(null);
    try {
      const execution = await runClean(root, [...selected]);
      setResult(execution);
      setStage("done");
    } catch (err) {
      setError(String(err));
      setStage("idle");
    }
  };

  if (loading) {
    return (
      <div className="review-view">
        <h2>Review</h2>
        <p className="review-loading">Running dry run…</p>
      </div>
    );
  }

  if (error && !plan) {
    return (
      <div className="review-view">
        <h2>Review</h2>
        <p className="review-error">{error}</p>
        <button type="button" className="btn btn-ghost" onClick={onBack}>
          ← Back to selection
        </button>
      </div>
    );
  }

  if (!plan) return null;

  if (stage === "done" && result) {
    return (
      <div className="review-view">
        <h2>Moved to quarantine</h2>
        <p className="review-done-summary">
          {result.quarantined.length} item{result.quarantined.length === 1 ? "" : "s"} moved
          {" · "}
          {formatBytes(result.pending_reclaim_bytes)} pending reclaim.
        </p>
        {result.failures.length > 0 && (
          <div className="review-failures">
            <h3>Couldn&apos;t move</h3>
            <ul>
              {result.failures.map((f) => (
                <li key={f.path}>
                  <span className="review-row-path">{f.path}</span>: {f.error}
                </li>
              ))}
            </ul>
          </div>
        )}
        <button type="button" className="btn btn-primary" onClick={onDone}>
          Back to Scan &amp; clean
        </button>
      </div>
    );
  }

  return (
    <div className="review-view">
      <h2>Review</h2>

      <div className="review-reassurance">
        These move to quarantine — restore anytime for 30 days. Nothing is deleted.
      </div>

      {error && <p className="review-error">{error}</p>}

      <div className="review-list">
        {plan.items.length === 0 && (
          <p className="review-empty">Nothing left to review — the selection cleared.</p>
        )}
        {plan.items.map((item) => (
          <div className="review-row" key={item.path}>
            <span className={`tier-badge tier-badge-${item.tier}`}>{TIER_LABEL[item.tier]}</span>
            <span className="review-row-path" title={item.path}>
              {item.path}
            </span>
            <span className="review-row-method">Quarantine move</span>
            <span className="review-row-size">{formatBytes(item.dedup_size)}</span>
          </div>
        ))}
      </div>

      <div className="review-totals">
        <div className="review-totals-item">
          <span className="review-totals-label">Pending reclaim (moves)</span>
          <strong>{formatBytes(plan.planned_bytes)}</strong>
        </div>
        <div className="review-totals-item">
          <span className="review-totals-label">Reclaimed (native commands)</span>
          <strong>{formatBytes(plan.store_estimated_bytes)}</strong>
        </div>
      </div>

      <div className="review-actions">
        <button type="button" className="btn btn-ghost" onClick={onBack} disabled={stage === "running"}>
          ← Back to selection
        </button>

        {stage === "armed" ? (
          <div className="review-confirm-armed">
            <button type="button" className="btn btn-ghost" onClick={() => setStage("idle")}>
              Cancel
            </button>
            <button type="button" className="btn btn-danger" onClick={confirm}>
              Confirm: move {plan.items.length} item{plan.items.length === 1 ? "" : "s"} (
              {formatBytes(plan.planned_bytes)})?
            </button>
          </div>
        ) : (
          <button
            type="button"
            className="btn btn-primary"
            disabled={plan.items.length === 0 || stage === "running"}
            onClick={confirm}
          >
            {stage === "running"
              ? "Moving…"
              : `Move ${plan.items.length} item${plan.items.length === 1 ? "" : "s"} to quarantine`}
          </button>
        )}
      </div>
    </div>
  );
}
