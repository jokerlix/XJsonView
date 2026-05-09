import React from "react";

/// Tokenize a chunk of JSON text and return colored spans. Handles
/// strings (with escape sequences), numbers, booleans, null, keys
/// (strings followed by ':'), and structural punctuation. Falls back
/// to plain text on parse oddities — never throws.
export function highlightJson(src: string): React.ReactNode {
  const out: React.ReactNode[] = [];
  let i = 0;
  let key = 0;
  const len = src.length;

  function push(node: React.ReactNode) {
    out.push(<React.Fragment key={key++}>{node}</React.Fragment>);
  }

  while (i < len) {
    const c = src[i];

    // Whitespace
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      let j = i;
      while (j < len && (src[j] === " " || src[j] === "\t" || src[j] === "\n" || src[j] === "\r")) j++;
      push(src.slice(i, j));
      i = j;
      continue;
    }

    // String — could be a key (followed by `:`) or a value.
    if (c === '"') {
      let j = i + 1;
      while (j < len) {
        if (src[j] === "\\" && j + 1 < len) { j += 2; continue; }
        if (src[j] === '"') { j++; break; }
        j++;
      }
      const text = src.slice(i, j);
      // Look ahead past whitespace for `:` to distinguish key vs value.
      let k = j;
      while (k < len && (src[k] === " " || src[k] === "\t" || src[k] === "\n" || src[k] === "\r")) k++;
      const isKey = src[k] === ":";
      push(<span className={isKey ? "text-sky-700 dark:text-sky-300" : "text-emerald-700 dark:text-emerald-300"}>{text}</span>);
      i = j;
      continue;
    }

    // Number
    if ((c >= "0" && c <= "9") || c === "-") {
      let j = i + 1;
      while (j < len && /[0-9eE+\-.]/.test(src[j])) j++;
      push(<span className="text-orange-700 dark:text-orange-300">{src.slice(i, j)}</span>);
      i = j;
      continue;
    }

    // true / false / null
    if (c === "t" && src.startsWith("true", i)) {
      push(<span className="text-blue-700 dark:text-blue-300">true</span>);
      i += 4;
      continue;
    }
    if (c === "f" && src.startsWith("false", i)) {
      push(<span className="text-blue-700 dark:text-blue-300">false</span>);
      i += 5;
      continue;
    }
    if (c === "n" && src.startsWith("null", i)) {
      push(<span className="text-rose-700 dark:text-rose-300">null</span>);
      i += 4;
      continue;
    }

    // Structural punctuation
    if (c === "{" || c === "}" || c === "[" || c === "]") {
      push(<span className="text-purple-600 dark:text-purple-400">{c}</span>);
      i++;
      continue;
    }
    if (c === "," || c === ":") {
      push(<span className="text-muted-foreground">{c}</span>);
      i++;
      continue;
    }

    // Anything else (truncation marker, malformed) — passthrough.
    push(c);
    i++;
  }

  return out;
}
