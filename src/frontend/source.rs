//! ファイル単位のソースと位置。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub struct SourceFile {
    pub id: SourceId,
    pub name: String,
    pub text: String,
}

impl AsRef<SourceFile> for SourceFile {
    fn as_ref(&self) -> &SourceFile {
        self
    }
}
