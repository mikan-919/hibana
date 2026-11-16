use std
// hibana:native の接続。Linux x86-64 のシステムコールを使う
fn __native_path(path: String) -> int {
    let p = __payload(path)
    let n = __load(p)
    let q = __alloc(n + 1)
    let mut i = 0
    while i < n {
        let b = __load8(p + 8 + i)
        if b == 0 { __die('パスに NUL は使えない') }
        __store8(q + i, b)
        i = i + 1
    }
    __store8(q + n, 0)
    q
}
fn __native_listen(port: int) -> int {
    if port < 1 || port > 65535 { __die('ポート番号は1から65535') }
    // 切断済みの接続への送信は Result の失敗として返す。
    let signal = __alloc(32)
    __store(signal, 1); __store(signal + 8, 0); __store(signal + 16, 0); __store(signal + 24, 0)
    __syscall(13, 13, signal, 0, 8, 0)
    let fd = __syscall(41, 2, 1, 0, 0, 0)
    if fd < 0 { __die('ソケットを作成できない') }
    let address = __alloc(16)
    __store(address, 0); __store(address + 8, 0)
    __store8(address, 2)
    __store8(address + 2, port / 256); __store8(address + 3, port % 256)
    __store8(address + 4, 127); __store8(address + 7, 1)
    let enabled = __alloc(8)
    __store(enabled, 1)
    __syscall(54, fd, 1, 2, enabled, 4)
    if __syscall(49, fd, address, 16, 0, 0) < 0 { __die('ポートを使用できない') }
    if __syscall(50, fd, 128, 0, 0, 0) < 0 { __die('接続を待ち受けられない') }
    fd
}
fn __native_accept(fd: int) -> int {
    let mut result = 0 - 4
    while result == 0 - 4 { result = __syscall(43, fd, 0, 0, 0, 0) }
    if result < 0 { __die('接続を受け付けられない') }
    result
}
fn __native_close(fd: int) { __syscall(3, fd, 0, 0, 0, 0) }
fn __native_array(p: int, n: int) -> Vec<int> {
    let array = __arr_new(n)
    let mut i = 0
    while i < n { __setv(array + 8 + i * 16, __load8(p + i)); i = i + 1 }
    __mkv(4, array)
}
fn __native_buffer(bytes: Vec<int>) -> int {
    let p = __payload(bytes)
    let n = __load(p)
    let buffer = __alloc(n)
    let mut i = 0
    while i < n {
        let value = __payload(__getv(p + 8 + i * 16))
        if value < 0 || value > 255 { __die('バイトは0から255') }
        __store8(buffer + i, value)
        i = i + 1
    }
    buffer
}
fn __native_utf8(text: String) -> Vec<int> {
    let p = __payload(text)
    __native_array(p + 8, __load(p))
}
fn __native_slice(bytes: Vec<int>, start: int, end: int) -> Vec<int> {
    let p = __payload(bytes)
    if start < 0 || end < start || end > __load(p) { __die('バイト列の範囲外') }
    let array = __arr_new(end - start)
    __copy(array + 8, p + 8 + start * 16, (end - start) * 16)
    __mkv(4, array)
}
fn __native_join(left: Vec<int>, right: Vec<int>) -> Vec<int> {
    let a = __payload(left)
    let b = __payload(right)
    let na = __load(a)
    let nb = __load(b)
    let array = __arr_new(na + nb)
    __copy(array + 8, a + 8, na * 16)
    __copy(array + 8 + na * 16, b + 8, nb * 16)
    __mkv(4, array)
}
fn __native_decode(bytes: Vec<int>) -> String {
    let n = __load(__payload(bytes))
    let p = __native_buffer(bytes)
    if !__utf8_valid(p, n) { __die('文字コードが UTF-8 でない') }
    __bytes(p, n)
}
fn __native_valid_utf8(bytes: Vec<int>) -> bool {
    __utf8_valid(__native_buffer(bytes), __load(__payload(bytes)))
}
fn __native_read(fd: int, maximum: int) -> int {
    if maximum < 1 || maximum > 1048576 { __die('読み取りの上限は1から1048576バイト') }
    let p = __alloc(maximum)
    let mut n = 0 - 4
    while n == 0 - 4 { n = __sys_read(fd, p, maximum) }
    if n < 0 { return std.Err('読み取れない') }
    std.Ok(__native_array(p, n))
}
fn __native_write(fd: int, bytes: Vec<int>) -> int {
    let p = __native_buffer(bytes)
    let n = __load(__payload(bytes))
    let mut i = 0
    while i < n {
        let written = __syscall(1, fd, p + i, n - i, 0, 0)
        if written != 0 - 4 {
            if written <= 0 { return std.Err('書き込めない') }
            i = i + written
        }
    }
    std.Ok({})
}
fn __native_read_file(path: String) -> int {
    let fd = __syscall(2, __native_path(path), 0, 0, 0, 0)
    if fd == 0 - 2 || fd == 0 - 20 { return std.Ok(std.None) }
    if fd < 0 { return std.Err('ファイルを開けない') }
    let mut capacity = 8192
    let mut buffer = __alloc(capacity)
    let mut size = 0
    let mut done = false
    while !done {
        if size == capacity {
            if capacity >= 1048576 { __native_close(fd); return std.Err('ファイルは1MiB未満') }
            capacity = capacity * 2
            let next = __alloc(capacity)
            __copy(next, buffer, size)
            buffer = next
        }
        let n = __sys_read(fd, buffer + size, capacity - size)
        if n < 0 && n != 0 - 4 { __native_close(fd); return std.Err('ファイルを読み取れない') }
        if n == 0 { done = true }
        else if n > 0 { size = size + n }
    }
    __native_close(fd)
    std.Ok(std.Some(__native_array(buffer, size)))
}
fn __native_write_file(path: String, bytes: Vec<int>) -> int {
    if __load(__payload(bytes)) >= 1048576 { return std.Err('保存できるデータは1MiB未満') }
    let temp = __cat(path, '.tmp')
    let fd = __syscall(2, __native_path(temp), 577, 384, 0, 0)
    if fd < 0 { return std.Err('保存用のファイルを作成できない') }
    let p = __native_buffer(bytes)
    let n = __load(__payload(bytes))
    let mut i = 0
    while i < n {
        let written = __syscall(1, fd, p + i, n - i, 0, 0)
        if written != 0 - 4 {
            if written <= 0 { __native_close(fd); return std.Err('ファイルを書き込めない') }
            i = i + written
        }
    }
    if __syscall(74, fd, 0, 0, 0, 0) < 0 { __native_close(fd); return std.Err('ファイルを同期できない') }
    __native_close(fd)
    if __syscall(82, __native_path(temp), __native_path(path), 0, 0, 0) < 0 { return std.Err('保存したファイルを置き換えられない') }
    std.Ok({})
}
fn __native_mkdir(path: String) -> int {
    let result = __syscall(83, __native_path(path), 448, 0, 0, 0)
    if result != 0 && result != 0 - 17 { return std.Err('保存先を作成できない') }
    std.Ok({})
}
