import { useEffect, useRef, useState } from "react";
import { cancelSearch, search, SearchEvent, SearchQuery } from "../api";

export interface Hit {
  node: number | null;
  path: string;
  matched_in: "key" | "value";
  snippet: string;
}

export interface SearchState {
  query: SearchQuery;
  hits: Hit[];
  totalSoFar: number;
  scanning: boolean;
  cancelled: boolean;
  error: string | null;
  hitCap: boolean;
}

const HIT_CAP = 1000;

export function useSearch(sessionId: string | null) {
  const [state, setState] = useState<SearchState>({
    query: { needle: "", case_sensitive: false, scope: "both" },
    hits: [],
    totalSoFar: 0,
    scanning: false,
    cancelled: false,
    error: null,
    hitCap: false,
  });
  const handleRef = useRef<string | null>(null);

  async function start(query: SearchQuery) {
    if (!sessionId) return;
    if (handleRef.current) {
      await cancelSearch(handleRef.current);
      handleRef.current = null;
    }
    setState({
      query,
      hits: [],
      totalSoFar: 0,
      scanning: true,
      cancelled: false,
      error: null,
      hitCap: false,
    });
    if (!query.needle.trim()) {
      setState((s) => ({ ...s, scanning: false }));
      return;
    }
    const handle = await search(sessionId, query, (e: SearchEvent) => {
      setState((prev) => {
        if (e.kind === "hit") {
          if (prev.hits.length >= HIT_CAP) {
            return { ...prev, totalSoFar: prev.totalSoFar + 1, hitCap: true };
          }
          return {
            ...prev,
            hits: [...prev.hits, e],
            totalSoFar: prev.totalSoFar + 1,
          };
        }
        if (e.kind === "progress") {
          return { ...prev, totalSoFar: e.hits_so_far };
        }
        if (e.kind === "done") {
          return { ...prev, scanning: false };
        }
        if (e.kind === "cancelled") {
          return { ...prev, scanning: false, cancelled: true };
        }
        if (e.kind === "error") {
          return { ...prev, scanning: false, error: e.message };
        }
        return prev;
      });
    });
    handleRef.current = handle.id;
  }

  async function cancel() {
    if (handleRef.current) {
      await cancelSearch(handleRef.current);
      handleRef.current = null;
    }
  }

  useEffect(() => {
    return () => {
      cancel();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { state, start, cancel };
}
