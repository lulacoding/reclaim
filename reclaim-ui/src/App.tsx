import { useState } from "react";
import TitleBar from "./components/TitleBar";
import Sidebar, { type NavView } from "./components/Sidebar";
import "./App.css";

const VIEW_COPY: Record<NavView, { title: string; body: string }> = {
  scan: {
    title: "Scan & clean",
    body: "The disk-usage treemap and tiered candidate list land here next.",
  },
  review: {
    title: "Review",
    body: "Selected items will be reviewed here before moving to quarantine.",
  },
  quarantine: {
    title: "Quarantine",
    body: "Quarantined items, their purge deadlines, and restore controls land here.",
  },
  settings: {
    title: "Settings",
    body: "Protected paths, grace period, and other preferences land here.",
  },
};

function App() {
  const [view, setView] = useState<NavView>("scan");
  const copy = VIEW_COPY[view];

  return (
    <div className="app-shell">
      <TitleBar />
      <div className="app-body">
        <Sidebar active={view} onNavigate={setView} />
        <main className="app-content">
          <div className="app-content-placeholder">
            <h2>{copy.title}</h2>
            <p>{copy.body}</p>
          </div>
        </main>
      </div>
    </div>
  );
}

export default App;
