import type { ReactElement } from "react";
import "./Sidebar.css";

export type NavView = "scan" | "review" | "quarantine" | "settings";

const NAV_ITEMS: { id: NavView; label: string; icon: ReactElement }[] = [
  {
    id: "scan",
    label: "Scan & clean",
    icon: (
      <svg width="17" height="17" viewBox="0 0 24 24" fill="none">
        <circle cx="11" cy="11" r="7" stroke="currentColor" strokeWidth="1.6" />
        <line x1="21" y1="21" x2="16.4" y2="16.4" stroke="currentColor" strokeWidth="1.6" />
      </svg>
    ),
  },
  {
    id: "review",
    label: "Review",
    icon: (
      <svg width="17" height="17" viewBox="0 0 24 24" fill="none">
        <rect x="3" y="5" width="3" height="3" rx="0.6" fill="currentColor" />
        <rect x="3" y="11" width="3" height="3" rx="0.6" fill="currentColor" />
        <rect x="3" y="17" width="3" height="3" rx="0.6" fill="currentColor" />
        <line x1="9" y1="6.5" x2="21" y2="6.5" stroke="currentColor" strokeWidth="1.6" />
        <line x1="9" y1="12.5" x2="21" y2="12.5" stroke="currentColor" strokeWidth="1.6" />
        <line x1="9" y1="18.5" x2="21" y2="18.5" stroke="currentColor" strokeWidth="1.6" />
      </svg>
    ),
  },
  {
    id: "quarantine",
    label: "Quarantine",
    icon: (
      <svg width="17" height="17" viewBox="0 0 24 24" fill="none">
        <path d="M12 3l7 4v5c0 5-3.5 7.5-7 9-3.5-1.5-7-4-7-9V7l7-4z" stroke="currentColor" strokeWidth="1.6" />
        <polyline points="9,12 11,14 15,10" stroke="currentColor" strokeWidth="1.6" fill="none" />
      </svg>
    ),
  },
  {
    id: "settings",
    label: "Settings",
    icon: (
      <svg width="17" height="17" viewBox="0 0 24 24" fill="none">
        <line x1="4" y1="6" x2="20" y2="6" stroke="currentColor" strokeWidth="1.6" />
        <circle cx="9" cy="6" r="2" fill="currentColor" stroke="none" />
        <line x1="4" y1="12" x2="20" y2="12" stroke="currentColor" strokeWidth="1.6" />
        <circle cx="15" cy="12" r="2" fill="currentColor" stroke="none" />
        <line x1="4" y1="18" x2="20" y2="18" stroke="currentColor" strokeWidth="1.6" />
        <circle cx="7" cy="18" r="2" fill="currentColor" stroke="none" />
      </svg>
    ),
  },
];

type SidebarProps = {
  active: NavView;
  onNavigate: (view: NavView) => void;
};

export default function Sidebar({ active, onNavigate }: SidebarProps) {
  return (
    <div className="sidebar">
      <nav className="sidebar-nav">
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            type="button"
            className={
              item.id === active ? "sidebar-item sidebar-item-active" : "sidebar-item"
            }
            aria-current={item.id === active ? "page" : undefined}
            onClick={() => onNavigate(item.id)}
          >
            {item.icon}
            <span>{item.label}</span>
          </button>
        ))}
      </nav>

      <div className="sidebar-spacer" />

      <div className="sidebar-disk">
        <div className="sidebar-disk-label">Disk usage</div>
        <div className="sidebar-disk-track">
          <div className="sidebar-disk-fill" style={{ width: "0%" }} />
        </div>
        <div className="sidebar-disk-meta">Scan a drive to see usage</div>
      </div>
    </div>
  );
}
