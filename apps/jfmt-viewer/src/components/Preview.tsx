import { useEffect, useState } from "react";
import { getValue, NodeId } from "../api";

interface Props {
  sessionId: string;
  node: NodeId | null;
}

export function Preview({ sessionId, node }: Props) {
  const [json, setJson] = useState<string>("");
  const [truncated, setTruncated] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (node === null) {
      setJson("");
      setTruncated(false);
      setErr(null);
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
    <pre
      style={{
        margin: 0,
        padding: 16,
        fontFamily: "ui-monospace, monospace",
        fontSize: 12,
        whiteSpace: "pre",
        overflow: "auto",
        height: "100%",
      }}
    >
      {json}
      {truncated && (
        <span style={{ color: "#a60", fontStyle: "italic" }}>
          {"\n(see truncation marker above; full export ships in M9)"}
        </span>
      )}
    </pre>
  );
}
