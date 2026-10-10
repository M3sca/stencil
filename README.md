# stencil

A minimal template engine in Rust, with Node.js bindings. No dependencies in the core crate.

## Usage

```rust
use stencil::{Context, Value, render};

let ctx = Context::from([
    ("name".to_string(), "Andrea".into()),
    ("projects".to_string(), Value::List(vec!["stencil".into(), "other".into()])),
]);

let out = render(
    "Hello {{ name | uppercase }}!{% for p in projects %} {{ p }}{% endfor %}",
    &ctx,
)?;
// "Hello ANDREA! stencil other"
```

`render(template, &ctx)` returns `Result<String, String>`; `render_with(template, &ctx, &partials)` also takes named templates for `include`. The context is a `HashMap<String, Value>`.

`Value` is one of `Nil`, `Bool`, `Num`, `Str`, `List` or `Map`. `From` conversions exist for `&str`, `i64`, `bool` and `Vec<Value>`.

## Usage from Node.js

Build the native addon from source (requires Rust):

```sh
git clone https://github.com/M3sca/stencil && cd stencil
npm install && npm run build
```

Or skip Rust: each [GitHub Release](https://github.com/M3sca/stencil/releases) has prebuilt binaries (macOS arm64/x64, Linux x64/arm64, Windows x64). Download the `.node` file for your platform into the repository folder, next to `index.js`.

Then, from your project:

```sh
npm install /path/to/stencil
```

```js
const { render } = require("stencil");

render("Hello {{ user.name | uppercase }}!", { user: { name: "Andrea" } });
// "Hello ANDREA!"
```

TypeScript types are included. The context is a plain JS object (values are converted as JSON: `null` becomes `nil`); errors are thrown as exceptions.

## Syntax

### Variables and filters

```text
{{ user.name }}                 dotted paths
{{ items.0 }}                   list index
{{ items.size }}                length of strings, lists and maps
{{ name | uppercase }}          filters
{{ items | join: ", " }}        filter arguments; chain filters with |
```

An undefined variable renders as an empty string. Supported literals are strings (`"a"` or `'a'`), numbers, `true`, `false` and `nil`.

| Filter | Effect |
|--------|--------|
| `uppercase`, `lowercase` | change case |
| `capitalize` | first letter uppercase, rest lowercase |
| `strip` | remove whitespace at both ends |
| `append: s`, `prepend: s` | add text after / before |
| `truncate: n` or `truncate: n, "…"` | cut to `n` characters, ellipsis included (default `...`) |
| `replace: old, new` | replace every occurrence of `old` |
| `split: sep` | split into a list (`split: ""` gives characters) |
| `url_encode` | percent-encode (space becomes `%20`) |
| `escape` | converts `& < > " '` to numeric entities |
| `default: v` | `v` if the value is nil, false or an empty string |
| `size` | length (0 if not applicable) |
| `plus: n`, `minus: n`, `times: n`, `divided_by: n` | arithmetic; `nil` counts as 0, numeric strings are accepted, `divided_by` is a decimal division |
| `round`, `round: digits`, `floor`, `ceil` | rounding (`digits` from 0 to 10) |
| `join: sep` | joins a list (default separator: space) |
| `first`, `last`, `reverse` | first / last item (or character), reversed list |
| `sort`, `sort: key` | sort numbers or strings (or maps by `key`); all of one kind, stable |
| `uniq` | remove duplicates, keeping the first of each |
| `map: key` | the `key` of every map in a list |
| `date`, `date: format` | format a Unix timestamp in UTC (default `%Y-%m-%d`) |

`date` understands `%Y %y %m %d %e %H %I %M %S %p %j %B %b %A %a %F %T %s %%`, for years 1 to 9999. Other `%x` sequences are printed as written.

### Conditions

```text
{% if n > 5 %}big{% elsif n == 5 %}five{% else %}small{% endif %}
```

Operators: `== != < > <= >= contains`, combinable with `not`, `and`, `or` and parentheses. `not` binds tightest, then `and`, then `or`. Only `nil` and `false` are falsy. `not` is a reserved word in conditions.

`unless` is `if not`, with an optional `else`:

```text
{% unless user.admin %}Limited access{% else %}Welcome back{% endunless %}
```

`case` compares one value against several (the first `when` that matches runs; values may be literals or variables):

```text
{% case role %}
  {% when "admin" %}Panel
  {% when "editor", "author" %}Editor
  {% else %}Read only
{% endcase %}
```

### Loops

```text
{% for p in projects %}{{ forloop.index }}. {{ p }}{% endfor %}
```

The list can be filtered: `{% for p in people | sort: "age" %}`.

`{% break %}` leaves the innermost loop and `{% continue %}` skips to its next item. They work from inside `if`, `case` and `block` within the loop, but not across an `include`, and outside a loop they are an error.

Inside a loop, `forloop` exposes `index`, `index0`, `first`, `last` and `length`. The loop variable is not visible outside the loop. In nested loops, `forloop` describes the innermost loop and `forloop.parentloop` the enclosing one (chainable: `forloop.parentloop.parentloop.index`); in the outermost loop `forloop.parentloop` is empty.

### Variables

```text
{% assign total = items | size %}
{% capture greeting %}Hello {{ user.name }}!{% endcapture %}
{{ greeting | uppercase }} ({{ total }} items)
```

`assign` stores the value of an expression (with filters); `capture` stores the text produced by a block. Assigning to a name that already exists (your own variable or an earlier `assign`) updates it; otherwise the new variable lives until the end of the enclosing loop iteration or include.

### Whitespace

Tags leave their surrounding whitespace in the output. A `-` next to the delimiter strips it: `{%-` and `{{-` remove the whitespace before, `-%}` and `-}}` the whitespace after (including line breaks).

```text
{% for x in xs -%}
  {{ x }}
{%- endfor %}
```

Write negative number literals with a space (`{{ -1 }}`, not `{{-1}}`), since `{{-` is whitespace control.

### Comments and literal text

```text
{# an inline comment #}
{% comment %}a block comment, {{ never rendered }}{% endcomment %}
{% raw %}{{ printed as is }} {% if x %}{% endraw %}
```

Comments produce no output. Inside `raw`, template syntax is not interpreted; use it to print `{{` and `{%` literally.

### Includes

```text
{% include "header" %}
```

Renders another template, with the same variables as the caller (including loop variables). Extra variables can be passed with `with`; they are evaluated in the caller's scope and are visible only inside the include:

```text
{% include "card" with title = post.title, n = items.size %}
```

Templates are passed by name, in memory:

```rust
let partials = Partials::from([("header".to_string(), "<h1>{{ title }}</h1>".to_string())]);
render_with("{% include \"header\" %}body", &ctx, &partials)?;
```

```js
render('{% include "header" %}body', { title: "Hi" }, { header: "<h1>{{ title }}</h1>" });
```

Names are plain keys of that map: they are never interpreted as file paths and the engine never reads the filesystem, so path traversal (`../../etc/passwd`) is not possible. The name must be a quoted literal; it cannot be built from a variable. Nesting is limited to 32 levels (so a template that includes itself fails with an error) and a single render to 100,000 includes.

### Template inheritance

A layout declares blocks; a page extends it and overrides some of them:

```text
layout:                                        page:
<h1>{% block title %}Site{% endblock %}</h1>   {% extends "layout" %}
{% block body %}{% endblock %}                 {% block body %}Hello {{ name }}{% endblock %}
```

Rendering `page` gives the layout with its `body` replaced and `title` left as written.

- `{% extends "name" %}` must be the first tag of the template, used once, and the name must be a quoted literal. The parent is looked up in the partials, like `include`.
- Everything outside `{% block %}` in a child template is ignored.
- Chains work (`base` → `section` → `page`); the most derived override wins. A block nested in another block can be overridden separately.
- Blocks see the same variables as the rest of the template, including loop variables.
- Without `extends`, a block just renders its body.
- A block name used twice in one template is an error. A block in a child that doesn't exist in any parent is silently ignored. There is no `block.super`.

## Limits

Templates are code, so a malicious one must not be able to exhaust memory, CPU or the stack. A render fails with an error when:

| Limit | Value |
|-------|-------|
| rendered output | 10 MiB |
| nested `if`/`for`/`block` in one template, and nested parentheses in a condition | 50 levels |
| total nested rendering depth (blocks, loops and includes together) | 100 levels |
| include/extends chain | 32 levels |
| includes in one render | 100,000 |
| loop iterations in one render | 1,000,000 |
| nodes executed in one render | 2,000,000 |
| one computed value (`assign`, `capture`, filter result) | 16 MiB |
| all values stored by `assign`/`capture` in one render | 128 MiB |

These are constants at the top of `src/lib.rs`. Each partial is parsed once per render. The size of the template, the partials and the context is up to the caller.

## Security notes

- **No automatic escaping.** `{{ value }}` is inserted as is. When rendering HTML with untrusted values, apply `| escape` yourself (`{{ comment | escape }}`).
- **No filesystem access.** Included and extended templates come from the map you pass in; names are never treated as paths.
- Templates cannot call arbitrary code: there is no way to reach anything except the values you pass in.

## Errors

Unclosed delimiters or blocks, unknown tags and filters, empty variable names and invalid conditions return `Err` with a descriptive message that includes the template line, e.g. `Unknown filter: nope (line 3)`. An error inside an included template reads `... (line 2) (in 'name')`, with the line in that template.

## Not supported

Custom filters or tags registered from Rust or Node.js, time zones, and locales.

## Development

```sh
cargo test    # Rust tests
cargo run --example demo   # demo in examples/demo.rs
npm test      # Node binding tests (after npm run build)

Releases: pushing a tag like `v0.1.0` makes GitHub Actions build every platform and attach the binaries to a GitHub Release.
```

## License

[MIT](LICENSE)
