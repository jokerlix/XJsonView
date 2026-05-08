import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Hit, SearchState } from "../lib/searchState";

interface Props {
  state: SearchState;
  cursor: number;
  onPick: (idx: number) => void;
}

export function HitList({ state, cursor, onPick }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: state.hits.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => 22,
    overscan: 10,
  });

  if (state.hits.length === 0 && !state.scanning) return null;

  return (
    <div
      style={{
        flex: "0 0 240px",
        borderRight: "1px solid #ddd",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          padding: "4px 8px",
          fontSize: 11,
          color: "#666",
          borderBottom: "1px solid #eee",
        }}
      >
        {state.scanning
          ? `Scanning… ${state.totalSoFar} hits`
          : `${state.hits.length} hits${state.hitCap ? " (1000+ — refine query)" : ""}`}
        {state.error && <span style={{ color: "#c00" }}> · {state.error}</span>}
      </div>
      <div ref={containerRef} style={{ flex: 1, overflow: "auto" }}>
        <div style={{ height: v.getTotalSize(), position: "relative" }}>
          {v.getVirtualItems().map((vi) => (
            <HitRow
              key={vi.key}
              hit={state.hits[vi.index]}
              top={vi.start}
              selected={vi.index === cursor}
              onClick={() => onPick(vi.index)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function HitRow({
  hit,
  top,
  selected,
  onClick,
}: {
  hit: Hit;
  top: number;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <div
      onClick={onClick}
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        height: 22,
        transform: `translateY(${top}px)`,
        background: selected ? "#cce4ff" : "transparent",
        cursor: "pointer",
        padding: "2px 8px",
        fontFamily: "ui-monospace, monospace",
        fontSize: 11,
        whiteSpace: "nowrap",
        overflow: "hidden",
        textOverflow: "ellipsis",
      }}
    >
      <span style={{ color: hit.matched_in === "key" ? "#06a" : "#a60" }}>
        {hit.matched_in === "key" ? "K" : "V"}{" "}
      </span>
      <span style={{ color: "#444" }}>{hit.path}</span>{" "}
      <span style={{ color: "#888" }} dangerouslySetInnerHTML={{ __html: renderSnippet(hit.snippet) }} />
    </div>
  );
}

function renderSnippet(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
}
