import React from "react";
import { ChildSummary } from "../api";
import { ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

interface Props {
  child: ChildSummary;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
  onSelect: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}

export function TreeRow({ child, depth, expanded, onToggle, onSelect, onContextMenu }: Props) {
  const isContainer = child.id !== null;
  const sizeHint = isContainer ? `[${child.child_count}]` : (child.preview ?? "");

  return (
    <div
      onClick={onSelect}
      onContextMenu={(e) => {
        if (onContextMenu) {
          e.preventDefault();
          onContextMenu(e);
        }
      }}
      style={{ paddingLeft: depth * 14 + 4 }}
      className="flex h-[22px] cursor-pointer select-none items-center gap-1 whitespace-nowrap font-mono text-[13px] hover:bg-accent/50"
    >
      <span
        onClick={(e) => {
          if (isContainer) {
            e.stopPropagation();
            onToggle();
          }
        }}
        className="inline-flex h-4 w-4 items-center justify-center text-muted-foreground"
      >
        {isContainer ? (
          expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />
        ) : (
          <span className="text-[8px]">●</span>
        )}
      </span>
      <span className={cn("text-[10px] uppercase", kindColor(child.kind))}>
        {child.kind === "ndjson_doc" ? "doc" : child.kind}
      </span>
      <span className="font-semibold text-foreground">{child.key}</span>
      <span className="truncate text-muted-foreground">{sizeHint}</span>
    </div>
  );
}

function kindColor(kind: ChildSummary["kind"]) {
  switch (kind) {
    case "object":
    case "array":
    case "ndjson_doc":
      return "text-purple-600 dark:text-purple-400";
    case "string":
      return "text-emerald-600 dark:text-emerald-400";
    case "number":
      return "text-orange-600 dark:text-orange-400";
    case "bool":
      return "text-blue-600 dark:text-blue-400";
    case "null":
      return "text-rose-600 dark:text-rose-400";
  }
}
