import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const { render } = createRequire(import.meta.url)("./index.js");

test("render from Node", () => {
  const ctx = { user: { name: "Andrea" }, xs: [1, 2, 3], n: 7 };
  assert.equal(render("Hello {{ user.name | uppercase }}!", ctx), "Hello ANDREA!");
  assert.equal(render("{% for x in xs %}{{ x }}{% if forloop.last %}.{% else %},{% endif %}{% endfor %}", ctx), "1,2,3.");
  assert.equal(render("{% if n > 5 and user.name == 'Andrea' %}ok{% endif %}", ctx), "ok");
  assert.equal(render("statico"), "statico");
});

test("errors become exceptions", () => {
  assert.throws(() => render("{% if x %}"), /Unclosed/);
  assert.throws(() => render("{{ a | nope }}", {}), /Unknown filter/);
  assert.throws(() => render("x", [1]), /object/);
});
