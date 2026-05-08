import { ChildSummary } from "../api";

interface Props {
  child: ChildSummary;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
}

export function TreeRow({ child, depth, expanded, onToggle }: Props) {
  const isContainer = child.id !== null;
  const chevron = isContainer ? (expanded ? "▾" : "▸") : "•";
  const sizeHint = isContainer ? `[${child.child_count}]` : (child.preview ?? "");
  return (
    <div
      style={{
        paddingLeft: depth * 16,
        cursor: isContainer ? "pointer" : "default",
        whiteSpace: "nowrap",
        fontFamily: "ui-monospace, monospace",
        fontSize: 13,
      }}
      onClick={isContainer ? onToggle : undefined}
    >
      <span style={{ width: 14, display: "inline-block" }}>{chevron}</span>
      <span style={{ color: "#888" }}> {child.kind}</span>{" "}
      <strong>{child.key}</strong>{" "}
      <span style={{ color: "#444" }}>{sizeHint}</span>
    </div>
  );
}
