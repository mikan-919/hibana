use std -> *
use './native.hb'

fn read_json(path: String) -> Result<Json, String> {
    let bytes = native.read_file(path)
    if len(bytes) == 0 { return Ok(JNull) }
    if !native.valid_utf8(bytes) { return Err('保存した文字コードが UTF-8 でない') }
    match parse_json(native.decode(bytes)) { Some(value) -> Ok(value) None -> Err('保存した JSON を読めない') }
}
fn write_json(path: String, value: Json) -> Result<unit, String> {
    let bytes = native.utf8(stringify(value))
    if len(bytes) >= 1048576 { return Err('保存できるデータは1MiB未満') }
    native.write_file(path, bytes)
    Ok({})
}
