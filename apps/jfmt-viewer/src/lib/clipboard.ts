import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { getPointer, NodeId } from "../api";

export async function copyPointer(sessionId: string, node: NodeId): Promise<string> {
  const pointer = await getPointer(sessionId, node);
  await writeText(pointer);
  return pointer;
}
