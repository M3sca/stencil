import init, { initSync, render as renderWasm } from "./stencil.js";

export { init, initSync };

// Call `await init()` (or `initSync`) once before `render`.
export function render(template, context, partials, filters) {
  const json = typeof context === "string" ? context : JSON.stringify(context ?? {});
  return renderWasm(template, json, partials ?? undefined, filters ?? undefined);
}
