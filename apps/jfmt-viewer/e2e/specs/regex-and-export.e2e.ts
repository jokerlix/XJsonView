import { browser, $, $$ } from "@wdio/globals";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURE = resolve(
  __dirname,
  "../../../../crates/jfmt-viewer-core/tests/fixtures/small.json",
);

describe("regex + export", () => {
  before(async () => {
    await browser.url(`tauri://localhost?file=${encodeURIComponent(FIXTURE)}`);
    await $("strong=users").waitForExist({ timeout: 10_000 });
  });

  it("regex search finds anchored value", async () => {
    await $("button[title='Regex (toggle)']").click();
    const input = await $("input[placeholder='🔍 search']");
    await input.click();
    await input.setValue("^Al");
    await browser.waitUntil(
      async () => {
        const matches = await $$("div*=Alice");
        return matches.length > 0;
      },
      { timeout: 5_000, timeoutMsg: "expected 'Alice' in hit list" },
    );
  });

  it("export_subtree writes the root subtree", async () => {
    const dir = mkdtempSync(join(tmpdir(), "jfmt-viewer-e2e-"));
    const out = join(dir, "root.json");

    await browser.execute((p: string) => {
      (window as unknown as Record<string, unknown>).__TAURI_DIALOG_SAVE_PATH__ = p;
    }, out);

    const users = await $("strong=users");
    await users.click({ button: "right" });
    const exportItem = await $("div*=Export subtree");
    await exportItem.click();

    await browser.waitUntil(
      async () => {
        const hints = await $$("span*=Exported");
        return hints.length > 0;
      },
      { timeout: 10_000, timeoutMsg: "expected export confirmation" },
    );

    const contents = readFileSync(out, "utf8");
    const parsed = JSON.parse(contents);
    if (!Array.isArray(parsed.users)) {
      throw new Error("expected exported users array");
    }
  });
});
