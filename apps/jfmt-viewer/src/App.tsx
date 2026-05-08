import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { closeFile, openFile } from "./api";
import { Tree } from "./components/Tree";

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

  async function pickFile() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json", "ndjson", "jsonl"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    if (session) await closeFile(session.sessionId);
    setProgress("opening…");
    const resp = await openFile(picked, (p) => {
      if (p.phase === "scanning") {
        setProgress(`scanning: ${p.bytes_done}/${p.bytes_total}`);
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
    <main style={{ fontFamily: "system-ui", padding: 16 }}>
      <h2 style={{ margin: "0 0 8px" }}>jfmt-viewer (M8.1)</h2>
      <button onClick={pickFile}>📁 Open</button>{" "}
      <span style={{ color: "#666" }}>{progress}</span>
      {session && (
        <>
          <h3 style={{ marginTop: 16 }}>
            {session.path} · {session.format} · {session.totalBytes} bytes
          </h3>
          <Tree sessionId={session.sessionId} rootId={session.rootId} />
        </>
      )}
    </main>
  );
}
