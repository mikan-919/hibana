use std -> *
use './types.hb' -> *
use './native.hb'

trait Storage {
    fn load(self, path: FilePath) -> Result<Option<Json>, String>
    fn save(self, path: FilePath, value: Json) -> Result<unit, String>
    fn asset(self, path: FilePath) -> Result<Option<Bytes>, String>
    fn directory(self, path: FilePath) -> Result<unit, String>
}
struct NativeStorage {}
impl Storage for NativeStorage {
    fn load(self, path: FilePath) -> Result<Option<Json>, String> {
        match native.read_file(path)? {
            None -> Ok(None)
            Some(bytes) -> {
                if !native.valid_utf8(bytes) { return Err('保存内容が UTF-8 でない') }
                match parse_json(native.decode(bytes)) { Some(v) -> Ok(Some(v)) None -> Err('保存した JSON を読めない') }
            }
        }
    }
    fn save(self, path: FilePath, value: Json) -> Result<unit, String> { native.write_file(path, native.utf8(stringify(value))) }
    fn asset(self, path: FilePath) -> Result<Option<Bytes>, String> { native.read_file(path) }
    fn directory(self, path: FilePath) -> Result<unit, String> { native.mkdir(path) }
}
enum Content { JsonFile(Json), BinaryFile(Bytes) }
struct Stored { path: FilePath, content: Content }
struct MemoryStorage { files: Ref<Vec<Stored>> }
impl Storage for MemoryStorage {
    fn load(self, path: FilePath) -> Result<Option<Json>, String> {
        for file in read(self.files) {
            if file.path == path { return match file.content { JsonFile(v) -> Ok(Some(v)) BinaryFile(_) -> Err('JSON ファイルでない') } }
        }
        Ok(None)
    }
    fn save(self, path: FilePath, value: Json) -> Result<unit, String> {
        let mut files: Vec<Stored> = []
        for file in read(self.files) { if file.path != path { files = push(files, file) } }
        write(self.files, push(files, Stored { path: path, content: JsonFile(value) }))
        Ok({})
    }
    fn asset(self, path: FilePath) -> Result<Option<Bytes>, String> {
        for file in read(self.files) {
            if file.path == path { return match file.content { BinaryFile(v) -> Ok(Some(v)) JsonFile(_) -> Err('バイナリファイルでない') } }
        }
        Ok(None)
    }
    fn directory(self, path: FilePath) -> Result<unit, String> { Ok({}) }
}
