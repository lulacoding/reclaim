import { getCurrentWindow } from "@tauri-apps/api/window";
import "./TitleBar.css";

const appWindow = getCurrentWindow();

export default function TitleBar() {
  return (
    <div className="titlebar">
      <div className="titlebar-drag" data-tauri-drag-region>
        <div className="titlebar-mark" />
        <span className="titlebar-title">Reclaim</span>
      </div>
      <div className="titlebar-controls">
        <button
          type="button"
          className="titlebar-btn"
          aria-label="Minimize"
          onClick={() => appWindow.minimize()}
        >
          <svg width="11" height="11" viewBox="0 0 24 24">
            <line x1="4" y1="12" x2="20" y2="12" stroke="currentColor" strokeWidth="1.6" />
          </svg>
        </button>
        <button
          type="button"
          className="titlebar-btn"
          aria-label="Maximize"
          onClick={() => appWindow.toggleMaximize()}
        >
          <svg width="10" height="10" viewBox="0 0 24 24">
            <rect x="4" y="4" width="16" height="16" rx="1.5" stroke="currentColor" strokeWidth="1.6" fill="none" />
          </svg>
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-btn-close"
          aria-label="Close"
          onClick={() => appWindow.close()}
        >
          <svg width="11" height="11" viewBox="0 0 24 24">
            <line x1="5" y1="5" x2="19" y2="19" stroke="currentColor" strokeWidth="1.6" />
            <line x1="19" y1="5" x2="5" y2="19" stroke="currentColor" strokeWidth="1.6" />
          </svg>
        </button>
      </div>
    </div>
  );
}
