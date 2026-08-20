import { useEffect, useState } from "react";
import { getSettings, listProtectedPaths, saveSettings } from "../api/commands";
import "./SettingsView.css";

const MIN_GRACE_PERIOD_DAYS = 30;

export default function SettingsView() {
  const [protectedPaths, setProtectedPaths] = useState<string[]>([]);
  const [gracePeriodDays, setGracePeriodDays] = useState(MIN_GRACE_PERIOD_DAYS);
  const [earlyPurgeEnabled, setEarlyPurgeEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([getSettings(), listProtectedPaths()])
      .then(([settings, paths]) => {
        if (cancelled) return;
        setGracePeriodDays(settings.grace_period_days);
        setEarlyPurgeEnabled(settings.early_purge_enabled);
        setProtectedPaths(paths);
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
  }, []);

  const persist = async (nextGracePeriodDays: number, nextEarlyPurgeEnabled: boolean) => {
    setSaveError(null);
    try {
      await saveSettings(nextGracePeriodDays, nextEarlyPurgeEnabled);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  const onGracePeriodChange = (value: number) => {
    const clamped = Number.isFinite(value)
      ? Math.max(MIN_GRACE_PERIOD_DAYS, Math.round(value))
      : MIN_GRACE_PERIOD_DAYS;
    setGracePeriodDays(clamped);
    void persist(clamped, earlyPurgeEnabled);
  };

  const onEarlyPurgeToggle = (checked: boolean) => {
    setEarlyPurgeEnabled(checked);
    void persist(gracePeriodDays, checked);
  };

  if (loading) {
    return (
      <div className="settings-view">
        <h2>Settings</h2>
        <p className="settings-loading">Loading settings…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="settings-view">
        <h2>Settings</h2>
        <p className="settings-error">{error}</p>
      </div>
    );
  }

  return (
    <div className="settings-view">
      <h2>Settings</h2>

      <section className="settings-section">
        <h3>Protected paths</h3>
        <p className="settings-section-hint">
          Reclaim will never scan, move, or purge anything under these roots.
        </p>
        <ul className="settings-protected-list">
          {protectedPaths.map((path) => (
            <li key={path} className="settings-protected-row" title={path}>
              {path}
            </li>
          ))}
        </ul>
      </section>

      <section className="settings-section">
        <h3>Grace period</h3>
        <p className="settings-section-hint">
          Days a quarantined item is kept before it&apos;s eligible for purge. Minimum{" "}
          {MIN_GRACE_PERIOD_DAYS} days.
        </p>
        <input
          type="number"
          className="settings-grace-input"
          min={MIN_GRACE_PERIOD_DAYS}
          value={gracePeriodDays}
          onChange={(e) => onGracePeriodChange(e.target.valueAsNumber)}
        />
      </section>

      <section className="settings-section">
        <h3>Early purge</h3>
        <label className="settings-toggle-row">
          <input
            type="checkbox"
            checked={earlyPurgeEnabled}
            onChange={(e) => onEarlyPurgeToggle(e.target.checked)}
          />
          <span>Allow purging a quarantined item before its grace period ends</span>
        </label>
        {earlyPurgeEnabled && (
          <div className="settings-warning">
            Purging permanently deletes a quarantined item — this cannot be undone. It&apos;s
            applied one item at a time with its own confirmation, never automatic or scheduled,
            and the protected paths above still apply.
          </div>
        )}
      </section>

      {saveError && <p className="settings-error">{saveError}</p>}
    </div>
  );
}
