import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ChildSummary,
  closeFile,
  getPointer,
  NodeId,
  openFile,
  openText,
  SearchQuery,
} from "./api";
import { Tree, TreeHandle } from "./components/Tree";
import { Preview } from "./components/Preview";
import { SearchBar } from "./components/SearchBar";
import { HitList } from "./components/HitList";
import { useSearch } from "./lib/searchState";
import { copyPointer } from "./lib/clipboard";
import { ContextMenu, ContextMenuItem } from "./components/ContextMenu";
import { runExportFlow, runFormatFlow } from "./lib/exportFlow";
import { useTheme } from "./lib/theme";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Clipboard,
  Copy,
  FileJson,
  FolderOpen,
  Moon,
  Sparkles,
  Sun,
} from "lucide-react";

interface OpenSession {
  sessionId: string;
  rootId: number;
  path: string;
  totalBytes: number;
  format: string;
}

function PasteModal({
  open,
  text,
  onChange,
  onClose,
  onSubmit,
}: {
  open: boolean;
  text: string;
  onChange: (s: string) => void;
  onClose: () => void;
  onSubmit: () => void;
}) {
  function tryFormat() {
    try {
      const parsed = JSON.parse(text);
      onChange(JSON.stringify(parsed, null, 2));
    } catch {
      // ignore — invalid JSON; user can fix and try again
    }
  }
  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Paste JSON / NDJSON</DialogTitle>
        </DialogHeader>
        <textarea
          autoFocus
          value={text}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") onSubmit();
          }}
          placeholder='{"foo": [1, 2, 3]}'
          className="min-h-[320px] resize-y rounded-md border border-input bg-background p-3 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <DialogFooter className="!flex-row !justify-between !sm:flex-row !sm:justify-between">
          <span className="self-center text-xs text-muted-foreground">
            {text.length.toLocaleString()} chars · Ctrl+Enter to open
          </span>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={tryFormat} disabled={!text.trim()}>
              <Sparkles className="h-3.5 w-3.5" /> Format
            </Button>
            <Button variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button size="sm" onClick={onSubmit} disabled={!text.trim()}>
              Open
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function App() {
  const { theme, toggle: toggleTheme } = useTheme();
  const [session, setSession] = useState<OpenSession | null>(null);
  const [progress, setProgress] = useState<string>("");
  const [selected, setSelected] = useState<NodeId | null>(null);
  const [selectedLeaf, setSelectedLeaf] = useState<ChildSummary | null>(null);
  const [pasteOpen, setPasteOpen] = useState(false);
  const [pasteText, setPasteText] = useState("");
  const [pointerHint, setPointerHint] = useState<string>("");
  const [menu, setMenu] = useState<{ node: NodeId | null; x: number; y: number } | null>(null);
  const [searchScope, setSearchScope] = useState<NodeId | undefined>(undefined);
  const [scopePath, setScopePath] = useState<string>("");

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
    const result = await treeRef.current?.expandToPointer(hit.path);
    if (!result) return;
    if (result.leaf) {
      setSelected(null);
      setSelectedLeaf(result.leaf);
    } else {
      setSelected(result.node);
      setSelectedLeaf(null);
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

  function menuItems(node: NodeId | null): ContextMenuItem[] {
    if (node === null || !session) return [];
    return [
      { label: "Search from this node", onClick: () => setSearchScope(node) },
      { label: "Export subtree…", onClick: () => exportSubtreeFlow(node) },
    ];
  }

  async function exportSubtreeFlow(node: NodeId) {
    if (!session) return;
    const ptr = await getPointer(session.sessionId, node);
    const safe = (ptr || "root").replace(/[/~]/g, "_").replace(/^_/, "");
    const result = await runExportFlow(session.sessionId, node, `${safe || "root"}.json`);
    if (result) {
      setPointerHint(result);
      setTimeout(() => setPointerHint(""), 4000);
    }
  }

  async function formatWholeFile() {
    if (!session) return;
    const result = await runFormatFlow(session.sessionId, session.rootId, session.path);
    if (result) {
      setPointerHint(result);
      setTimeout(() => setPointerHint(""), 4000);
    }
  }

  function startSearchScoped(q: SearchQuery) {
    return startSearch({ ...q, from_node: searchScope });
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
    (async () => {
      if (!session || searchScope === undefined) {
        setScopePath("");
        return;
      }
      const p = await getPointer(session.sessionId, searchScope);
      setScopePath(p || "/");
    })();
  }, [session, searchScope]);

  useEffect(() => {
    if (searchState.query.needle.trim()) {
      startSearchScoped(searchState.query);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchScope]);

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

  async function submitPaste() {
    const text = pasteText.trim();
    if (!text) return;
    if (session) await closeFile(session.sessionId);
    setProgress("opening pasted text…");
    setSelected(null);
    setSelectedLeaf(null);
    try {
      const resp = await openText(text, (p) => {
        if (p.phase === "scanning") {
          const pct = ((p.bytes_done / Math.max(1, p.bytes_total)) * 100).toFixed(0);
          setProgress(`scanning ${pct}%`);
        } else if (p.phase === "ready") {
          setProgress(`ready (${p.build_ms} ms)`);
        } else if (p.phase === "error") {
          setProgress(`error: ${p.message}`);
        }
      });
      setSession({
        sessionId: resp.session_id,
        rootId: resp.root_id,
        path: "(pasted text)",
        totalBytes: resp.total_bytes,
        format: resp.format,
      });
      setPasteOpen(false);
      setPasteText("");
    } catch (err: unknown) {
      const msg = err && typeof err === "object" && "message" in err
        ? String((err as { message: unknown }).message)
        : String(err);
      setProgress(`error: ${msg}`);
    }
  }

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
        setProgress(`scanning ${pct}%`);
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

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  return (
    <main className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex flex-wrap items-center gap-2 border-b px-3 py-2">
        <div className="flex items-center gap-1.5 pr-2 font-semibold tracking-tight">
          <FileJson className="h-4 w-4 text-primary" /> jfmt
        </div>
        <Button size="sm" variant="outline" onClick={pickFile}>
          <FolderOpen className="h-3.5 w-3.5" /> Open
        </Button>
        <Button size="sm" variant="outline" onClick={() => setPasteOpen(true)}>
          <Clipboard className="h-3.5 w-3.5" /> Paste
        </Button>
        {session && (
          <Button size="sm" variant="outline" onClick={formatWholeFile} title="Export current document, formatted">
            <Sparkles className="h-3.5 w-3.5" /> Format
          </Button>
        )}
        {progress && (
          <span className="text-xs text-muted-foreground">{progress}</span>
        )}
        {session && (
          <span className="ml-1 truncate text-xs text-muted-foreground" title={session.path}>
            <span className="font-medium text-foreground">{session.path}</span>{" "}
            · {session.format} · {fmtBytes(session.totalBytes)}
          </span>
        )}
        {session && selected !== null && (
          <Button
            size="sm"
            variant="ghost"
            onClick={copyCurrentPointer}
            title="Copy JSON Pointer (Ctrl+C with no text selected)"
          >
            <Copy className="h-3.5 w-3.5" />
          </Button>
        )}
        {pointerHint && (
          <span className="text-xs text-emerald-600 dark:text-emerald-400">{pointerHint}</span>
        )}
        {session && (
          <div className="ml-2">
            <SearchBar
              state={searchState}
              cursor={searchCursor}
              onCursorChange={setSearchCursor}
              onQuery={startSearchScoped}
              onCancel={cancelSearchOp}
              scopePath={scopePath}
              onClearScope={() => setSearchScope(undefined)}
            />
          </div>
        )}
        <div className="ml-auto">
          <Button size="icon" variant="ghost" onClick={toggleTheme} title="Toggle theme">
            {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </Button>
        </div>
      </header>
      {session && (
        <div className="flex flex-1 overflow-hidden">
          <HitList
            state={searchState}
            cursor={searchCursor}
            onPick={jumpToHit}
            sessionId={session.sessionId}
            rootId={session.rootId}
          />
          <div className="basis-[40%] border-r overflow-hidden">
            <Tree
              ref={treeRef}
              sessionId={session.sessionId}
              rootId={session.rootId}
              onSelect={(node, leaf) => {
                setSelected(node);
                setSelectedLeaf(leaf ?? null);
              }}
              selectedId={selected}
              onContextMenu={(node, x, y) => setMenu({ node, x, y })}
            />
          </div>
          <div className="flex-1 overflow-hidden">
            <Preview
              sessionId={session.sessionId}
              node={selected}
              leaf={selectedLeaf}
              onExport={exportSubtreeFlow}
            />
          </div>
        </div>
      )}
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems(menu.node)}
          onDismiss={() => setMenu(null)}
        />
      )}
      <PasteModal
        open={pasteOpen}
        text={pasteText}
        onChange={setPasteText}
        onClose={() => setPasteOpen(false)}
        onSubmit={submitPaste}
      />
    </main>
  );
}
