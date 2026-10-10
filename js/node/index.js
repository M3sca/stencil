"use strict";
const wasm = require("./stencil.js");

// `context` is an object, or a JSON string you already have (skips one JSON.stringify).
function render(template, context, partials, filters) {
  const json = typeof context === "string" ? context : JSON.stringify(context ?? {});
  return wasm.render(template, json, partials ?? undefined, filters ?? undefined);
}

exports.render = render;
