import { JSX, useEffect, useState } from "react";
import { ChildSummary, getChildren, NodeId } from "../api";
import { TreeRow } from "./TreeRow";

interface Props {
  sessionId: string;
  rootId: NodeId;
}

interface NodeState {
  loaded: ChildSummary[];
  total: number;
  expanded: boolean;
}

export function Tree({ sessionId, rootId }: Props) {
  const [byId, setById] = useState<Map<NodeId, NodeState>>(new Map());

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const r = await getChildren(sessionId, rootId, 0, 200);
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
    const r = await getChildren(sessionId, id, 0, 200);
    const next = new Map(byId);
    next.set(id, { loaded: r.items, total: r.total, expanded: true });
    setById(next);
  }

  function render(id: NodeId, depth: number): JSX.Element[] {
    const state = byId.get(id);
    if (!state) return [];
    const out: JSX.Element[] = [];
    for (const c of state.loaded) {
      out.push(
        <TreeRow
          key={`${id}-${c.key}-${c.id ?? "leaf"}`}
          child={c}
          depth={depth}
          expanded={c.id !== null && (byId.get(c.id)?.expanded ?? false)}
          onToggle={() => c.id !== null && toggle(c.id)}
        />,
      );
      if (c.id !== null && byId.get(c.id)?.expanded) {
        out.push(...render(c.id, depth + 1));
      }
    }
    if (state.total > state.loaded.length) {
      out.push(
        <div key={`${id}-more`} style={{ paddingLeft: depth * 16, color: "#888" }}>
          (+{state.total - state.loaded.length} more — virtual scroll lands in M8.2)
        </div>,
      );
    }
    return out;
  }

  return <div>{render(rootId, 0)}</div>;
}
