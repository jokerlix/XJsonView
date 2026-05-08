import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { closeFile, NodeId, openFile } from "./api";
import { Tree } from "./components/Tree";
import { Preview } from "./components/Preview";

interface OpenSession {
  sessionId: string;
  rootId: number;
  path: string;
  totalBytes: number;
  format: string;
}

export function App() {
  const [session, setSession] = useState<OpenSession | null>(null);
  const [progress, setProgress] = useState<string>("");
  const [selected, setSelected] = useState<NodeId | null>(null);

  async function pickFile() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json", "ndjson", "jsonl"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    if (session) await closeFile(session.sessionId);
    setProgress("opening…");
    setSelected(null);
    const resp = await openFile(picked, (p) => {
      if (p.phase === "scanning") {
        const pct = ((p.bytes_done / Math.max(1, p.bytes_total)) * 100).toFixed(0);
        setProgress(`scanning: ${pct}%`);
      } else if (p.phase === "ready") {
        setProgress(`ready (${p.build_ms} ms)`);
      } else if (p.phase === "error") {
        setProgress(`error: ${p.message}`);
      }
    });
    setSession({
      sessionId: resp.session_id,
      rootId: resp.root_id,
      path: picked,
      totalBytes: resp.total_bytes,
      format: resp.format,
    });
  }

  return (
    <main
      style={{
        fontFamily: "system-ui",
        height: "100vh",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <header style={{ padding: 8, borderBottom: "1px solid #ddd" }}>
        <button onClick={pickFile}>📁 Open</button>{" "}
        <span style={{ color: "#666" }}>{progress}</span>
        {session && (
          <span style={{ marginLeft: 16, color: "#444", fontSize: 12 }}>
            {session.path} · {session.format} · {session.totalBytes} bytes
          </span>
        )}
      </header>
      {session && (
        <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
          <div style={{ flex: "0 0 40%", borderRight: "1px solid #ddd" }}>
            <Tree
              sessionId={session.sessionId}
              rootId={session.rootId}
              onSelect={setSelected}
              selectedId={selected}
            />
          </div>
          <div style={{ flex: 1, overflow: "hidden" }}>
            <Preview sessionId={session.sessionId} node={selected} />
          </div>
        </div>
      )}
    </main>
  );
}
