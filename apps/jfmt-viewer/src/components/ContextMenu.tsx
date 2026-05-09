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
      style={{ top: y, left: x }}
      className="fixed z-50 min-w-[180px] rounded-md border bg-card p-1 text-sm text-card-foreground shadow-md"
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
          className="cursor-pointer rounded-sm px-2 py-1.5 hover:bg-accent hover:text-accent-foreground"
        >
          {it.label}
        </div>
      ))}
    </div>
  );
}
