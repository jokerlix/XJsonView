import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChildSummary, getChildren, NodeId } from "../api";
import { TreeRow } from "./TreeRow";

interface Props {
  sessionId: string;
  rootId: NodeId;
  onSelect?: (node: NodeId | null) => void;
  selectedId?: NodeId | null;
  onContextMenu?: (node: NodeId | null, x: number, y: number) => void;
}

export interface TreeHandle {
  expandToPointer(pointer: string): Promise<NodeId | null>;
}

interface NodeState {
  loaded: ChildSummary[];
  total: number;
  expanded: boolean;
}

interface FlatRow {
  child: ChildSummary;
  depth: number;
  parentId: NodeId;
}

const PAGE_LIMIT = 200;
const ROW_HEIGHT = 22;

export const Tree = forwardRef<TreeHandle, Props>(function Tree(
  { sessionId, rootId, onSelect, selectedId, onContextMenu },
  ref,
) {
  const [byId, setById] = useState<Map<NodeId, NodeState>>(new Map());
  const containerRef = useRef<HTMLDivElement>(null);

  useImperativeHandle(ref, () => ({
    async expandToPointer(pointer: string): Promise<NodeId | null> {
      if (pointer === "") return rootId;
      const segs = pointer
        .split("/")
        .slice(1)
        .map((s) => s.replace(/~1/g, "/").replace(/~0/g, "~"));
      let cur: NodeId = rootId;
      let workingMap = byId;
      for (const seg of segs) {
        let state = workingMap.get(cur);
        if (!state) {
          const r = await getChildren(sessionId, cur, 0, PAGE_LIMIT);
          state = { loaded: r.items, total: r.total, expanded: true };
          workingMap = new Map(workingMap);
          workingMap.set(cur, state);
          setById(workingMap);
        } else if (!state.expanded) {
          state = { ...state, expanded: true };
          workingMap = new Map(workingMap);
          workingMap.set(cur, state);
          setById(workingMap);
        }
        const child = state.loaded.find((c) => c.key === seg);
        if (!child || child.id === null) return null;
        cur = child.id;
      }
      onSelect?.(cur);
      return cur;
    },
  }));

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const r = await getChildren(sessionId, rootId, 0, PAGE_LIMIT);
      if (cancelled) return;
      const m = new Map<NodeId, NodeState>();
      m.set(rootId, { loaded: r.items, total: r.total, expanded: true });
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
    next.set(id, { loaded: r.items, total: r.total, expanded: true });
    setById(next);
  }

  async function loadMore(id: NodeId) {
    const cur = byId.get(id);
    if (!cur || cur.loaded.length >= cur.total) return;
    const r = await getChildren(sessionId, id, cur.loaded.length, PAGE_LIMIT);
    const next = new Map(byId);
    next.set(id, {
      ...cur,
      loaded: [...cur.loaded, ...r.items],
    });
    setById(next);
  }

  // Flatten the tree to a linear list for virtualization.
  const rows: FlatRow[] = [];
  function flatten(id: NodeId, depth: number) {
    const state = byId.get(id);
    if (!state) return;
    for (const c of state.loaded) {
      rows.push({ child: c, depth, parentId: id });
      if (c.id !== null && byId.get(c.id)?.expanded) {
        flatten(c.id, depth + 1);
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

  // Auto-page additional children when scrolled near the bottom of any
  // partially-loaded container.
  useEffect(() => {
    for (const [id, state] of byId) {
      if (state.loaded.length < state.total) {
        let lastIdx = -1;
        for (let i = rows.length - 1; i >= 0; i--) {
          if (rows[i].parentId === id) { lastIdx = i; break; }
        }
        if (lastIdx === -1) continue;
        const visibleEnd = (virtualizer.getVirtualItems().at(-1)?.index ?? 0) + 1;
        if (visibleEnd >= lastIdx - 50) {
          loadMore(id);
        }
      }
    }
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
                onSelect={() => onSelect?.(row.child.id)}
                onContextMenu={(e) => onContextMenu?.(row.child.id, e.clientX, e.clientY)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
});
