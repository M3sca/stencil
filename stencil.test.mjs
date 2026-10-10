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

test("include with named partials", () => {
  const partials = { row: "<{{ x }}>", list: "{% for x in xs %}{% include 'row' %}{% endfor %}" };
  assert.equal(render("{% include 'list' %}", { xs: [1, 2] }, partials), "<1><2>");
  assert.throws(() => render("{% include 'nope' %}", {}, partials), /Unknown template/);
  assert.throws(() => render("{% include '../x' %}", {}, partials), /Unknown template/);
});

test("include with arguments and output cap", () => {
  assert.equal(render("{% include 'c' with t = 'A' %}", {}, { c: "[{{ t }}]" }), "[A]");
  const big = Array.from({ length: 1000 }, (_, i) => i);
  assert.throws(() => render("{% for a in big %}{% for b in big %}{{ s }}{% endfor %}{% endfor %}", { big, s: "x".repeat(100) }), /Output too large/);
});

test("extends and blocks", () => {
  const partials = { layout: "<h1>{% block title %}Site{% endblock %}</h1>{% block body %}{% endblock %}" };
  const page = "{% extends 'layout' %}{% block body %}Hi {{ n }}{% endblock %}";
  assert.equal(render(page, { n: 1 }, partials), "<h1>Site</h1>Hi 1");
  assert.throws(() => render("{% extends 'x' %}", {}, partials), /Unknown template/);
  const big = Array.from({ length: 1000 }, (_, i) => i);
  assert.throws(() => render("{% for a in big %}{% for b in big %}{% for c in big %}{% endfor %}{% endfor %}{% endfor %}", { big }), /Too many loop iterations/);
});

test("forloop.parentloop in nested loops", () => {
  const t = "{% for a in xs %}{% for b in xs %}{{ forloop.parentloop.index }}.{{ forloop.index }} {% endfor %}{% endfor %}";
  assert.equal(render(t, { xs: [1, 2] }), "1.1 1.2 2.1 2.2 ");
});
