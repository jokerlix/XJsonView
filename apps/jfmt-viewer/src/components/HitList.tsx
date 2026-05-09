import React, { useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Hit, SearchState } from "../lib/searchState";
import { childForSegment, getValue, NodeId } from "../api";
import { cn } from "@/lib/utils";
import { highlightJson } from "@/lib/jsonHighlight";

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
  const hoverCloseTimerRef = useRef<number | null>(null);
  const hoverGenRef = useRef(0);

  function clearOpenTimer() {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }
  function clearCloseTimer() {
    if (hoverCloseTimerRef.current !== null) {
      clearTimeout(hoverCloseTimerRef.current);
      hoverCloseTimerRef.current = null;
    }
  }

  /// Schedule popover close with a small grace period so the user can
  /// move the mouse from the row into the popover without it vanishing.
  function scheduleHide() {
    clearOpenTimer();
    clearCloseTimer();
    hoverCloseTimerRef.current = window.setTimeout(() => {
      hoverGenRef.current += 1;
      setHover(null);
    }, 150);
  }

  function keepOpen() {
    clearCloseTimer();
  }

  async function startHover(hit: Hit, hitIdx: number, x: number, y: number) {
    clearOpenTimer();
    clearCloseTimer();
    const myGen = ++hoverGenRef.current;
    hoverTimerRef.current = window.setTimeout(async () => {
      const segs = hit.path
        .split("/")
        .slice(1)
        .map((s) => s.replace(/~1/g, "/").replace(/~0/g, "~"));
      let cur: NodeId = rootId;
      let okSoFar = true;
      for (let i = 0; i < segs.length; i++) {
        const childId = await childForSegment(sessionId, cur, segs[i]);
        if (myGen !== hoverGenRef.current) return;
        if (childId === null) {
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
    setHover({ hitIdx, x, y, json: null, err: null });
  }

  if (state.hits.length === 0 && !state.scanning) return null;

  return (
    <div className="flex w-[260px] flex-col overflow-hidden border-r bg-muted/30">
      <div className="flex items-center gap-1 border-b px-3 py-2 text-[11px] text-muted-foreground">
        {state.scanning ? (
          <>
            <Spinner /> Scanning · {state.totalSoFar.toLocaleString()} hits
          </>
        ) : (
          <>
            <span className="font-medium text-foreground">
              {state.hits.length.toLocaleString()}
            </span>{" "}
            hits
            {state.hitCap && (
              <span className="ml-1 rounded bg-amber-100 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:bg-amber-900/40 dark:text-amber-400">
                1000+ — refine
              </span>
            )}
          </>
        )}
        {state.error && (
          <span className="ml-1 text-destructive">· {state.error}</span>
        )}
      </div>
      <div ref={containerRef} className="flex-1 overflow-auto">
        <div style={{ height: v.getTotalSize() }} className="relative">
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
              onMouseLeave={scheduleHide}
            />
          ))}
        </div>
      </div>
      {hover && (
        <HoverPopover hover={hover} onMouseEnter={keepOpen} onMouseLeave={scheduleHide} />
      )}
    </div>
  );
}

function Spinner() {
  return (
    <span className="inline-block h-2.5 w-2.5 animate-spin rounded-full border border-muted-foreground border-t-transparent" />
  );
}

function HoverPopover({
  hover,
  onMouseEnter,
  onMouseLeave,
}: {
  hover: { x: number; y: number; json: string | null; err: string | null };
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}) {
  return (
    <div
      style={{ left: hover.x, top: hover.y }}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      className="fixed z-50 max-h-[420px] max-w-[640px] overflow-auto whitespace-pre rounded-md border bg-card p-2 font-mono text-[11px] text-card-foreground shadow-lg"
    >
      {hover.err ? (
        <span className="text-destructive">{hover.err}</span>
      ) : hover.json === null ? (
        <span className="text-muted-foreground">Loading…</span>
      ) : (
        highlightJson(hover.json)
      )}
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
      style={{ transform: `translateY(${top}px)` }}
      className={cn(
        "absolute left-0 right-0 top-0 flex h-[22px] cursor-pointer items-center gap-1 overflow-hidden whitespace-nowrap px-2 font-mono text-[11px]",
        selected ? "bg-primary/10 text-foreground" : "hover:bg-accent/50",
      )}
    >
      <span
        className={cn(
          "inline-block min-w-[12px] text-center font-bold",
          hit.matched_in === "key" ? "text-blue-600 dark:text-blue-400" : "text-amber-600 dark:text-amber-400",
        )}
      >
        {hit.matched_in === "key" ? "K" : "V"}
      </span>
      <span className="truncate text-foreground/70">{hit.path}</span>
      <span
        className="truncate text-muted-foreground"
        dangerouslySetInnerHTML={{ __html: renderSnippet(hit.snippet) }}
      />
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
