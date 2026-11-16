use std -> *

type Port = int[p -> 1 <= p && p <= 65535]
type Status = int[s -> 100 <= s && s < 600]
type Byte = int[b -> 0 <= b && b <= 255]
type Bytes = Vec<Byte>
type Offset = int[n -> n >= 0]
type ReadSize = int[n -> 1 <= n && n <= 1048576]
type FilePath = String[s -> len(s) > 0]
type Socket = int[n -> n >= 0]
type RequestId = int[n -> n > 0]

enum Method { Get, Post, Put, Delete }
fn parse_method(value: String) -> Result<Method, String> {
    match value { 'GET' -> Ok(Get) 'POST' -> Ok(Post) 'PUT' -> Ok(Put) 'DELETE' -> Ok(Delete) _ -> Err('HTTP メソッドが未対応') }
}
fn method_name(method: Method) -> String {
    match method { Get -> 'GET' Post -> 'POST' Put -> 'PUT' Delete -> 'DELETE' }
}
