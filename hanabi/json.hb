use std -> *

trait ToJson { fn to_json(self) -> Json }
impl ToJson for int { fn to_json(self) -> Json { JInt(self) } }
impl ToJson for String { fn to_json(self) -> Json { JStr(self) } }
impl ToJson for bool { fn to_json(self) -> Json { JBool(self) } }
impl ToJson for Json { fn to_json(self) -> Json { self } }
impl<A: ToJson> ToJson for Vec<A> {
    fn to_json(self) -> Json { JArr(map(self, fn(value) { value.to_json() })) }
}

syntax JsonField = name:Ident ':' value:Expr
macro JsonObject('object' '{' fields:(JsonField % ',') '}') {
    quote { JObj([$(Field { key: ${str_of(fields.name)}, value: to_json($fields.value) }),*]) }
}
