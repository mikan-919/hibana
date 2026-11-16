// hanabi のネイティブ実行環境との接続。アプリからは server と storage を使う
extern 'hibana:native' {
    fn listen(port: int) -> int
    fn accept(fd: int) -> int
    fn close(fd: int)
    fn fork() -> int
    fn wait(pid: int)
    fn exit(code: int) -> never
    fn read(fd: int, maximum: int) -> Vec<int>
    fn write(fd: int, bytes: Vec<int>)
    fn slice(bytes: Vec<int>, start: int, end: int) -> Vec<int>
    fn join(left: Vec<int>, right: Vec<int>) -> Vec<int>
    fn read_file(path: String) -> Vec<int>
    fn write_file(path: String, bytes: Vec<int>)
    fn mkdir(path: String)
    fn utf8(text: String) -> Vec<int>
    fn decode(bytes: Vec<int>) -> String
    fn valid_utf8(bytes: Vec<int>) -> bool
}
