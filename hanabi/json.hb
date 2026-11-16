use std -> *

trait Codec {
    fn encode(self: Self) -> Json
    fn decode(value: Json) -> Result<Self, String>
    fn decode_path(value: Json) -> Result<Self, String> { decode<Self>(value) }
}
impl Codec for String {
    fn encode(self: Self) -> Json { JStr(self) }
    fn decode(value: Json) -> Result<String, String> { match value { JStr(s) -> Ok(s) _ -> Err('文字列が必要') } }
}
impl Codec for int {
    fn encode(self: Self) -> Json { JInt(self) }
    fn decode(value: Json) -> Result<int, String> { match value { JInt(n) -> Ok(n) _ -> Err('整数が必要') } }
    fn decode_path(value: Json) -> Result<int, String> { match value { JStr(s) -> parse_int(s) _ -> decode<int>(value) } }
}
impl Codec for bool {
    fn encode(self: Self) -> Json { JBool(self) }
    fn decode(value: Json) -> Result<bool, String> { match value { JBool(v) -> Ok(v) _ -> Err('真偽値が必要') } }
}
impl Codec for unit {
    fn encode(self: Self) -> Json { JNull }
    fn decode(value: Json) -> Result<unit, String> { match value { JNull -> Ok({}) _ -> Err('null が必要') } }
}
impl<A: Codec> Codec for Vec<A> {
    fn encode(self: Self) -> Json { JArr(map(self, fn(v) { v.encode() })) }
    fn decode(value: Json) -> Result<Vec<A>, String> {
        match value {
            JArr(items) -> {
                let mut result: Vec<A> = []
                for item in items { result = push(result, decode<A>(item)?) }
                Ok(result)
            }
            _ -> Err('配列が必要')
        }
    }
}
fn required(value: Json, key: String) -> Result<Json, String> {
    match field(value, key) { Some(v) -> Ok(v) None -> Err(key + ' が必要') }
}
fn property<A: Codec>(value: Json, key: String) -> Result<A, String> {
    match decode<A>(required(value, key)?) { Ok(value) -> Ok(value) Err(e) -> Err(key + ': ' + e) }
}
fn path_property<A: Codec>(value: Json, key: String) -> Result<A, String> {
    match decode_path<A>(required(value, key)?) { Ok(value) -> Ok(value) Err(e) -> Err(key + ': ' + e) }
}
fn parse<A: Codec>(text: String) -> Result<A, String> {
    match parse_json(text) { Some(v) -> decode<A>(v) None -> Err('JSON を読めない') }
}
fn print<A: Codec>(value: A) -> String { stringify(value.encode()) }

// 宣言したフィールドから型・符号化・復号をまとめて生成する。
syntax WireField = name:Ident ':' ty:Type
macro Record('record' name:Ident '{' fields:(WireField % ',') '}') -> Decl {
    quote {
        struct $name { $($fields.name: $fields.ty),* }
        impl Codec for $name {
            fn encode(self: $name) -> Json { JObj([$(Field { key: ${str_of(fields.name)}, value: encode(self.$fields.name) }),*]) }
            fn decode(value: Json) -> Result<$name, String> {
                Ok($name { $($fields.name: property<$fields.ty>(value, ${str_of(fields.name)})?),* })
            }
            fn decode_path(value: Json) -> Result<$name, String> {
                Ok($name { $($fields.name: path_property<$fields.ty>(value, ${str_of(fields.name)})?),* })
            }
        }
    }
}
// 条件を持つスカラーは別型にし、静的な構築でも動的な復号でも同じ条件を要求する。
macro Scalar('scalar' name:Ident '(' ty:Type ')') -> Decl {
    let spelling = str_of(ty)
    let base = if starts_with(spelling, 'int') { ident('int') } else if starts_with(spelling, 'String') { ident('String') } else { error('scalar の基底型は int か String') }
    quote {
        struct $name($ty)
        impl Codec for $name {
            fn encode(self: $name) -> Json { self.0.encode() }
            fn decode(value: Json) -> Result<$name, String> { Ok($name(refine<$ty>(decode<$base>(value)?)?)) }
            fn decode_path(value: Json) -> Result<$name, String> { Ok($name(refine<$ty>(decode_path<$base>(value)?)?)) }
        }
    }
}
syntax WireCase = name:Ident '(' ty:Type ')'
macro Union('union' name:Ident '{' cases:(WireCase % ',') '}') -> Decl {
    quote {
        enum $name { $($cases.name($cases.ty)),* }
        impl Codec for $name {
            fn encode(self: $name) -> Json { match self { $($cases.name(payload) -> JObj([Field { key: 'tag', value: JStr(${str_of(cases.name)}) }, Field { key: 'value', value: payload.encode() }])),* } }
            fn decode(value: Json) -> Result<$name, String> {
                let tag = decode<String>(required(value, 'tag')?)?
                let payload = required(value, 'value')?
                match tag { $(${str_of(cases.name)} -> Ok($cases.name(decode<$cases.ty>(payload)?))),* _ -> Err('未知のメッセージ: ' + tag) }
            }
        }
    }
}
