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
  queryError: string | null;  // NEW
}

const HIT_CAP = 1000;

export function useSearch(sessionId: string | null) {
  const [state, setState] = useState<SearchState>({
    query: { needle: "", mode: "substring", case_sensitive: false, scope: "both" },
    hits: [],
    totalSoFar: 0,
    scanning: false,
    cancelled: false,
    error: null,
    hitCap: false,
    queryError: null,
  });
  const handleRef = useRef<string | null>(null);
  // Buffers updated synchronously per IPC event; flushed to React state on
  // an animation frame. Without this, a search that hits 100k+ nodes
  // triggers 100k setState calls and freezes the UI.
  const pendingHitsRef = useRef<Hit[]>([]);
  const committedHitsCountRef = useRef(0);
  const totalSoFarRef = useRef(0);
  const hitCapRef = useRef(false);
  const flushScheduledRef = useRef(false);
  // Bumped on every start/cancel so in-flight IPC events from a prior run
  // are silently dropped instead of mutating the new state.
  const generationRef = useRef(0);

  function scheduleFlush() {
    if (flushScheduledRef.current) return;
    flushScheduledRef.current = true;
    requestAnimationFrame(() => {
      flushScheduledRef.current = false;
      const incoming = pendingHitsRef.current;
      pendingHitsRef.current = [];
      committedHitsCountRef.current += incoming.length;
      const total = totalSoFarRef.current;
      const cap = hitCapRef.current;
      setState((prev) => {
        if (
          incoming.length === 0 &&
          prev.totalSoFar === total &&
          prev.hitCap === cap
        ) {
          return prev;
        }
        return {
          ...prev,
          hits: incoming.length > 0 ? [...prev.hits, ...incoming] : prev.hits,
          totalSoFar: total,
          hitCap: cap,
        };
      });
    });
  }

  async function start(query: SearchQuery) {
    if (!sessionId) return;
    if (handleRef.current) {
      await cancelSearch(handleRef.current);
      handleRef.current = null;
    }
    pendingHitsRef.current = [];
    committedHitsCountRef.current = 0;
    totalSoFarRef.current = 0;
    hitCapRef.current = false;
    const myGen = ++generationRef.current;
    setState({
      query,
      hits: [],
      totalSoFar: 0,
      scanning: true,
      cancelled: false,
      error: null,
      hitCap: false,
      queryError: null,
    });
    if (!query.needle.trim()) {
      setState((s) => ({ ...s, scanning: false }));
      return;
    }
    try {
      const handle = await search(sessionId, query, (e: SearchEvent) => {
        if (myGen !== generationRef.current) return; // stale run, ignore
        if (e.kind === "hit") {
          totalSoFarRef.current += 1;
          if (hitCapRef.current) {
            // Count only — never append past the cap.
          } else if (
            committedHitsCountRef.current + pendingHitsRef.current.length >=
            HIT_CAP
          ) {
            hitCapRef.current = true;
            // Once we have HIT_CAP hits we stop the backend so progress
            // stops ticking and the user sees a stable "1000+" total.
            if (handleRef.current) {
              const h = handleRef.current;
              handleRef.current = null;
              cancelSearch(h).catch(() => {});
            }
          } else {
            pendingHitsRef.current.push(e);
          }
          scheduleFlush();
          return;
        }
        if (e.kind === "progress") {
          totalSoFarRef.current = e.hits_so_far;
          scheduleFlush();
          return;
        }
        if (e.kind === "done") {
          // Force one final flush to settle any pending hits / totals.
          requestAnimationFrame(() => {
            const incoming = pendingHitsRef.current;
            pendingHitsRef.current = [];
            setState((prev) => ({
              ...prev,
              hits: incoming.length > 0 ? [...prev.hits, ...incoming] : prev.hits,
              totalSoFar: totalSoFarRef.current,
              hitCap: hitCapRef.current,
              scanning: false,
            }));
          });
          return;
        }
        if (e.kind === "cancelled") {
          setState((prev) => ({ ...prev, scanning: false, cancelled: true }));
          return;
        }
        if (e.kind === "error") {
          setState((prev) => ({ ...prev, scanning: false, error: e.message }));
          return;
        }
      });
      handleRef.current = handle.id;
    } catch (err: unknown) {
      const msg = (err && typeof err === "object" && "message" in err)
        ? String((err as { message: unknown }).message)
        : String(err);
      if (msg.toLowerCase().startsWith("invalid query")) {
        setState((s) => ({ ...s, scanning: false, queryError: msg }));
      } else {
        setState((s) => ({ ...s, scanning: false, error: msg }));
      }
    }
  }

  async function cancel() {
    // Bump generation so any in-flight IPC events from the cancelled run
    // are dropped on arrival.
    generationRef.current += 1;
    pendingHitsRef.current = [];
    committedHitsCountRef.current = 0;
    totalSoFarRef.current = 0;
    hitCapRef.current = false;
    if (handleRef.current) {
      await cancelSearch(handleRef.current);
      handleRef.current = null;
    }
    // Clear UI state immediately — user expects results to vanish.
    setState((prev) => ({
      ...prev,
      hits: [],
      totalSoFar: 0,
      hitCap: false,
      scanning: false,
      cancelled: true,
    }));
  }

  useEffect(() => {
    return () => {
      cancel();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { state, start, cancel };
}
