use stencil::{Context, Value, render};

fn main() -> Result<(), String> {
    let template = "Hello {{ name | uppercase }}! Projects:{% for p in projects %} {{ p }}{% if forloop.last %}.{% else %},{% endif %}{% endfor %}";

    let context = Context::from([
        ("name".to_string(), "Andrea".into()),
        ("projects".to_string(), Value::List(vec!["stencil".into(), "other".into()])),
    ]);

    println!("{}", render(template, &context)?);
    Ok(())
}
