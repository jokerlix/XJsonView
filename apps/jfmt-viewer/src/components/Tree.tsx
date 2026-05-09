import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChildSummary, childForSegment, getChildren, NodeId } from "../api";
import { TreeRow } from "./TreeRow";

interface Props {
  sessionId: string;
  rootId: NodeId;
  onSelect?: (node: NodeId | null, leaf?: ChildSummary) => void;
  selectedId?: NodeId | null;
  onContextMenu?: (node: NodeId | null, x: number, y: number) => void;
}

export interface ExpandResult {
  node: NodeId;
  leaf?: ChildSummary;
}

export interface TreeHandle {
  expandToPointer(pointer: string): Promise<ExpandResult | null>;
}

/// Sparse loaded data per parent. `loaded.get(i)` is the i-th direct
/// child if its window has been fetched; otherwise undefined (rendered
/// as a placeholder). Sparse to allow jump-to-row 750000 without first
/// fetching rows 0..749999.
interface NodeState {
  loaded: Map<number, ChildSummary>;
  total: number;
  expanded: boolean;
}

/// A virtualized row. `kind: "real"` means the underlying ChildSummary is
/// loaded; `kind: "placeholder"` means the row exists logically but its
/// data hasn't been fetched — TreeRow renders a stub and the auto-page
/// effect will fill in the surrounding window.
type FlatRow =
  | { kind: "real"; child: ChildSummary; depth: number; parentId: NodeId; idx: number }
  | { kind: "placeholder"; depth: number; parentId: NodeId; idx: number };

const PAGE_LIMIT = 200;
const BATCH_LIMIT = 2000;
const ROW_HEIGHT = 22;

function parseSegIndex(seg: string): number | null {
  if (!/^\d+$/.test(seg)) return null;
  const n = Number(seg);
  return Number.isFinite(n) ? n : null;
}

function mergeLoaded(
  existing: Map<number, ChildSummary>,
  offset: number,
  items: ChildSummary[],
): Map<number, ChildSummary> {
  const next = new Map(existing);
  for (let i = 0; i < items.length; i++) {
    next.set(offset + i, items[i]);
  }
  return next;
}

function findByKey(loaded: Map<number, ChildSummary>, key: string): ChildSummary | undefined {
  for (const c of loaded.values()) {
    if (c.key === key) return c;
  }
  return undefined;
}

function findIdxByKey(loaded: Map<number, ChildSummary>, key: string): number {
  for (const [idx, c] of loaded) {
    if (c.key === key) return idx;
  }
  return -1;
}

/// Total rendered row count of `id`'s expanded subtree (excluding `id`'s
/// own row). Equals `total` plus any descendants of expanded children.
function subtreeSize(id: NodeId, byId: Map<NodeId, NodeState>): number {
  const state = byId.get(id);
  if (!state || !state.expanded) return 0;
  let size = state.total;
  for (const c of state.loaded.values()) {
    if (c.id !== null && byId.get(c.id)?.expanded) {
      size += subtreeSize(c.id, byId);
    }
  }
  return size;
}

/// Compute the flat row index of a target whose path through the tree
/// is `path`: each step gives the parent NodeId and the integer index
/// of the child within that parent. No materialization needed for
/// preceding siblings — we count using `total` and only recurse into
/// expanded ones.
function rowIdxOf(
  path: Array<{ parent: NodeId; childIdx: number }>,
  byId: Map<NodeId, NodeState>,
): number {
  let row = -1;
  for (const { parent, childIdx } of path) {
    const state = byId.get(parent);
    if (!state) return -1;
    for (let i = 0; i < childIdx; i++) {
      row += 1;
      const sibling = state.loaded.get(i);
      if (sibling && sibling.id !== null && byId.get(sibling.id)?.expanded) {
        row += subtreeSize(sibling.id, byId);
      }
    }
    row += 1;
  }
  return row;
}

