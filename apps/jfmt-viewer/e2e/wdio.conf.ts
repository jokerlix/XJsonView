import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

let driver: ChildProcess | null = null;

export const config: WebdriverIO.Config = {
  hostname: "127.0.0.1",
  port: 4444,
  specs: ["./specs/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: [
    {
      "tauri:options": {
        application: resolve(__dirname, "../../../target/release/jfmt-viewer-app"),
      },
      browserName: "wry",
    } as WebdriverIO.Capabilities,
  ],
  reporters: ["spec"],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 60_000 },
  logLevel: "info",
  onPrepare() {
    spawnSync("cargo", ["build", "--release", "-p", "jfmt-viewer-app"], {
      stdio: "inherit",
    });
    driver = spawn("tauri-driver", [], { stdio: "inherit" });
  },
  onComplete() {
    driver?.kill();
  },
};
