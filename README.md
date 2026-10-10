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

`render(template, &ctx)` returns `Result<String, String>`. The context is a `HashMap<String, Value>`.

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
| `size` | length (0 if not applicable) |
| `join: sep` | joins a list (default separator: space) |
| `default: v` | `v` if the value is nil, false or an empty string |
| `escape` | converts `& < > " '` to numeric entities |

### Conditions

```text
{% if n > 5 %}big{% elsif n == 5 %}five{% else %}small{% endif %}
```

Operators: `== != < > <= >= contains`, combinable with `and`, `or` and parentheses. `and` binds tighter than `or`. Only `nil` and `false` are falsy.

### Loops

```text
{% for p in projects %}{{ forloop.index }}. {{ p }}{% endfor %}
```

Inside a loop, `forloop` exposes `index`, `index0`, `first`, `last` and `length`. The loop variable is not visible outside the loop.

## Errors

Unclosed delimiters or blocks, unknown tags and filters, empty variable names and invalid conditions return `Err` with a descriptive message.

## Not supported

`assign`, `capture`, `unless`, `case`, whitespace control (`{%-`), `break`/`continue`, comments.

## Development

```sh
cargo test    # Rust tests
cargo run --example demo   # demo in examples/demo.rs
npm test      # Node binding tests (after npm run build)

Releases: pushing a tag like `v0.1.0` makes GitHub Actions build every platform and attach the binaries to a GitHub Release.
```

## License

[MIT](LICENSE)
