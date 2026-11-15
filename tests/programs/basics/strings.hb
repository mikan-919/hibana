// 文字列の連結・長さ・分割・部分文字列と、for ループ
fn greet(name: String) -> String { 'Hello, ' + name + '!' }

fn main() {
    put(greet('hibana'))
    let parts = split('/user/42/posts', '/')
    put(len(parts))
    // for x in Vec
    for p in parts {
        if len(p) > 0 { put(p) }
    }
    // for i in a..b では 0 <= i < len(parts) が分かるので、添字の証明が要らない
    let mut joined = ''
    for i in 0..len(parts) {
        joined = joined + '[' + parts[i] + ']'
    }
    put(joined)
    put(split('', ''))
    put(split('abc', ''))
    put(split('aé🙂', ''))
    put(split('a,,b', ','))
    put(split('', ','))
    put(slice('hibana', 0, 3) + to_str(1 + 2) + to_str(starts_with('hibana', 'hi')))
    // a && b の b では a が真と分かっている
    if len(parts) > 2 && parts[2] == '42' { put('id is 42') }
}
