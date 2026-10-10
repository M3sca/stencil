// Builds the addon and copies it to stencil.node (loaded via `require("./stencil.node")`).
import { execFileSync } from "node:child_process";
import { copyFileSync } from "node:fs";

const lib = { darwin: "libstencil.dylib", win32: "stencil.dll" }[process.platform] ?? "libstencil.so";
execFileSync("cargo", ["build", "--release", "--lib", "--features", "node"], { stdio: "inherit" });
copyFileSync(`target/release/${lib}`, "stencil.node");
