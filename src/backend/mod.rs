//! インタプリタと各出力形式のバックエンド。
pub mod eval;
pub mod ir;
pub mod js;
pub mod native;
pub mod wasm;

// 標準ライブラリが整数の範囲超過を受け取る操作。
pub(crate) fn is_integer_overflow_arm(name: &crate::parser::Name) -> bool {
    use crate::parser::{Name, qual};
    *name == Name::associated(qual("std", "IntOverflow"), "raise_overflow".into())
}
