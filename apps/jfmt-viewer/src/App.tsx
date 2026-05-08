import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { closeFile, NodeId, openFile } from "./api";
import { Tree, TreeHandle } from "./components/Tree";
import { Preview } from "./components/Preview";
import { SearchBar } from "./components/SearchBar";
import { HitList } from "./components/HitList";
import { useSearch } from "./lib/searchState";
import { copyPointer } from "./lib/clipboard";

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
  const [pointerHint, setPointerHint] = useState<string>("");

  const sessionId = session?.sessionId ?? null;
  const { state: searchState, start: startSearch, cancel: cancelSearchOp } = useSearch(sessionId);
  const [searchCursor, setSearchCursor] = useState(0);

  useEffect(() => {
    setSearchCursor(0);
  }, [searchState.query.needle, searchState.query.scope, searchState.query.case_sensitive]);

  const treeRef = useRef<TreeHandle>(null);

  async function jumpToHit(idx: number) {
    setSearchCursor(idx);
    const hit = searchState.hits[idx];
    if (!hit) return;
    const id = await treeRef.current?.expandToPointer(hit.path);
    if (id !== null && id !== undefined) {
      setSelected(id);
    }
  }

  useEffect(() => {
    if (searchState.hits.length > 0 && searchCursor < searchState.hits.length) {
      jumpToHit(searchCursor);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchCursor]);

  async function copyCurrentPointer() {
    if (!session || selected === null) return;
    const p = await copyPointer(session.sessionId, selected);
    setPointerHint(`copied: ${p || "(root)"}`);
    setTimeout(() => setPointerHint(""), 2000);
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const isCopy = (e.ctrlKey || e.metaKey) && e.key === "c";
      if (isCopy && session && selected !== null) {
        const sel = window.getSelection();
        if (sel && sel.toString().length === 0) {
          e.preventDefault();
          copyCurrentPointer();
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session, selected]);

  useEffect(() => {
    const url = new URL(window.location.href);
    const f = url.searchParams.get("file");
    if (f) {
      (async () => {
        setProgress("opening…");
        const resp = await openFile(f, (p) => {
          if (p.phase === "ready") setProgress(`ready (${p.build_ms} ms)`);
        });
        setSession({
          sessionId: resp.session_id,
          rootId: resp.root_id,
          path: f,
          totalBytes: resp.total_bytes,
          format: resp.format,
        });
      })();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
        {session && selected !== null && (
          <button
            onClick={copyCurrentPointer}
            title="Copy JSON Pointer (Ctrl+C with no text selected)"
            style={{ marginLeft: 8 }}
          >
            📋 Copy ptr
          </button>
        )}
        {pointerHint && (
          <span style={{ marginLeft: 8, color: "#080", fontSize: 12 }}>
            {pointerHint}
          </span>
        )}
        {session && (
          <span style={{ marginLeft: 16 }}>
            <SearchBar
              state={searchState}
              cursor={searchCursor}
              onCursorChange={setSearchCursor}
              onQuery={startSearch}
              onCancel={cancelSearchOp}
            />
          </span>
        )}
      </header>
      {session && (
        <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
          <HitList state={searchState} cursor={searchCursor} onPick={jumpToHit} />
          <div style={{ flex: "0 0 40%", borderRight: "1px solid #ddd" }}>
            <Tree
              ref={treeRef}
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
