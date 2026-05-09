import { useEffect, useState } from "react";
import { ChildSummary, getValue, NodeId } from "../api";

interface Props {
  sessionId: string;
  node: NodeId | null;
  leaf?: ChildSummary | null;
  onExport?: (node: NodeId) => void;
}

export function Preview({ sessionId, node, leaf, onExport }: Props) {
  const [json, setJson] = useState<string>("");
  const [truncated, setTruncated] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (node === null) {
      setJson("");
      setTruncated(false);
      setErr(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setErr(null);
    getValue(sessionId, node)
      .then((r) => {
        if (cancelled) return;
        setJson(r.json);
        setTruncated(r.truncated);
        setLoading(false);
      })
      .catch((e) => {
        if (cancelled) return;
        setErr(String(e));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, node]);

  // Scalar leaf — render its inline preview directly; no IPC needed.
  if (node === null && leaf) {
    return (
      <div style={{ height: "100%", overflow: "auto" }}>
        <div style={{ padding: "8px 16px", color: "#666", fontSize: 12 }}>
          {leaf.kind} · <strong>{leaf.key}</strong>
        </div>
        <pre
          style={{
            margin: 0,
            padding: 16,
            fontFamily: "ui-monospace, monospace",
            fontSize: 12,
            whiteSpace: "pre-wrap",
          }}
        >
          {leaf.preview ?? ""}
        </pre>
      </div>
    );
  }

  if (node === null) {
    return (
      <div style={{ padding: 16, color: "#999", fontStyle: "italic" }}>
        Select a node in the tree to preview.
      </div>
    );
  }
  if (loading) {
    return <div style={{ padding: 16, color: "#666" }}>Loading…</div>;
  }
  if (err) {
    return (
      <div style={{ padding: 16, color: "#c00" }}>
        Error: {err}
      </div>
    );
  }
  return (
    <div style={{ height: "100%", overflow: "auto" }}>
      <pre
        style={{
          margin: 0,
          padding: 16,
          fontFamily: "ui-monospace, monospace",
          fontSize: 12,
          whiteSpace: "pre",
        }}
      >
        {json}
      </pre>
      {truncated && node !== null && (
        <button
          onClick={() => onExport?.(node)}
          style={{
            display: "block",
            margin: "8px 16px",
            padding: "4px 12px",
            background: "#fee",
            border: "1px solid #c66",
            cursor: "pointer",
            fontFamily: "system-ui",
            fontSize: 13,
          }}
        >
          Export full subtree →
        </button>
      )}
    </div>
  );
}
