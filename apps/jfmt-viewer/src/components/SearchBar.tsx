import { useEffect, useRef, useState } from "react";
import { SearchQuery } from "../api";
import { SearchState } from "../lib/searchState";

interface Props {
  onQuery: (q: SearchQuery) => void;
  onCancel: () => void;
  state: SearchState;
  cursor: number;
  onCursorChange: (next: number) => void;
}

const DEBOUNCE_MS = 250;

export function SearchBar({ onQuery, onCancel, state, cursor, onCursorChange }: Props) {
  const [needle, setNeedle] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [scope, setScope] = useState<SearchQuery["scope"]>("both");
  const tRef = useRef<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (tRef.current !== null) clearTimeout(tRef.current);
    tRef.current = window.setTimeout(() => {
      if (needle.trim() === "") {
        onCancel();
      } else {
        onQuery({ needle, mode: "substring", case_sensitive: caseSensitive, scope });
      }
    }, DEBOUNCE_MS);
    return () => {
      if (tRef.current !== null) clearTimeout(tRef.current);
    };
  }, [needle, caseSensitive, scope, onQuery, onCancel]);

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
    <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      <input
        ref={inputRef}
        value={needle}
        onChange={(e) => setNeedle(e.target.value)}
        placeholder="🔍 search"
        style={{
          width: 200,
          padding: "2px 6px",
          fontFamily: "ui-monospace, monospace",
          fontSize: 12,
        }}
      />
      <button
        onClick={() => setCaseSensitive((b) => !b)}
        title="Case sensitive"
        style={{ fontWeight: caseSensitive ? "bold" : "normal" }}
      >
        Aa
      </button>
      <select
        value={scope}
        onChange={(e) => setScope(e.target.value as SearchQuery["scope"])}
      >
        <option value="both">both</option>
        <option value="keys">keys</option>
        <option value="values">values</option>
      </select>
      <span style={{ color: "#666", fontSize: 12, minWidth: 60 }}>
        {counter}
      </span>
      {state.hits.length > 0 && (
        <>
          <button
            onClick={() => onCursorChange((cursor - 1 + state.hits.length) % state.hits.length)}
            title="Previous (Shift+F3)"
          >
            ↑
          </button>
          <button
            onClick={() => onCursorChange((cursor + 1) % state.hits.length)}
            title="Next (F3)"
          >
            ↓
          </button>
        </>
      )}
      {needle && (
        <button onClick={() => setNeedle("")} title="Clear (Esc)">
          ✕
        </button>
      )}
    </span>
  );
}
