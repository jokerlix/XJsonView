import { ChildSummary } from "../api";

interface Props {
  child: ChildSummary;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
  onSelect: () => void;
}

export function TreeRow({ child, depth, expanded, onToggle, onSelect }: Props) {
  const isContainer = child.id !== null;
  const chevron = isContainer ? (expanded ? "▾" : "▸") : "•";
  const sizeHint = isContainer ? `[${child.child_count}]` : (child.preview ?? "");

  return (
    <div
      style={{
        height: 22,
        paddingLeft: depth * 16,
        cursor: "pointer",
        whiteSpace: "nowrap",
        fontFamily: "ui-monospace, monospace",
        fontSize: 13,
        userSelect: "none",
      }}
      onClick={onSelect}
    >
      <span
        style={{ width: 14, display: "inline-block" }}
        onClick={(e) => {
          if (isContainer) {
            e.stopPropagation();
            onToggle();
          }
        }}
      >
        {chevron}
      </span>
      <span style={{ color: "#888" }}>
        {" "}{child.kind === "ndjson_doc" ? "doc" : child.kind}
      </span>{" "}
      <strong>{child.key}</strong>{" "}
      <span style={{ color: "#444" }}>{sizeHint}</span>
    </div>
  );
}
