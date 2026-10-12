import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const { render } = createRequire(import.meta.url)("./pkg/node/index.js");

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

test("new syntax and filters from Node", () => {
  const ctx = { people: [{ name: "bob", age: 30 }, { name: "amy", age: 25 }] };
  assert.equal(render("{% for p in people | sort: 'age' %}{{ p.name | capitalize }}{% unless forloop.last %}, {% endunless %}{% endfor %}", ctx), "Amy, Bob");
  assert.equal(render("{% assign n = people | size %}{% case n %}{% when 2 %}two{% else %}other{% endcase %}", ctx), "two");
  assert.equal(render("{{ 1700000000 | date: '%F' }} {{ 7 | divided_by: 2 }}"), "2023-11-14 3.5");
  assert.throws(() => render("{{ 1 | divided_by: 0 }}"), /Division by zero/);
});

test("context can be a ready JSON string", () => {
  assert.equal(render("Hi {{ n }}", '{"n":"Ann"}'), "Hi Ann");
  assert.equal(render("Hi {{ n }}", null), "Hi ");
  assert.throws(() => render("x", "{bad json"), /Invalid context JSON/);
});

test("custom filters", () => {
  const money = (v, currency = "EUR") => new Intl.NumberFormat("it-IT", { style: "currency", currency }).format(v).replace(/\u00a0/g, " ");
  const filters = {
    money,
    pair: (v) => [v, v * 2],
    shout: (v, a, b, c, d, e) => String(v).toUpperCase() + [a, b, c, d, e].filter((x) => x !== undefined).length,
    nothing: () => undefined,
    uppercase: (v) => "mine:" + v, // takes precedence over the built-in
  };
  const r = (t, ctx = { n: 5 }) => render(t, ctx, undefined, filters);
  assert.equal(r("{{ n | money }}"), "5,00 €");
  assert.equal(r("{{ n | money: 'USD' }}"), "5,00 USD");
  assert.equal(r("{{ n | pair | join: '-' }}"), "5-10");
  assert.equal(r("{{ 'a' | shout: 1, 2, 3, 4, 5 }}"), "A5"); // any number of arguments
  assert.equal(r("[{{ n | nothing }}]"), "[]");
  assert.equal(r("{{ 'x' | uppercase }}"), "mine:x");
  assert.equal(render("{{ 'x' | uppercase }}"), "X");
  assert.equal(r("{% for p in ps %}{{ p.price | money }};{% endfor %}", { ps: [{ price: 1 }, { price: 2.5 }] }), "1,00 €;2,50 €;");
});

test("custom filters fail safely", () => {
  const err = (t, filters) => { try { render(t, {}, undefined, filters); } catch (e) { return e.message; } return "did not throw"; };
  assert.equal(err("a\n{{ 1 | boom }}", { boom: () => { throw new Error("nope"); } }), "Filter 'boom' failed: nope (line 2)");
  assert.match(err("{{ 1 | later }}", { later: async () => 1 }), /synchronous/);
  assert.match(err("{{ 1 | big }}", { big: () => "x".repeat(20 << 20) }), /too large/);
  assert.match(err("{{ 1 | x }}", { x: 5 }), /filters\.x must be a function/);
  assert.match(err("{{ 1 | x }}", "nope"), /must be an object/);
  // Templates can only reach the object's own properties, not what it inherits.
  assert.match(err("{{ 1 | toString }}", { a: () => 1 }), /Unknown filter: toString/);
  assert.match(err("{{ 1 | constructor }}", {}), /Unknown filter: constructor/);
  assert.match(err("{{ 1 | hasOwnProperty }}", { a: () => 1 }), /Unknown filter/);
  // A filter that returns something that isn't JSON is an error, not a crash.
  assert.match(err("{{ 1 | f }}", { f: () => 1n }), /./);
});

test("the web build works too", async () => {
  const web = await import("./pkg/web/index.js");
  const { readFileSync } = await import("node:fs");
  web.initSync({ module: readFileSync(new URL("./pkg/web/stencil_bg.wasm", import.meta.url)) });
  assert.equal(web.render("Hello {{ n | uppercase }}", { n: "x" }), "Hello X");
  assert.equal(web.render("{{ n | money }}", { n: 2 }, undefined, { money: (v) => `$${v}` }), "$2");
});

test("custom functions", () => {
  const functions = { add: (a, b) => a + b, year: () => 2026, list: (...xs) => xs, boom: () => { throw new Error("nope"); } };
  const r = (t, ctx = { n: 5 }) => render(t, ctx, undefined, undefined, functions);
  assert.equal(r("{{ add(n, 2) }}"), "7");
  assert.equal(r("{{ year() }}"), "2026");
  assert.equal(r("{{ add(add(1, 2), n) | plus: 1 }}"), "9");
  assert.equal(r("{{ list('a,b', 'c(d)') | join: '|' }}"), "a,b|c(d)");
  assert.equal(r("{{ add(n | plus: 1, 1) }}"), "7");
  assert.equal(r("{% if add(n, 1) == 6 %}y{% endif %}{% if not add(0, 0) %}n{% endif %}"), "y");
  assert.equal(r("{% assign x = add(1, 1) %}{{ x }}"), "2");
  assert.equal(r("{{ '\"add(1, 2)' }}"), "\"add(1, 2)");
  const err = (t) => { try { r(t); } catch (e) { return e.message; } return "did not throw"; };
  assert.equal(err("{{ nope(1) }}"), "Unknown function: nope (line 1)");
  assert.equal(err("{{ boom() }}"), "Function 'boom' failed: nope (line 1)");
  assert.match(err("{{ " + "add(".repeat(30) + "1" + ")".repeat(30) + " }}"), /nested too deeply/);
});
