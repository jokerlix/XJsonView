import { useEffect, useState } from "react";
import { ChildSummary, getValue, NodeId } from "../api";
import { Button } from "@/components/ui/button";
import { ArrowDownToLine } from "lucide-react";
import { highlightJson } from "@/lib/jsonHighlight";

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

  if (node === null && leaf) {
    return (
      <div className="flex h-full flex-col">
        <div className="border-b px-4 py-2 text-xs text-muted-foreground">
          <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wide">
            {leaf.kind}
          </span>{" "}
          <span className="font-medium text-foreground">{leaf.key}</span>
        </div>
        <pre className="flex-1 overflow-auto whitespace-pre-wrap break-all p-4 font-mono text-xs">
          {leaf.preview ?? ""}
        </pre>
      </div>
    );
  }

  if (node === null) {
    return (
      <div className="p-4 text-sm italic text-muted-foreground">
        Select a node in the tree to preview.
      </div>
    );
  }
  if (loading) {
    return <div className="p-4 text-sm text-muted-foreground">Loading…</div>;
  }
  if (err) {
    return <div className="p-4 text-sm text-destructive">Error: {err}</div>;
  }
  return (
    <div className="flex h-full flex-col overflow-hidden">
      <pre className="flex-1 overflow-auto whitespace-pre p-4 font-mono text-xs leading-5">
        {/* Highlighting tokenizes into per-token spans; for very large
            payloads (root of a huge file) that's hundreds of thousands
            of React nodes — fall back to plain text to keep render
            snappy. 100 KB caps at ~10k tokens which renders comfortably. */}
        {json.length > 100_000 ? json : highlightJson(json)}
      </pre>
      {truncated && node !== null && (
        <div className="border-t bg-amber-50/60 px-4 py-2 dark:bg-amber-950/20">
          <Button size="sm" variant="outline" onClick={() => onExport?.(node)}>
            <ArrowDownToLine className="h-3.5 w-3.5" /> Export full subtree
          </Button>
        </div>
      )}
    </div>
  );
}
