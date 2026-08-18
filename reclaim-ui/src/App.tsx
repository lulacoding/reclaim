import { useState } from "react";
import TitleBar from "./components/TitleBar";
import Sidebar, { type NavView } from "./components/Sidebar";
import ScanView from "./components/ScanView";
import ReviewView from "./components/ReviewView";
import type { Candidate } from "./api/types";
import "./App.css";

type PlaceholderView = Exclude<NavView, "scan" | "review">;

const VIEW_COPY: Record<PlaceholderView, { title: string; body: string }> = {
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
  const [root, setRoot] = useState("");
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const toggleSelected = (path: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const finishReview = () => {
    setSelected(new Set());
    setView("scan");
  };

  return (
    <div className="app-shell">
      <TitleBar />
      <div className="app-body">
        <Sidebar active={view} onNavigate={setView} />
        <main className="app-content">
          {view === "scan" && (
            <ScanView
              root={root}
              onRootChange={setRoot}
              candidates={candidates}
              onCandidates={setCandidates}
              selected={selected}
              onToggleSelected={toggleSelected}
              onReview={() => setView("review")}
            />
          )}
          {view === "review" && (
            <ReviewView
              root={root}
              selected={selected}
              onBack={() => setView("scan")}
              onDone={finishReview}
            />
          )}
          {view !== "scan" && view !== "review" && (
            <div className="app-content-placeholder">
              <h2>{VIEW_COPY[view].title}</h2>
              <p>{VIEW_COPY[view].body}</p>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

export default App;
