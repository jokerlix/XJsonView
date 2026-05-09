import React, { useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Hit, SearchState } from "../lib/searchState";
import { childForSegment, getValue, NodeId } from "../api";

interface Props {
  state: SearchState;
  cursor: number;
  onPick: (idx: number) => void;
  sessionId: string;
  rootId: NodeId;
}

const HOVER_DELAY_MS = 250;
const HOVER_MAX_PREVIEW_BYTES = 8 * 1024;

export function HitList({ state, cursor, onPick, sessionId, rootId }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: state.hits.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => 22,
    overscan: 10,
  });

  const [hover, setHover] = useState<{
    hitIdx: number;
    x: number;
    y: number;
    json: string | null;
    err: string | null;
  } | null>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const hoverGenRef = useRef(0);

  function cancelHover() {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
    hoverGenRef.current += 1;
    setHover(null);
  }

  async function startHover(hit: Hit, hitIdx: number, x: number, y: number) {
    if (hoverTimerRef.current !== null) clearTimeout(hoverTimerRef.current);
    const myGen = ++hoverGenRef.current;
    hoverTimerRef.current = window.setTimeout(async () => {
      // Walk the JSON pointer; if the last seg is a scalar leaf we get
      // its parent's NodeId (showing the full record gives more context
      // than just the leaf value alone).
      const segs = hit.path
        .split("/")
        .slice(1)
        .map((s) => s.replace(/~1/g, "/").replace(/~0/g, "~"));
      let cur: NodeId = rootId;
      let okSoFar = true;
      for (let i = 0; i < segs.length; i++) {
        const childId = await childForSegment(sessionId, cur, segs[i]);
        if (myGen !== hoverGenRef.current) return; // hover cancelled
        if (childId === null) {
          // Leaf scalar — only valid as last segment; parent is `cur`.
          if (i !== segs.length - 1) okSoFar = false;
          break;
        }
        cur = childId;
      }
      if (!okSoFar) {
        setHover({ hitIdx, x, y, json: null, err: "(invalid path)" });
        return;
      }
      try {
        const r = await getValue(sessionId, cur, HOVER_MAX_PREVIEW_BYTES);
        if (myGen !== hoverGenRef.current) return;
        setHover({ hitIdx, x, y, json: r.json, err: null });
      } catch (e) {
        if (myGen !== hoverGenRef.current) return;
        setHover({ hitIdx, x, y, json: null, err: String(e) });
      }
    }, HOVER_DELAY_MS);
    setHover({ hitIdx, x, y, json: null, err: null }); // show "Loading…"
  }

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
              onMouseEnter={(e) =>
                startHover(
                  state.hits[vi.index],
                  vi.index,
                  e.currentTarget.getBoundingClientRect().right + 8,
                  e.currentTarget.getBoundingClientRect().top,
                )
              }
              onMouseLeave={cancelHover}
            />
          ))}
        </div>
      </div>
      {hover && <HoverPopover hover={hover} />}
    </div>
  );
}

function HoverPopover({
  hover,
}: {
  hover: { hitIdx: number; x: number; y: number; json: string | null; err: string | null };
}) {
  return (
    <div
      style={{
        position: "fixed",
        left: hover.x,
        top: hover.y,
        maxWidth: 520,
        maxHeight: 360,
        overflow: "auto",
        background: "#fff",
        border: "1px solid #aaa",
        boxShadow: "0 4px 16px rgba(0,0,0,0.12)",
        padding: 8,
        fontFamily: "ui-monospace, monospace",
        fontSize: 11,
        whiteSpace: "pre",
        zIndex: 1000,
        pointerEvents: "none",
      }}
    >
      {hover.err
        ? <span style={{ color: "#c00" }}>{hover.err}</span>
        : hover.json === null
          ? <span style={{ color: "#999" }}>Loading…</span>
          : hover.json}
    </div>
  );
}

function HitRow({
  hit,
  top,
  selected,
  onClick,
  onMouseEnter,
  onMouseLeave,
}: {
  hit: Hit;
  top: number;
  selected: boolean;
  onClick: () => void;
  onMouseEnter: (e: React.MouseEvent<HTMLDivElement>) => void;
  onMouseLeave: () => void;
}) {
  return (
    <div
      onClick={onClick}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
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
