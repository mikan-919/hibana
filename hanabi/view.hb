use std -> *
use './html.hb'
use './json.hb' -> { Codec }

struct Html(String)
struct Attribute { name: String, value: String }
trait Child { fn child(self) -> Html }
impl Child for String { fn child(self) -> Html { text(self) } }
impl Child for Html { fn child(self) -> Html { self } }
impl Child for Vec<Html> { fn child(self) -> Html { fragment(self) } }
fn text(value: String) -> Html { Html(html.escape(value)) }
fn attribute(name: String, value: String) -> Attribute { Attribute { name: name, value: value } }
trait AttributeValue { fn attribute_value(self, name: String) -> Attribute }
impl AttributeValue for String { fn attribute_value(self, name: String) -> Attribute { attribute(name, self) } }
impl AttributeValue for bool { fn attribute_value(self, name: String) -> Attribute { attribute(if self { name } else { '' }, '') } }
fn event<A: Codec>(name: String, action: A) -> Attribute { attribute('data-hanabi-' + name, stringify(action.encode())) }
fn empty_attributes() -> Vec<Attribute> { [] }
fn empty_children() -> Vec<Html> { [] }
fn render(value: Html) -> String { value.0 }
fn fragment(children: Vec<Html>) -> Html {
    let mut result = ''
    for child in children { result = result + child.0 }
    Html(result)
}
fn each<A>(values: Vec<A>, child: (A) -> Html) -> Html { fragment(map(values, child)) }
fn element(tag: String, attributes: Vec<Attribute>, children: Vec<Html>) -> Html {
    let mut result = '<' + tag
    for attr in attributes {
        if attr.name != '' { result = result + ' ' + attr.name + '="' + html.escape(attr.value) + '"' }
    }
    result = result + '>'
    if tag != 'input' && tag != 'br' && tag != 'hr' && tag != 'img' && tag != 'meta' && tag != 'link' {
        result = result + fragment(children).0 + '</' + tag + '>'
    }
    Html(result)
}
syntax AttributeName = Ident ('-' Ident)* | 'type' | 'for'
syntax JsxAttr = name:AttributeName '=' (literal:String | '{' expression:Expr '}')
syntax JsxElement = '<' ~ tag:Ident attrs:JsxAttr* '/>' | '<' ~ tag:Ident attrs:JsxAttr* '>' children:JsxChild* '</' ~ close:Ident ~ '>'
syntax JsxChild = el:JsxElement | '{' expression:Expr '}' | text:Text
macro Jsx(el:JsxElement) {
    if !is_null(el.close) && str_of(el.tag) != str_of(el.close) { error('閉じタグが一致しない: ' + str_of(el.tag)) }
    let mut attributes = quote { empty_attributes() }
    let mut ai = 0
    while ai < len(el.attrs) {
        let attr = at(el.attrs, ai)
        ai = ai + 1
        let name = str_of(attr.name)
        let value = if is_null(attr.expression) { quote { $attr.literal } } else { quote { $attr.expression } }
        let item = if name == 'onClick' || name == 'onSubmit' || name == 'onChange' {
            if !is_null(attr.literal) { error('イベントには型付きメッセージを書く') }
            let kind = if name == 'onClick' { 'click' } else if name == 'onSubmit' { 'submit' } else { 'change' }
            quote { event(${kind}, $value) }
        } else {
            if starts_with(name, 'on') { error('未対応のイベント: ' + name) }
            quote { attribute_value($value, ${name}) }
        }
        attributes = quote { push($attributes, $item) }
    }
    let mut children = quote { empty_children() }
    let mut ci = 0
    while !is_null(el.children) && ci < len(el.children) {
        let c = at(el.children, ci)
        ci = ci + 1
        let item = if !is_null(c.el) { quote { $c.el } } else if !is_null(c.expression) { quote { child($c.expression) } } else { quote { text(${str_of(c.text)}) } }
        children = quote { push($children, $item) }
    }
    quote { element(${str_of(el.tag)}, $attributes, $children) }
}
