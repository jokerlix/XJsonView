import { useEffect, useState } from "react";

type Theme = "light" | "dark";
const STORAGE_KEY = "jfmt-viewer-theme";

function applyTheme(t: Theme) {
  const root = document.documentElement;
  if (t === "dark") root.classList.add("dark");
  else root.classList.remove("dark");
}

function loadTheme(): Theme {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "dark" || v === "light") return v;
  } catch { /* ignore */ }
  return "light";
}

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(() => loadTheme());

  useEffect(() => {
    applyTheme(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch { /* ignore */ }
  }, [theme]);

  function toggle() {
    setThemeState((t) => (t === "dark" ? "light" : "dark"));
  }

  return { theme, toggle };
}
