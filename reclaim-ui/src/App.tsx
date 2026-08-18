import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

// Scaffold-stage verification only (BUILD_PLAN M5 item 2): calls the
// `scan` IPC command and renders the raw ScanSummary JSON. The real scan
// view (treemap + tiered list) is M5 item 3.
function App() {
  const [path, setPath] = useState("");
  const [result, setResult] = useState<string>("");
  const [error, setError] = useState<string>("");

  async function runScan() {
    setError("");
    setResult("");
    try {
      const summary = await invoke("scan", { path: path || null });
      setResult(JSON.stringify(summary, null, 2));
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <main className="container">
      <h1>Reclaim</h1>

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          runScan();
        }}
      >
        <input
          id="scan-path-input"
          onChange={(e) => setPath(e.currentTarget.value)}
          placeholder="Path to scan (defaults to .)"
        />
        <button type="submit">Scan</button>
      </form>

      {error && <pre style={{ color: "red" }}>{error}</pre>}
      {result && <pre>{result}</pre>}
    </main>
  );
}

export default App;
