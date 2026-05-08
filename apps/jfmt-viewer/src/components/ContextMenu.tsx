import { useEffect } from "react";

export interface ContextMenuItem {
  label: string;
  onClick: () => void;
}

interface Props {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onDismiss: () => void;
}

export function ContextMenu({ x, y, items, onDismiss }: Props) {
  useEffect(() => {
    function onClickOutside() {
      onDismiss();
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") onDismiss();
    }
    window.addEventListener("click", onClickOutside, { once: true });
    window.addEventListener("keydown", onEsc);
    return () => {
      window.removeEventListener("keydown", onEsc);
    };
  }, [onDismiss]);

  return (
    <div
      role="menu"
      style={{
        position: "fixed",
        top: y,
        left: x,
        background: "white",
        border: "1px solid #888",
        boxShadow: "0 2px 6px rgba(0,0,0,0.2)",
        padding: 4,
        zIndex: 1000,
        fontFamily: "system-ui",
        fontSize: 13,
      }}
    >
      {items.map((it, i) => (
        <div
          key={i}
          role="menuitem"
          onClick={(e) => {
            e.stopPropagation();
            it.onClick();
            onDismiss();
          }}
          style={{
            padding: "4px 12px",
            cursor: "pointer",
          }}
          onMouseEnter={(e) => (e.currentTarget.style.background = "#eef")}
          onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
        >
          {it.label}
        </div>
      ))}
    </div>
  );
}
