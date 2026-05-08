import { invoke } from "@tauri-apps/api/core";

export type NodeId = number;

export interface ChildSummary {
  id: NodeId | null;
  key: string;
  kind: "object" | "array" | "string" | "number" | "bool" | "null" | "ndjson_doc";
  child_count: number;
  preview: string | null;
}

export async function ping(): Promise<string> {
  return invoke<string>("ping");
}
