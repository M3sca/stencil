// Builds the WebAssembly package into ./pkg: `pkg/node` (CommonJS) and `pkg/web` (ES modules).
// Needs the wasm32 target (`rustup target add wasm32-unknown-unknown`) and wasm-bindgen-cli.
import { execFileSync } from "node:child_process";
import { cpSync, readFileSync, rmSync, writeFileSync } from "node:fs";

const run = (cmd, args) => execFileSync(cmd, args, { stdio: "inherit" });

// wasm-bindgen-cli must be the same version as the wasm-bindgen crate.
const wanted = /name = "wasm-bindgen"\nversion = "([^"]+)"/.exec(readFileSync("Cargo.lock", "utf8"))?.[1];
let have = "";
try {
  have = execFileSync("wasm-bindgen", ["--version"]).toString().split(" ")[1].trim();
} catch {}
if (have !== wanted) {
  console.error(`wasm-bindgen ${wanted} is required (found ${have || "none"}).\nInstall it with: cargo install wasm-bindgen-cli --version ${wanted} --locked`);
  process.exit(1);
}

const version = /^version = "([^"]+)"/m.exec(readFileSync("Cargo.toml", "utf8"))[1];
run("cargo", ["build", "--release", "--lib", "--target", "wasm32-unknown-unknown", "--features", "wasm"]);

rmSync("pkg", { recursive: true, force: true });
for (const [target, dir] of [["nodejs", "node"], ["web", "web"]]) {
  run("wasm-bindgen", ["--target", target, "--out-dir", `pkg/${dir}`, "target/wasm32-unknown-unknown/release/stencil.wasm"]);
  cpSync(`js/${dir}`, `pkg/${dir}`, { recursive: true });
}
writeFileSync("pkg/package.json", readFileSync("js/package.json", "utf8").replace("__VERSION__", version));
cpSync("README.md", "pkg/README.md");
cpSync("LICENSE", "pkg/LICENSE");
