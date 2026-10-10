use crate::limits::{MAX_INCLUDE_DEPTH, MAX_NESTING};
use crate::{Context, Partials, Value, render, render_with};
use std::collections::HashMap;

fn ctx(pairs: Vec<(&str, Value)>) -> Context {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[test]
fn variables_paths_and_filters() {
    let user = Value::Map(HashMap::from([("name".into(), "Andrea".into())]));
    let c = ctx(vec![("user", user), ("xs", vec![1.into(), 2.into()].into())]);
    assert_eq!(render("Hello {{ user.name | uppercase }}!", &c).unwrap(), "Hello ANDREA!");
    assert_eq!(render("{{ x | default: 'n/a' }}", &c).unwrap(), "n/a");
    assert_eq!(render("{{ xs | join: ', ' }}", &c).unwrap(), "1, 2");
    assert_eq!(render("{{ xs.size }}", &c).unwrap(), "2");
    assert_eq!(render("{{ '<a>' | escape }}", &c).unwrap(), "&#60;a&#62;");
}

#[test]
fn if_elsif_else() {
    let t = "{% if n > 5 %}big{% elsif n == 5 %}five{% else %}small{% endif %}";
    for (n, want) in [(9, "big"), (5, "five"), (1, "small")] {
        assert_eq!(render(t, &ctx(vec![("n", n.into())])).unwrap(), want);
    }
    let c = ctx(vec![("a", true.into())]);
    assert_eq!(render("{% if a and b %}y{% endif %}", &c).unwrap(), "");
    assert_eq!(render("{% if a or b %}y{% endif %}", &c).unwrap(), "y");
    // `and` binds tighter than `or`; parentheses override; connectives inside strings are plain text.
    assert_eq!(render("{% if a or b and c %}y{% endif %}", &c).unwrap(), "y");
    assert_eq!(render("{% if (a or b) and c %}y{% endif %}", &c).unwrap(), "");
    let c = ctx(vec![("s", "x and y".into())]);
    assert_eq!(render("{% if s == 'x and y' %}y{% endif %}", &c).unwrap(), "y");
    assert!(render("{% if (a %}y{% endif %}", &c).is_err());
}

#[test]
fn for_loops() {
    let c = ctx(vec![("xs", vec!["a".into(), "b".into()].into())]);
    assert_eq!(render("{% for x in xs %}{{ forloop.index }}={{ x }};{% endfor %}", &c).unwrap(), "1=a;2=b;");
    let meta = "{% for x in xs %}{% if forloop.first %}[{% endif %}{{ forloop.index0 }}/{{ forloop.length }}{% if forloop.last %}]{% endif %}{% endfor %}";
    assert_eq!(render(meta, &c).unwrap(), "[0/21/2]");
    // `forloop` doesn't outlive its loop, and in nested loops it is the innermost one's.
    assert_eq!(render("{% for x in xs %}{% for y in xs %}{{ forloop.index }}{% endfor %}{{ forloop.index }}{% endfor %}x{{ forloop.index }}", &c).unwrap(), "121122x");
    // `forloop.parentloop` reaches the enclosing loops, chained for deeper nesting; the outermost has none.
    let t = "{% for a in xs %}{% for b in xs %}{% for c in xs %}{{ forloop.parentloop.parentloop.index }}{{ forloop.parentloop.index }}{{ forloop.index }}{% if forloop.last %};{% endif %}{% endfor %}{% endfor %}{% endfor %}";
    assert_eq!(render(t, &c).unwrap(), "111112;121122;211212;221222;");
    assert_eq!(render("{% for a in xs %}[{{ forloop.parentloop.index }}]{% endfor %}", &c).unwrap(), "[][]");
    let rows = vec![vec![1.into(), 2.into()].into(), vec![3.into()].into()];
    let c = ctx(vec![("rows", rows.into())]);
    let t = "{% for r in rows %}{% for c in r %}{{ c }}{% endfor %}|{% endfor %}";
    assert_eq!(render(t, &c).unwrap(), "12|3|");
    // Loop variables don't leak out of the loop.
    assert_eq!(render("{% for x in xs %}{% endfor %}[{{ x }}]", &c).unwrap(), "[]");
}

#[test]
fn includes() {
    let partials = Partials::from([
        ("row".to_string(), "<{{ x }}>".to_string()),
        ("list".to_string(), "{% for x in xs %}{% include 'row' %}{% endfor %}".to_string()),
        ("loop".to_string(), "{% include \"loop\" %}".to_string()),
        ("bad".to_string(), "{% if %}".to_string()),
        ("../../etc/passwd".to_string(), "should never be reached by path".to_string()),
    ]);
    let c = ctx(vec![("xs", vec![1.into(), 2.into()].into()), ("x", "g".into())]);
    let r = |t: &str| render_with(t, &c, &partials);
    assert_eq!(r("{% include \"row\" %}").unwrap(), "<g>"); // sees caller variables
    assert_eq!(r("{% include \"list\" %}").unwrap(), "<1><2>"); // nested, loop variable visible
    assert!(r("{% include \"nope\" %}").unwrap_err().contains("Unknown template"));
    assert!(r("{% include \"loop\" %}").unwrap_err().contains("too deep"));
    assert!(r("{% include row %}").unwrap_err().contains("quoted"));
    assert!(r("{% include \"\" %}").is_err());
    assert!(r("{% include \"bad\" %}").unwrap_err().contains("in 'bad'"));
    // Names are only map keys: a traversal-looking name matches only an identical key.
    assert!(r("{% include \"../Cargo.toml\" %}").unwrap_err().contains("Unknown template"));
    // Templates that include each other many times over are cut off instead of exploding.
    let mut chain: Partials = (0..25).map(|i| (format!("p{i}"), format!("{{% include 'p{}' %}}{{% include 'p{}' %}}", i + 1, i + 1))).collect();
    chain.insert("p25".into(), "x".into());
    assert!(render_with("{% include 'p0' %}", &c, &chain).unwrap_err().contains("Too many includes"));
    // `with` passes extra variables, evaluated in the caller's scope, visible only in the include.
    let p2 = Partials::from([("card".to_string(), "[{{ title }}|{{ n }}]".to_string())]);
    let w = |t: &str| render_with(t, &c, &p2);
    assert_eq!(w("{% include 'card' with title = 'A, b', n = 3 %}").unwrap(), "[A, b|3]");
    assert_eq!(w("{% for x in xs %}{% include 'card' with title = x, n = xs.size %}{% endfor %}").unwrap(), "[1|2][2|2]");
    assert_eq!(w("{% include 'card' with title = 'T' %}{{ title }}.").unwrap(), "[T|].");
    for bad in ["with", "with a", "with = 1", "with a =", "with 1x y = 2", "x = 1"] {
        assert!(w(&format!("{{% include 'card' {bad} %}}")).is_err(), "{bad}");
    }
    // Includes need registered partials: plain render() has none.
    assert!(render("{% include \"row\" %}", &c).is_err());
}

#[test]
fn iterations_are_capped() {
    let big: Vec<Value> = (0..1000).map(|i| (i as i64).into()).collect();
    let small: Vec<Value> = (0..500).map(|i| (i as i64).into()).collect();
    let c = ctx(vec![("big", big.into()), ("small", small.into())]);
    let t = "{% for a in big %}{% for b in big %}{% for c in big %}{% endfor %}{% endfor %}{% endfor %}";
    assert!(render(t, &c).unwrap_err().contains("Too many loop iterations"));
    assert_eq!(render("{% for a in small %}{% for b in small %}{% endfor %}{% endfor %}", &c).unwrap(), "");
}

#[test]
fn extends_and_blocks() {
    let partials = Partials::from([
        ("layout".to_string(), "<h1>{% block title %}Site{% endblock %}</h1>|{% block body %}{% endblock %}".to_string()),
        ("section".to_string(), "{% extends 'layout' %}{% block title %}{{ title }}!{% endblock %}{% block body %}S:{% block inner %}in{% endblock %}{% endblock %}".to_string()),
        ("rows".to_string(), "{% for x in xs %}{% block row %}-{% endblock %}{% endfor %}".to_string()),
        ("self".to_string(), "{% extends 'self' %}".to_string()),
        ("dup".to_string(), "{% block a %}{% endblock %}{% block a %}{% endblock %}".to_string()),
        ("late".to_string(), "x{% extends 'layout' %}".to_string()),
        ("with".to_string(), "{% extends 'layout' with a = 1 %}".to_string()),
        ("page".to_string(), "{% extends 'layout' %}\n{% block body %}Hi {{ name }}{% endblock %}".to_string()),
    ]);
    let c = ctx(vec![("name", "Ann".into()), ("title", "T".into()), ("xs", vec![1.into(), 2.into()].into())]);
    let r = |t: &str| render_with(t, &c, &partials);
    // A child overrides some blocks and inherits the rest; text outside blocks is ignored.
    assert_eq!(r("{% include 'page' %}").unwrap(), "<h1>Site</h1>|Hi Ann");
    assert_eq!(r("{% extends 'layout' %}ignored{% block body %}B{% endblock %}").unwrap(), "<h1>Site</h1>|B");
    // Several levels, and overriding a block nested in a parent's block.
    assert_eq!(r("{% extends 'section' %}{% block inner %}IN2{% endblock %}").unwrap(), "<h1>T!</h1>|S:IN2");
    assert_eq!(r("{% extends 'section' %}").unwrap(), "<h1>T!</h1>|S:in");
    // Blocks inside loops see the loop variable.
    assert_eq!(r("{% extends 'rows' %}{% block row %}{{ x }}{% endblock %}").unwrap(), "12");
    // Without extends a block just renders its body; unknown blocks in a child are ignored.
    assert_eq!(r("{% block a %}d{% endblock %}").unwrap(), "d");
    assert_eq!(r("{% extends 'layout' %}{% block nope %}x{% endblock %}").unwrap(), "<h1>Site</h1>|");
    // Errors.
    assert!(r("{% extends 'nope' %}").unwrap_err().contains("Unknown template"));
    assert!(r("{% extends 'self' %}").unwrap_err().contains("too deep"));
    assert!(r("{% include 'dup' %}").unwrap_err().contains("Duplicate block"));
    assert!(r("{% include 'late' %}").unwrap_err().contains("first tag"));
    assert!(r("{% include 'with' %}").unwrap_err().contains("only a quoted"));
    assert!(r("{% extends layout %}").unwrap_err().contains("quoted"));
    assert!(r("{% extends 'layout' %}{% extends 'layout' %}").is_err());
    assert!(r("{% if true %}{% extends 'layout' %}{% endif %}").is_err());
    assert!(r("{% block %}{% endblock %}").is_err());
    assert!(r("{% block a b %}{% endblock %}").is_err());
    assert!(r("{% block a %}").is_err());
}

#[test]
fn hostile_templates_are_bounded() {
    let c = Context::new();
    // Many tags in a row: parsing must be linear (it used to be quadratic).
    let flat = "{% for a in xs %}{% endfor %}".repeat(30_000);
    assert_eq!(render(&flat, &c).unwrap(), "");
    // Deep nesting is refused instead of overflowing the stack.
    let deep = |open: &str, close: &str| format!("{}x{}", open.repeat(100_000), close.repeat(100_000));
    for (o, cl) in [("{% if true %}", "{% endif %}"), ("{% for a in xs %}", "{% endfor %}"), ("{% block a %}", "{% endblock %}")] {
        assert!(render(&deep(o, cl), &c).unwrap_err().contains("nested too deeply"), "{o}");
    }
    let parens = format!("{{% if {}true{} %}}x{{% endif %}}", "(".repeat(100_000), ")".repeat(100_000));
    assert!(render(&parens, &c).unwrap_err().contains("nested too deeply"));
    // Deep nesting spread over a long include chain is cut by the total depth budget, not by a stack overflow.
    let nest = |inner: &str| format!("{}{}{}", "{% if (((true))) %}".repeat(MAX_NESTING - 1), inner, "{% endif %}".repeat(MAX_NESTING - 1));
    let mut partials: Partials = (0..MAX_INCLUDE_DEPTH - 1).map(|i| (format!("p{i}"), nest(&format!("{{% include 'p{}' %}}", i + 1)))).collect();
    partials.insert(format!("p{}", MAX_INCLUDE_DEPTH - 1), "x".into());
    assert!(render_with("{% include 'p0' %}", &c, &partials).unwrap_err().contains("nested too deeply"));
    // Ordinary nesting through ordinary includes works.
    let mut ok: Partials = (0..8).map(|i| (format!("p{i}"), format!("{{% if true %}}{{% if true %}}{{% include 'p{}' %}}{{% endif %}}{{% endif %}}", i + 1))).collect();
    ok.insert("p8".into(), "x".into());
    assert_eq!(render_with("{% include 'p0' %}", &c, &ok).unwrap(), "x");
}

#[test]
fn work_is_bounded_and_partials_are_parsed_once() {
    let big: Vec<Value> = (0..1000).map(|i| (i as i64).into()).collect();
    let c = ctx(vec![("big", big.into())]);
    // Lots of cheap nodes inside one big partial included in a loop: capped by total steps.
    let partials = Partials::from([("p".to_string(), "{% if false %}{% endif %}".repeat(3000))]);
    let err = render_with("{% for a in big %}{% include 'p' %}{% endfor %}", &c, &partials).unwrap_err();
    assert!(err.contains("too complex"), "{err}");
}

#[test]
fn comments_and_raw() {
    let c = ctx(vec![("x", "v".into())]);
    assert_eq!(render("a{# a {{ x }} comment #}b", &c).unwrap(), "ab");
    assert_eq!(render("a{% comment %}{{ x }}{% if %}{% endcomment %}b", &c).unwrap(), "ab");
    // Raw content is output as is, including template syntax.
    assert_eq!(render("{% raw %}{{ x }} {% if %}{% endraw %}!{{ x }}", &c).unwrap(), "{{ x }} {% if %}!v");
    assert_eq!(render("{% raw %}{% endraw %}x", &c).unwrap(), "x");
    // Unclosed ones are errors, with the line where they start.
    assert_eq!(render("a\n{% raw %}b", &c).unwrap_err(), "Unclosed block: missing 'endraw' (line 2)");
    assert!(render("{% comment %}b", &c).unwrap_err().contains("endcomment"));
    assert!(render("a{# b", &c).unwrap_err().contains("'#}'"));
    // Lines keep counting through comments and raw blocks.
    assert!(render("{# a\nb #}\n{% raw %}\n\n{% endraw %}\n{% nope %}", &c).unwrap_err().ends_with("(line 6)"));
    // A very long run of `{%` without a closing tag stays linear.
    assert!(render(&format!("{{% raw %}}{}", "{%".repeat(500_000)), &c).is_err());
}

#[test]
fn errors_have_line_numbers() {
    let c = ctx(vec![("x", "v".into())]);
    let partials = Partials::from([("p".to_string(), "ok\n{{ x | nope }}".to_string())]);
    let err = |t: &str| render_with(t, &c, &partials).unwrap_err();
    assert_eq!(err("a\nb\n{% nope %}"), "Unknown tag: nope (line 3)");
    assert_eq!(err("{{ x }}\n{{ }}"), "Variable name cannot be empty (line 2)");
    assert_eq!(err("{% if x %}\n\n{% block %}"), "Invalid block name: '' (line 3)");
    assert_eq!(err("{% if x %}\n\n"), "Unclosed block: missing 'endif' (line 1)");
    assert_eq!(err("{{ x\n\n"), "Unclosed delimiter: missing '}}' (line 1)");
    // Runtime errors: unknown filter, bad condition (the `elsif` line), unknown template.
    assert_eq!(err("\n{{ x | nope }}"), "Unknown filter: nope (line 2)");
    assert_eq!(err("{% if false %}\n{% elsif (x %}\n{% endif %}"), "Invalid condition: missing ')' (line 2)");
    assert_eq!(err("a\n\n{% include 'zz' %}"), "Unknown template: zz (line 3)");
    // Errors inside an included template point to its own line.
    assert_eq!(err("{% include 'p' %}"), "Unknown filter: nope (line 2) (in 'p')");
    // Multi-line tags and outputs count their newlines.
    assert!(err("{{ x\n}}\n{% nope %}").contains("(line 3)"));
}

#[test]
fn whitespace_control() {
    let c = ctx(vec![("x", "v".into()), ("xs", vec![1.into(), 2.into()].into())]);
    let r = |t: &str| render(t, &c).unwrap();
    // Without dashes whitespace is kept.
    assert_eq!(r("a {% if true %} b {% endif %} c"), "a  b  c");
    // `{%-` strips before the tag, `-%}` after it; same for `{{-` and `-}}`.
    assert_eq!(r("a  {%- if true -%}  b  {%- endif -%}  c"), "abc");
    assert_eq!(r("a  {{- x -}}  b"), "avb");
    assert_eq!(r("a  {{- x }}  b"), "av  b");
    assert_eq!(r("{% for x in xs -%}\n  {{ x }}\n{%- endfor %}"), "12");
    assert_eq!(r("{% for x in xs %}\n{{ x }}\n{% endfor %}"), "\n1\n\n2\n");
    assert_eq!(r("{% if true -%}x{% endif -%}  \n"), "x");
    assert_eq!(r("a {#- c -#} b"), "ab");
    assert_eq!(r("{%- raw -%}  {{x}}  {%- endraw -%}"), "{{x}}");
    assert_eq!(r("{{ -1 }}"), "-1");
    // Line numbers don't change.
    assert_eq!(render("a\n\n{%- nope %}", &c).unwrap_err(), "Unknown tag: nope (line 3)");
}

#[test]
fn unless_and_not() {
    let c = ctx(vec![("a", true.into()), ("b", false.into())]);
    let r = |t: &str| render(t, &c).unwrap();
    assert_eq!(r("{% unless a %}no{% endunless %}"), "");
    assert_eq!(r("{% unless b %}no{% endunless %}"), "no");
    assert_eq!(r("{% unless a %}A{% else %}B{% endunless %}"), "B");
    assert_eq!(r("{% unless missing %}M{% endunless %}"), "M");
    assert_eq!(r("{% if not a %}y{% else %}n{% endif %}"), "n");
    assert_eq!(r("{% if not b and a %}y{% endif %}"), "y");
    assert_eq!(r("{% if not (a or b) %}y{% else %}n{% endif %}"), "n");
    assert_eq!(r("{% if not not a %}y{% endif %}"), "y");
    assert_eq!(r("{% if not a == b %}y{% endif %}"), "y");
    assert!(render("{% unless a %}x", &c).unwrap_err().contains("endunless"));
    assert!(render("{% unless %}x{% endunless %}", &c).is_err());
    // A long run of `not` is refused, not a stack overflow.
    let t = format!("{{% if {}a %}}x{{% endif %}}", "not ".repeat(100_000));
    assert!(render(&t, &c).unwrap_err().contains("nested too deeply"));
}

#[test]
fn case_when() {
    let c = ctx(vec![("n", 2.into()), ("s", "b".into()), ("k", "b".into())]);
    let r = |t: &str| render(t, &c).unwrap();
    assert_eq!(r("{% case n %}{% when 1 %}one{% when 2 %}two{% else %}other{% endcase %}"), "two");
    assert_eq!(r("{% case n %}{% when 1 %}one{% else %}other{% endcase %}"), "other");
    assert_eq!(r("{% case n %}{% when 1 %}one{% endcase %}"), "");
    assert_eq!(r("{% case s %}{% when 'a', 'b' %}ab{% when 'c' %}c{% endcase %}"), "ab");
    assert_eq!(r("{% case s %}{% when k %}same{% endcase %}"), "same");
    // Text between `case` and the first `when` is ignored; only the first match runs.
    assert_eq!(r("{% case n %}ignored{% when 2 %}x{% when 2 %}y{% endcase %}"), "x");
    assert_eq!(r("{% case n %}{% when 2 %}{% if true %}in{% endif %}{% endcase %}"), "in");
    assert!(render("{% case %}{% endcase %}", &c).is_err());
    assert!(render("{% case n %}{% when %}x{% endcase %}", &c).is_err());
    assert!(render("{% case n %}{% when 1 %}x", &c).unwrap_err().contains("endcase"));
}

#[test]
fn break_and_continue() {
    let xs: Vec<Value> = (1..=3).map(|i| (i as i64).into()).collect();
    let c = ctx(vec![("xs", xs.into())]);
    let r = |t: &str| render(t, &c);
    assert_eq!(r("{% for x in xs %}{% if x == 3 %}{% break %}{% endif %}{{ x }}{% endfor %}").unwrap(), "12");
    assert_eq!(r("{% for x in xs %}{% if x == 2 %}{% continue %}{% endif %}{{ x }}{% endfor %}").unwrap(), "13");
    // Only the innermost loop stops; through case and block too.
    let nested = "{% for a in xs %}{% for b in xs %}{% if b == 2 %}{% break %}{% endif %}{{ a }}{{ b }},{% endfor %}|{% endfor %}";
    assert_eq!(r(nested).unwrap(), "11,|21,|31,|");
    let case = "{% for x in xs %}{% case x %}{% when 2 %}{% continue %}{% endcase %}{{ x }}{% endfor %}";
    assert_eq!(r(case).unwrap(), "13");
    // Text after a break in the same body is skipped.
    assert_eq!(r("{% for x in xs %}a{% break %}b{% endfor %}").unwrap(), "a");
    // Errors: outside a loop, or across an include.
    assert_eq!(r("a\n{% break %}").unwrap_err(), "break outside a loop (line 2)");
    assert!(r("{% continue 1 %}").unwrap_err().contains("no arguments"));
    let partials = Partials::from([("p".to_string(), "{% break %}".to_string())]);
    let err = render_with("{% for x in xs %}{% include 'p' %}{% endfor %}", &c, &partials).unwrap_err();
    assert!(err.contains("break outside a loop"), "{err}");
    // The loop bookkeeping stays consistent after a break.
    assert_eq!(r("{% for x in xs %}{% break %}{% endfor %}{% break %}").unwrap_err(), "break outside a loop (line 1)");
}

#[test]
fn assign_and_capture() {
    let c = ctx(vec![("x", "v".into()), ("xs", vec![1.into(), 2.into(), 3.into()].into())]);
    let r = |t: &str| render(t, &c).unwrap();
    assert_eq!(r("{% assign a = 'hi' %}{{ a }}"), "hi");
    assert_eq!(r("{% assign n = xs.size %}{{ n }}"), "3");
    assert_eq!(r("{% assign u = x | uppercase %}{{ u }}"), "V");
    // Shadows a caller variable; the nearest one is updated, also from inside a loop.
    assert_eq!(r("{% assign x = 'w' %}{{ x }}"), "w");
    assert_eq!(r("{% assign c = 0 %}{% for i in xs %}{% assign c = i %}{% endfor %}{{ c }}"), "3");
    // A variable created inside a loop iteration ends with it.
    assert_eq!(r("{% for i in xs %}{% assign t = i %}{% endfor %}[{{ t }}]"), "[]");
    assert_eq!(r("{% capture g %}Hello {{ x }}!{% endcapture %}{{ g | uppercase }}|{{ g }}"), "HELLO V!|Hello v!");
    assert_eq!(r("{% capture list %}{% for i in xs %}{{ i }}{% endfor %}{% endcapture %}{{ list }}"), "123");
    for bad in ["{% assign %}", "{% assign a %}", "{% assign a = %}", "{% assign 1 b = 2 %}", "{% assign a-b = 1 %}", "{% capture %}{% endcapture %}", "{% capture a b %}{% endcapture %}"] {
        assert!(render(bad, &c).is_err(), "{bad}");
    }
    assert!(render("{% capture a %}x", &c).unwrap_err().contains("endcapture"));
}

#[test]
fn stored_values_are_bounded() {
    // 9 MB captured per iteration, stored under one name: the running total is capped, not just each value.
    let c = ctx(vec![("s", "x".repeat(1 << 20).as_str().into()), ("xs", (0..30).map(|i| (i as i64).into()).collect::<Vec<Value>>().into())]);
    let t = format!("{{% for i in xs %}}{{% capture c %}}{}{{% endcapture %}}{{% endfor %}}", "{{ s }}".repeat(9));
    assert!(render(&t, &c).unwrap_err().contains("Too much data"));
}

fn person(name: &str, age: i64) -> Value {
    Value::Map(HashMap::from([("name".to_string(), name.into()), ("age".to_string(), age.into())]))
}

#[test]
fn text_filters() {
    let c = ctx(vec![("s", "  Hello World  ".into()), ("t", "Hello World".into())]);
    let r = |t: &str| render(t, &c).unwrap();
    assert_eq!(r("[{{ s | strip }}]"), "[Hello World]");
    assert_eq!(r("{{ t | append: '!' }}{{ t | prepend: '>' }}"), "Hello World!>Hello World");
    assert_eq!(r("{{ t | truncate: 8 }}|{{ t | truncate: 20 }}|{{ t | truncate: 5, '' }}|{{ t | truncate: 8, '~' }}"), "Hello...|Hello World|Hello|Hello W~");
    assert_eq!(r("{{ t | replace: 'o', '0' }}"), "Hell0 W0rld");
    assert_eq!(r("{{ 'a,b,c' | split: ',' | join: '-' }}"), "a-b-c");
    assert_eq!(r("{{ 'abc' | split: '' | join: '.' }}"), "a.b.c");
    assert_eq!(r("{{ 'a b' | url_encode }}|{{ 'é&' | url_encode }}|{{ 'x-_.~' | url_encode }}"), "a%20b|%C3%A9%26|x-_.~");
    for (bad, msg) in [("{{ t | append }}", "needs an argument"), ("{{ t | replace: '', 'x' }}", "non-empty"), ("{{ t | truncate: -1 }}", "non-negative"), ("{{ t | split }}", "needs an argument")] {
        assert!(render(bad, &c).unwrap_err().contains(msg), "{bad}");
    }
}

#[test]
fn number_filters() {
    let c = ctx(vec![("s", "4".into())]);
    let r = |t: &str| render(t, &c).unwrap();
    assert_eq!(r("{{ 2 | plus: 3 }} {{ 10 | minus: 4 }} {{ 3 | times: 2.5 }} {{ 7 | divided_by: 2 }}"), "5 6 7.5 3.5");
    assert_eq!(r("{{ missing | plus: 1 }} {{ s | plus: 1 }}"), "1 5");
    assert_eq!(r("{{ 3.14159 | round: 2 }} {{ 2.5 | round }} {{ 2.7 | floor }} {{ 2.1 | ceil }} {{ -2.5 | round }}"), "3.14 3 2 3 -3");
    for (bad, msg) in [("{{ 7 | divided_by: 0 }}", "Division by zero"), ("{{ 'x' | plus: 1 }}", "expects a number"), ("{{ 1 | plus: 'y' }}", "expects a number"),
                       ("{{ 1 | round: 11 }}", "0 to 10"), ("{{ 1e308 | times: 10 }}", "out of range"), ("{{ 1 | plus }}", "needs an argument")] {
        assert!(render(bad, &c).unwrap_err().contains(msg), "{bad}");
    }
}

#[test]
fn list_filters() {
    let people = vec![person("bob", 30), person("amy", 25), person("cy", 30)];
    let c = ctx(vec![
        ("xs", vec![3.into(), 1.into(), 2.into()].into()),
        ("dup", vec![1.into(), 1.into(), 2.into(), 1.into(), "1".into()].into()),
        ("words", vec!["pear".into(), "apple".into()].into()),
        ("people", people.into()),
        ("mixed", vec![1.into(), "a".into()].into()),
        ("nan", vec![Value::Num(f64::NAN), 1.into()].into()),
    ]);
    let r = |t: &str| render(t, &c).unwrap();
    assert_eq!(r("{{ xs | sort | join: '' }} {{ xs | reverse | join: '' }} {{ xs | first }} {{ xs | last }}"), "123 213 3 2");
    assert_eq!(r("{{ words | sort | join: ',' }} {{ 'abc' | first }}{{ 'abc' | last }}"), "apple,pear ac");
    assert_eq!(r("{{ dup | uniq | size }} {{ dup | uniq | join: '' }}"), "3 121");
    assert_eq!(r("{{ people | map: 'name' | join: ',' }}"), "bob,amy,cy");
    // Sorting by a key is stable.
    assert_eq!(r("{{ people | sort: 'age' | map: 'name' | join: ',' }}"), "amy,bob,cy");
    assert_eq!(r("{{ people | uniq | size }} {{ missing | map: 'x' | size }} {{ xs | first }}"), "3 0 3");
    assert_eq!(r("{{ xs | sort | map: 'x' | size }}"), "3");
    // A loop can iterate over a filtered list.
    assert_eq!(r("{% for p in people | sort: 'age' %}{{ p.name }}{% endfor %}|{% for x in xs | reverse %}{{ x }}{% endfor %}"), "amy,bob,cy".replace(',', "") + "|213");
    // Variables named like float words are still variables.
    let w = ctx(vec![("inf", 5.into()), ("nan", "n".into()), ("e", "E".into())]);
    assert_eq!(render("{{ inf }}{{ nan }}{{ e }}{{ 1e2 }}{{ -3 }}{{ .5 }}", &w).unwrap(), "5nE100-30.5");
    for (bad, msg) in [("{{ mixed | sort }}", "same kind"), ("{{ nan | sort }}", "same kind"), ("{{ xs | sort: 'k' }}", "list of maps"),
                       ("{{ 5 | map: 'a' }}", "expects a list"), ("{{ xs | map }}", "needs an argument"), ("{{ people | sort }}", "same kind")] {
        assert!(render(bad, &c).unwrap_err().contains(msg), "{bad}");
    }
}

#[test]
fn date_filter() {
    let c = Context::new();
    let r = |t: &str| render(t, &c).unwrap();
    assert_eq!(r("{{ 0 | date }}"), "1970-01-01");
    assert_eq!(r("{{ 1700000000 | date: '%A, %d %B %Y %H:%M:%S' }}"), "Tuesday, 14 November 2023 22:13:20");
    assert_eq!(r("{{ 1700000000 | date: '%a %b %e %y %j %I:%M %p %F %T' }}"), "Tue Nov 14 23 318 10:13 PM 2023-11-14 22:13:20");
    assert_eq!(r("{{ 951782400 | date: '%F %j' }}"), "2000-02-29 060"); // leap day
    assert_eq!(r("{{ -86400 | date: '%F %A' }}"), "1969-12-31 Wednesday");
    assert_eq!(r("{{ '86400' | date: '%s %% %Q' }}"), "86400 % %Q");
    assert_eq!(r("{{ 253402300799 | date: '%F %T' }}"), "9999-12-31 23:59:59");
    assert_eq!(r("{{ -62135596800 | date }}"), "0001-01-01");
    assert!(render("{{ 253402300800 | date }}", &c).unwrap_err().contains("out of range"));
    assert!(render("{{ 'x' | date }}", &c).is_err());
}

#[test]
fn filters_cannot_blow_up_memory() {
    let mib = "a".repeat(1 << 20);
    let big: Vec<Value> = (0..1000).map(|i| (i as i64).into()).collect();
    let c = ctx(vec![("s", mib.as_str().into()), ("big", big.into())]);
    let err = |t: &str| render(t, &c).unwrap_err();
    // Each of these would allocate far more than the cap: refused before allocating.
    assert!(err(&format!("{{{{ s | replace: 'a', '{}' }}}}", "b".repeat(100))).contains("too large"));
    assert!(err(&format!("{{{{ big | join: '{}' }}}}", "-".repeat(100_000))).contains("too large"));
    assert!(err("{% assign t = s %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}{% assign t = t | append: t %}").contains("too large"));
    let long = ctx(vec![("s", "a".repeat(5 << 20).as_str().into())]);
    assert!(render("{{ s | split: '' | size }}", &long).unwrap_err().contains("too large"));
    assert!(render("{{ s | url_encode }}{{ s | url_encode }}{{ s | url_encode }}", &long).is_err());
    // A string that doubles on every loop turn stops at the cap instead of eating memory.
    assert!(err("{% assign s = 'xxxxxxxxxx' %}{% for i in big %}{% assign s = s | append: s %}{% endfor %}").contains("too large"));
    // Repeating an expensive filter is charged for the data it walks.
    let lots = ctx(vec![("lots", (0..100_000).map(|i| (i as i64).into()).collect::<Vec<Value>>().into()), ("big", (0..1000).map(|i| (i as i64).into()).collect::<Vec<Value>>().into())]);
    let e = render("{% for i in big %}{{ lots | reverse | size }}{% endfor %}", &lots).unwrap_err();
    assert!(e.contains("too complex"), "{e}");
}

#[test]
fn output_is_capped() {
    let big: Vec<Value> = (0..1000).map(|i| (i as i64).into()).collect();
    let c = ctx(vec![("big", big.into()), ("s", "x".repeat(100).as_str().into())]);
    let t = "{% for a in big %}{% for b in big %}{{ s }}{% endfor %}{% endfor %}";
    assert!(render(t, &c).unwrap_err().contains("Output too large"));
    assert!(render("{% for a in big %}{{ s }}{% endfor %}", &c).unwrap().len() == 100_000);
}

#[test]
fn errors() {
    let c = Context::new();
    assert!(render("Hello {{ name", &c).is_err());
    assert!(render("{% if a %}x", &c).is_err());
    assert!(render("{% boh %}", &c).is_err());
    assert!(render("{{ a | nope }}", &c).is_err());
    assert_eq!(render("Hello {{ name }}!", &c).unwrap(), "Hello !");
}
