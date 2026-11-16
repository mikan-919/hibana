use std -> *
use './types.hb' -> *

extern 'hibana:native' {
    fn listen(port: Port) -> Socket
    fn accept(fd: Socket) -> Socket
    fn close(fd: Socket)
    fn read(fd: Socket, maximum: ReadSize) -> Result<Bytes, String>
    fn write(fd: Socket, bytes: Bytes) -> Result<unit, String>
    fn slice(bytes: Bytes, start: int[n -> 0 <= n && n <= len(bytes)], end: int[n -> start <= n && n <= len(bytes)]) -> Bytes[result -> len(result) == end - start]
    fn join(left: Bytes, right: Bytes) -> Bytes[result -> len(result) == len(left) + len(right)]
    fn read_file(path: FilePath) -> Result<Option<Bytes>, String>
    fn write_file(path: FilePath, bytes: Bytes) -> Result<unit, String>
    fn mkdir(path: FilePath) -> Result<unit, String>
    fn utf8(text: String) -> Bytes
    fn decode(bytes: Bytes) -> String
    fn valid_utf8(bytes: Bytes) -> bool
}
