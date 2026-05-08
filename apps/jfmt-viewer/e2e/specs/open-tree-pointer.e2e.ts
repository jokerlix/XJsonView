import { browser, $ } from "@wdio/globals";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURE = resolve(
  __dirname,
  "../../../../crates/jfmt-viewer-core/tests/fixtures/small.json",
);

describe("jfmt-viewer", () => {
  it("opens a file and shows the tree root", async () => {
    await browser.url(`tauri://localhost?file=${encodeURIComponent(FIXTURE)}`);
    const root = await $("strong=users");
    await root.waitForExist({ timeout: 10_000 });
    expect(await root.getText()).toBe("users");
  });

  it("copies a JSON Pointer", async () => {
    await $("strong=users").click();
    await $("strong=0").waitForExist();
    await $("strong=0").click();
    await $("strong=name").waitForExist();
    await $("strong=name").click();
    await $("button*=Copy ptr").click();
    const hint = await $("span=copied: /users/0/name");
    await hint.waitForExist({ timeout: 3_000 });
  });
});
