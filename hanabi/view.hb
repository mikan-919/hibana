use std -> *
use './html.hb'

struct Html(String)
struct Attribute { name: String, value: String }
fn text(value: String) -> Html { Html(html.escape(value)) }
fn attribute(name: String, value: String) -> Attribute { Attribute { name: replace(name, '_', '-'), value: value } }
fn render(value: Html) -> String { value.0 }
fn fragment(children: Vec<Html>) -> Html {
    let mut result = ''
    for child in children { result = result + child.0 }
    Html(result)
}
fn each<A>(values: Vec<A>, child: (A) -> Html) -> Html { fragment(map(values, child)) }
fn element(tag: String, attributes: Vec<Attribute>, children: Vec<Html>) -> Html {
    let mut result = '<' + tag
    for attr in attributes { result = result + ' ' + attr.name + '="' + html.escape(attr.value) + '"' }
    result = result + '>'
    if tag != 'input' && tag != 'br' && tag != 'hr' && tag != 'img' {
        result = result + fragment(children).0 + '</' + tag + '>'
    }
    Html(result)
}
syntax ViewAttribute = name:Ident '=' value:Expr | name:String '=' value:Expr
macro View('view' tag:Ident '(' attrs:(ViewAttribute % ',') ')' '{' children:(Expr % ';') '}') {
    quote { element(${str_of(tag)}, [$(attribute(${str_of(attrs.name)}, $attrs.value)),*], [$($children),*]) }
}
macro ViewPlain('view' tag:Ident '{' children:(Expr % ';') '}') {
    quote { element(${str_of(tag)}, [], [$($children),*]) }
}
