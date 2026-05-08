import { invoke, Channel } from "@tauri-apps/api/core";

export type NodeId = number;
export type Kind =
  | "object"
  | "array"
  | "string"
  | "number"
  | "bool"
  | "null"
  | "ndjson_doc";

export interface ChildSummary {
  id: NodeId | null;
  key: string;
  kind: Kind;
  child_count: number;
  preview: string | null;
}

export interface OpenFileResp {
  session_id: string;
  root_id: NodeId;
  format: "json" | "ndjson";
  total_bytes: number;
}

export type IndexProgress =
  | { phase: "scanning"; bytes_done: number; bytes_total: number }
  | { phase: "ready"; build_ms: number }
  | { phase: "error"; message: string };

export interface GetChildrenResp {
  items: ChildSummary[];
  total: number;
}

export type SearchMode = "substring" | "regex";

export interface SearchQuery {
  needle: string;
  mode: SearchMode;
  case_sensitive: boolean;
  scope: "both" | "keys" | "values";
  from_node?: NodeId;
}

export type SearchEvent =
  | { kind: "hit"; node: NodeId | null; path: string; matched_in: "key" | "value"; snippet: string }
  | { kind: "progress"; bytes_done: number; bytes_total: number; hits_so_far: number }
  | { kind: "done"; total_hits: number; elapsed_ms: number }
  | { kind: "cancelled" }
  | { kind: "error"; message: string };

export async function openFile(
  path: string,
  onProgress: (p: IndexProgress) => void,
): Promise<OpenFileResp> {
  const channel = new Channel<IndexProgress>();
  channel.onmessage = onProgress;
  return invoke<OpenFileResp>("open_file", { path, onProgress: channel });
}

export async function closeFile(sessionId: string): Promise<void> {
  await invoke("close_file", { sessionId });
}

export async function getChildren(
  sessionId: string,
  parent: NodeId,
  offset: number,
  limit: number,
): Promise<GetChildrenResp> {
  return invoke<GetChildrenResp>("get_children", {
    sessionId,
    parent,
    offset,
    limit,
  });
}

export async function getValue(
  sessionId: string,
  node: NodeId,
  maxBytes?: number,
): Promise<{ json: string; truncated: boolean }> {
  return invoke("get_value", { sessionId, node, maxBytes });
}

export async function getPointer(
  sessionId: string,
  node: NodeId,
): Promise<string> {
  const r = await invoke<{ pointer: string }>("get_pointer", { sessionId, node });
  return r.pointer;
}

export async function search(
  sessionId: string,
  query: SearchQuery,
  onEvent: (e: SearchEvent) => void,
): Promise<{ id: string }> {
  const channel = new Channel<SearchEvent>();
  channel.onmessage = onEvent;
  return invoke<{ id: string }>("search", { sessionId, query, onEvent: channel });
}

export async function cancelSearch(handle: string): Promise<void> {
  await invoke("cancel_search", { handle });
}

export interface ExportSubtreeResp {
  bytes_written: number;
  elapsed_ms: number;
}

export async function exportSubtree(
  sessionId: string,
  node: NodeId,
  targetPath: string,
  pretty: boolean,
): Promise<ExportSubtreeResp> {
  return invoke<ExportSubtreeResp>("export_subtree", {
    sessionId,
    node,
    targetPath,
    pretty,
  });
}
