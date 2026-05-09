import { useEffect, useRef, useState } from "react";
import { SearchMode, SearchQuery } from "../api";
import { SearchState } from "../lib/searchState";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ChevronDown, ChevronUp, Search, X } from "lucide-react";

interface Props {
  onQuery: (q: SearchQuery) => void;
  onCancel: () => void;
  state: SearchState;
  cursor: number;
  onCursorChange: (next: number) => void;
  scopePath?: string;
  onClearScope?: () => void;
}

const DEBOUNCE_MS = 250;

export function SearchBar({
  onQuery,
  onCancel,
  state,
  cursor,
  onCursorChange,
  scopePath,
  onClearScope,
}: Props) {
  const [needle, setNeedle] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [scope, setScope] = useState<SearchQuery["scope"]>("both");
  const [mode, setMode] = useState<SearchMode>("substring");
  const tRef = useRef<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const onQueryRef = useRef(onQuery);
  const onCancelRef = useRef(onCancel);
  useEffect(() => {
    onQueryRef.current = onQuery;
    onCancelRef.current = onCancel;
  });
  useEffect(() => {
    if (tRef.current !== null) clearTimeout(tRef.current);
    tRef.current = window.setTimeout(() => {
      if (needle.trim() === "") {
        onCancelRef.current();
      } else {
        onQueryRef.current({ needle, mode, case_sensitive: caseSensitive, scope });
      }
    }, DEBOUNCE_MS);
    return () => {
      if (tRef.current !== null) clearTimeout(tRef.current);
    };
  }, [needle, caseSensitive, scope, mode]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.ctrlKey || e.metaKey) && e.key === "f") {
        e.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
        return;
      }
      if (state.hits.length === 0) return;
      if (e.key === "F3" && !e.shiftKey) {
        e.preventDefault();
        onCursorChange((cursor + 1) % state.hits.length);
      } else if (e.key === "F3" && e.shiftKey) {
        e.preventDefault();
        onCursorChange((cursor - 1 + state.hits.length) % state.hits.length);
      } else if (e.key === "Escape" && document.activeElement === inputRef.current) {
        setNeedle("");
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cursor, state.hits.length, onCursorChange]);

  const counter = state.scanning
    ? `${cursor + (state.hits.length > 0 ? 1 : 0)}/${state.totalSoFar}+`
    : state.hits.length > 0
      ? `${cursor + 1}/${state.hits.length}`
      : state.totalSoFar > 0
        ? "(no results)"
        : "";

  return (
    <div className="flex items-center gap-1">
      <div className="relative">
        <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={inputRef}
          value={needle}
          onChange={(e) => setNeedle(e.target.value)}
          placeholder="Search…"
          title={state.queryError ?? undefined}
          className={`h-8 w-56 pl-7 font-mono text-xs ${state.queryError ? "border-destructive focus-visible:ring-destructive" : ""}`}
        />
        {needle && (
          <button
            onClick={() => setNeedle("")}
            title="Clear (Esc)"
            className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>
      <Button
        size="sm"
        variant={caseSensitive ? "default" : "outline"}
        onClick={() => setCaseSensitive((b) => !b)}
        title="Case sensitive"
        className="h-8 w-8 p-0 font-mono text-xs"
      >
        Aa
      </Button>
      <Button
        size="sm"
        variant={mode === "regex" ? "default" : "outline"}
        onClick={() => setMode((m) => (m === "regex" ? "substring" : "regex"))}
        title="Regex (toggle)"
        className="h-8 w-8 p-0 font-mono text-xs"
      >
        .*
      </Button>
      <select
        value={scope}
        onChange={(e) => setScope(e.target.value as SearchQuery["scope"])}
        className="h-8 rounded-md border border-input bg-background px-1 text-xs"
      >
        <option value="both">both</option>
        <option value="keys">keys</option>
        <option value="values">values</option>
      </select>
      {scopePath && (
        <span
          onClick={onClearScope}
          title="Click to clear scope"
          className="inline-flex cursor-pointer items-center gap-1 rounded-md border border-blue-300 bg-blue-50 px-2 py-0.5 text-[11px] text-blue-700 hover:bg-blue-100 dark:border-blue-700 dark:bg-blue-950/40 dark:text-blue-300 dark:hover:bg-blue-950/60"
        >
          scope: {scopePath} <X className="h-3 w-3" />
        </span>
      )}
      <span className="min-w-[60px] text-xs text-muted-foreground">{counter}</span>
      {state.hits.length > 0 && (
        <>
          <Button
            size="icon"
            variant="ghost"
            className="h-7 w-7"
            onClick={() => onCursorChange((cursor - 1 + state.hits.length) % state.hits.length)}
            title="Previous (Shift+F3)"
          >
            <ChevronUp className="h-3.5 w-3.5" />
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-7 w-7"
            onClick={() => onCursorChange((cursor + 1) % state.hits.length)}
            title="Next (F3)"
          >
            <ChevronDown className="h-3.5 w-3.5" />
          </Button>
        </>
      )}
    </div>
  );
}