export const Tree = forwardRef<TreeHandle, Props>(function Tree(
  { sessionId, rootId, onSelect, selectedId, onContextMenu },
  ref,
) {
  const [byId, setById] = useState<Map<NodeId, NodeState>>(new Map());
  const containerRef = useRef<HTMLDivElement>(null);
  // Per-parent in-flight guard: prevents the auto-page useEffect from
  // launching dozens of concurrent loads while the first one is still
  // running.
  const loadingRef = useRef<Set<NodeId>>(new Set());

  useImperativeHandle(ref, () => ({
    async expandToPointer(pointer: string): Promise<ExpandResult | null> {
      if (pointer === "") return { node: rootId };
      const segs = pointer
        .split("/")
        .slice(1)
        .map((s) => s.replace(/~1/g, "/").replace(/~0/g, "~"));
      let cur: NodeId = rootId;
      let workingMap = byId;
      let result: ExpandResult | null = null;
      // Track (parent, childIdx) for each step so we can compute the
      // flat row index after the loop without materializing siblings.
      const path: Array<{ parent: NodeId; childIdx: number }> = [];

      for (let i = 0; i < segs.length; i++) {
        const seg = segs[i];
        const isLast = i === segs.length - 1;
        const childId = await childForSegment(sessionId, cur, seg);

        let state = workingMap.get(cur);
        if (!state) {
          const r = await getChildren(sessionId, cur, 0, PAGE_LIMIT);
          state = {
            loaded: mergeLoaded(new Map(), 0, r.items),
            total: r.total,
            expanded: true,
          };
          workingMap = new Map(workingMap);
          workingMap.set(cur, state);
        } else if (!state.expanded) {
          state = { ...state, expanded: true };
          workingMap = new Map(workingMap);
          workingMap.set(cur, state);
        }

        // For the row-idx math: arrays use the segment as integer index;
        // objects need a key→index lookup which only works if the key is
        // in the loaded window. Fetch the surrounding window if missing.
        let childIdx = parseSegIndex(seg);
        if (childIdx === null) {
          childIdx = findIdxByKey(state.loaded, seg);
        }
        // Fetch the leaf's window so it appears as a real row, not a
        // placeholder, after the scroll lands.
        if (childIdx !== null && childIdx >= 0 && !state.loaded.has(childIdx)) {
          const off = Math.max(0, childIdx - 50);
          const r = await getChildren(sessionId, cur, off, 200);
          state = {
            ...state,
            loaded: mergeLoaded(state.loaded, off, r.items),
          };
          workingMap = new Map(workingMap);
          workingMap.set(cur, state);
        }
        if (childIdx !== null && childIdx >= 0) {
          path.push({ parent: cur, childIdx });
        }

        if (childId === null) {
          if (!isLast) {
            setById(workingMap);
            return null;
          }
          let leaf = findByKey(state.loaded, seg);
          if (!leaf) {
            const segIdx = parseSegIndex(seg);
            if (segIdx !== null && segIdx < state.total) {
              const r = await getChildren(sessionId, cur, segIdx, 1);
              leaf = r.items.find((c) => c.key === seg);
            }
          }
          result = leaf ? { node: cur, leaf } : { node: cur };
          break;
        }
        cur = childId;
        if (isLast) {
          result = { node: cur };
        }
      }

      setById(workingMap);

      // Scroll the virtualizer to the target row. Use rAF to wait for the
      // post-setState render so the new flat-row count is in effect.
      if (path.length > 0) {
        const targetRow = rowIdxOf(path, workingMap);
        if (targetRow >= 0) {
          requestAnimationFrame(() => {
            virtualizerRef.current?.scrollToIndex(targetRow, { align: "center" });
          });
        }
      }

      if (result) {
        if (result.leaf) {
          onSelect?.(null, result.leaf);
        } else {
          onSelect?.(result.node);
        }
      }
      return result;
    },
  }));

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const r = await getChildren(sessionId, rootId, 0, PAGE_LIMIT);
      if (cancelled) return;
      const m = new Map<NodeId, NodeState>();
      m.set(rootId, {
        loaded: mergeLoaded(new Map(), 0, r.items),
        total: r.total,
        expanded: true,
      });
      setById(m);
    })();
    return () => {
      cancelled = true;
    };
  }, [sessionId, rootId]);

  async function toggle(id: NodeId) {
    const cur = byId.get(id);
    if (cur) {
      const next = new Map(byId);
      next.set(id, { ...cur, expanded: !cur.expanded });
      setById(next);
      return;
    }
    const r = await getChildren(sessionId, id, 0, PAGE_LIMIT);
    const next = new Map(byId);
    next.set(id, {
      loaded: mergeLoaded(new Map(), 0, r.items),
      total: r.total,
      expanded: true,
    });
    setById(next);
  }

  /// Fetch a window of size BATCH_LIMIT centered on `targetIdx` if those
  /// rows aren't already loaded. Coalesces concurrent requests per
  /// parent via `loadingRef`.
  async function loadWindow(parentId: NodeId, targetIdx: number) {
    if (loadingRef.current.has(parentId)) return;
    const cur = byId.get(parentId);
    if (!cur) return;
    if (targetIdx < 0 || targetIdx >= cur.total) return;
    if (cur.loaded.has(targetIdx)) return;
    loadingRef.current.add(parentId);
    try {
      const off = Math.max(0, targetIdx - Math.floor(BATCH_LIMIT / 4));
      const limit = Math.min(BATCH_LIMIT, cur.total - off);
      const r = await getChildren(sessionId, parentId, off, limit);
      const next = new Map(byId);
      next.set(parentId, {
        ...cur,
        loaded: mergeLoaded(cur.loaded, off, r.items),
      });
      setById(next);
    } finally {
      loadingRef.current.delete(parentId);
    }
  }

  /// Build a virtual flat-row list that includes placeholder rows for
  /// unloaded children. Total size of each container == `total`, so the
  /// virtualizer reports the correct full extent and `scrollToIndex` to
  /// any in-range index works without materializing all siblings.
  const rows: FlatRow[] = [];
  function flatten(id: NodeId, depth: number) {
    const state = byId.get(id);
    if (!state) return;
    for (let i = 0; i < state.total; i++) {
      const c = state.loaded.get(i);
      if (c) {
        rows.push({ kind: "real", child: c, depth, parentId: id, idx: i });
        if (c.id !== null && byId.get(c.id)?.expanded) {
          flatten(c.id, depth + 1);
        }
      } else {
        rows.push({ kind: "placeholder", depth, parentId: id, idx: i });
      }
    }
  }
  flatten(rootId, 0);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });
  // Stash the latest virtualizer in a ref so the imperative handle can
  // call scrollToIndex from within an async closure.
  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;

  // Whenever visible rows include placeholders, fetch a window covering
  // them. Triggers per render so scroll-driven and jump-driven cases
  // both auto-fill.
  useEffect(() => {
    const visible = virtualizer.getVirtualItems();
    if (visible.length === 0) return;
    // Find the deepest placeholder visible per parent and request a load.
    const wanted = new Map<NodeId, number>();
    for (const vi of visible) {
      const r = rows[vi.index];
      if (r && r.kind === "placeholder") {
        const cur = wanted.get(r.parentId) ?? -1;
        if (r.idx > cur) wanted.set(r.parentId, r.idx);
      }
    }
    for (const [parentId, idx] of wanted) {
      loadWindow(parentId, idx);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  });

  return (
    <div
      ref={containerRef}
      style={{
        height: "100%",
        overflow: "auto",
        contain: "strict",
      }}
    >
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((vi) => {
          const row = rows[vi.index];
          if (!row) return null;
          if (row.kind === "placeholder") {
            return (
              <div
                key={vi.key}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${vi.start}px)`,
                  height: ROW_HEIGHT,
                  paddingLeft: row.depth * 16,
                  fontFamily: "ui-monospace, monospace",
                  fontSize: 13,
                  color: "#bbb",
                  fontStyle: "italic",
                }}
              >
                … row {row.idx}
              </div>
            );
          }
          const expanded =
            row.child.id !== null && (byId.get(row.child.id)?.expanded ?? false);
          const selected = selectedId !== undefined && row.child.id === selectedId;
          return (
            <div
              key={vi.key}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${vi.start}px)`,
                background: selected ? "#cce4ff" : "transparent",
              }}
            >
              <TreeRow
                child={row.child}
                depth={row.depth}
                expanded={expanded}
                onToggle={() => row.child.id !== null && toggle(row.child.id)}
                onSelect={() =>
                  onSelect?.(row.child.id, row.child.id === null ? row.child : undefined)
                }
                onContextMenu={(e) => onContextMenu?.(row.child.id, e.clientX, e.clientY)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
});
