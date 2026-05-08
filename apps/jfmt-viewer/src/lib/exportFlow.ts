import { save } from "@tauri-apps/plugin-dialog";
import { exportSubtree, NodeId } from "../api";

export async function runExportFlow(
  sessionId: string,
  node: NodeId,
  defaultName = "subtree.json",
): Promise<string | null> {
  // E2E hook: if window.__TAURI_DIALOG_SAVE_PATH__ is set (only by tests),
  // bypass the native save dialog. Production runtime never sets this.
  const w = window as unknown as Record<string, unknown>;
  const overridePath = typeof w.__TAURI_DIALOG_SAVE_PATH__ === "string"
    ? (w.__TAURI_DIALOG_SAVE_PATH__ as string)
    : null;
  const path =
    overridePath ??
    (await save({
      defaultPath: defaultName,
      filters: [{ name: "JSON", extensions: ["json"] }],
    }));
  if (!path) return null;
  const r = await exportSubtree(sessionId, node, path, true);
  return `Exported ${r.bytes_written} bytes to ${path}`;
}
