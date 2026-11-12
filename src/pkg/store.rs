//! ファイル内容で重複を排除する、変更不可のパッケージストア。
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn io_error(path: &Path, error: std::io::Error) -> String {
    format!("{}: {error}", path.display())
}

/// 新しい一時ツリーへ取り込む。後始末と最終配置への公開は呼び出し側が行う。
/// ハードリンクの変更はストアにも反映されるため、ファイルは変更しない。
pub(super) fn import_tree(source: &Path, destination: &Path, data_root: &Path) -> Result<(), String> {
    let source = source.canonicalize().map_err(|e| io_error(source, e))?;
    let objects = data_root.join("store/v1/files");
    fs::create_dir_all(&objects).map_err(|e| io_error(&objects, e))?;
    fs::create_dir(destination).map_err(|e| io_error(destination, e))?;
    import_dir(&source, &source, destination, &objects)
}

fn import_dir(root: &Path, source: &Path, destination: &Path, objects: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|e| io_error(source, e))? {
        let entry = entry.map_err(|e| io_error(source, e))?;
        // 入れ子のリポジトリも含め、Git の管理情報は取り込まない。
        if entry.file_name() == ".git" {
            continue;
        }
        let source = entry.path();
        let destination = destination.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| io_error(&source, e))?;
        if kind.is_dir() {
            fs::create_dir(&destination).map_err(|e| io_error(&destination, e))?;
            import_dir(root, &source, &destination, objects)?;
        } else if kind.is_file() {
            import_file(&source, &destination, objects)?;
        } else if kind.is_symlink() {
            import_symlink(root, &source, &destination)?;
        } else {
            return Err(format!("パッケージに通常ファイル以外がある: {}", source.display()));
        }
    }
    Ok(())
}

fn import_file(source: &Path, destination: &Path, objects: &Path) -> Result<(), String> {
    let mut input = File::open(source).map_err(|e| io_error(source, e))?;
    let metadata = input.metadata().map_err(|e| io_error(source, e))?;
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    // 保存したバイト列とハッシュが一致するよう、書き込みと計算を同時に行う。
    let (temp, mut output) = loop {
        let path = objects.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break (Temp(path), file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_error(&path, e)),
        }
    };
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|e| io_error(source, e))?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count]).map_err(|e| io_error(&temp.0, e))?;
        hash.update(&buffer[..count]);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output
            .set_permissions(fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }))
            .map_err(|e| io_error(&temp.0, e))?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        output.set_permissions(permissions).map_err(|e| io_error(&temp.0, e))?;
    }
    drop(output);
    let digest = format!("{:x}", hash.finalize());
    let shard = objects.join(&digest[..2]);
    fs::create_dir_all(&shard).map_err(|e| io_error(&shard, e))?;
    let object = shard.join(format!("{}-{}", &digest[2..], if executable { "x" } else { "n" }));
    // 置き換えずに公開する。同時取得で同じオブジェクトが先に公開されてもよい。
    match fs::hard_link(&temp.0, &object) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(io_error(&object, e)),
    }
    fs::hard_link(&object, destination).map_err(|e| io_error(destination, e))
}

fn import_symlink(root: &Path, source: &Path, destination: &Path) -> Result<(), String> {
    let target = fs::read_link(source).map_err(|e| io_error(source, e))?;
    if target.is_absolute() {
        return Err(format!(
            "パッケージ内の絶対シンボリックリンクは使えない: {}",
            source.display()
        ));
    }
    // 一度パッケージ外へ出てから戻るリンクも、移動後は元の取得先を指すので拒否する。
    let mut depth = source
        .parent()
        .unwrap()
        .strip_prefix(root)
        .unwrap()
        .components()
        .count();
    for component in target.components() {
        match component {
            Component::ParentDir if depth == 0 => {
                return Err(format!("パッケージ外へのシンボリックリンク: {}", source.display()));
            }
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            _ => (),
        }
    }
    let resolved = source.canonicalize().map_err(|e| io_error(source, e))?;
    let relative = resolved
        .strip_prefix(root)
        .map_err(|_| format!("パッケージ外へのシンボリックリンク: {}", source.display()))?;
    if relative
        .components()
        .any(|c| matches!(c, Component::Normal(name) if name == ".git"))
        || target
            .components()
            .any(|c| matches!(c, Component::Normal(name) if name == ".git"))
    {
        return Err(format!(".git へのシンボリックリンクは使えない: {}", source.display()));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, destination).map_err(|e| io_error(destination, e))
    }
    #[cfg(not(unix))]
    {
        let _ = destination;
        Err(format!(
            "この環境ではパッケージのシンボリックリンクに未対応: {}",
            source.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "hibana-cas-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn source(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir(&path).unwrap();
            path
        }
        fn import(&self, source: &Path, name: &str) -> PathBuf {
            let destination = self.0.join(name);
            import_tree(source, &destination, &self.0.join("data")).unwrap();
            destination
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn imports_files_and_skips_git_metadata() {
        let s = Sandbox::new();
        let source = s.source("source");
        fs::create_dir_all(source.join("src/.git")).unwrap();
        fs::write(source.join("src/lib.hb"), "hello").unwrap();
        fs::write(source.join("src/.git/config"), "not package content").unwrap();
        fs::write(source.join(".git"), "gitdir: elsewhere").unwrap();
        let dst = s.import(&source, "package");
        assert_eq!(fs::read(dst.join("src/lib.hb")).unwrap(), b"hello");
        assert!(!dst.join(".git").exists());
        assert!(!dst.join("src/.git").exists());
        assert!(fs::metadata(dst.join("src/lib.hb")).unwrap().permissions().readonly());
        assert!(import_tree(&source, &dst, &s.0.join("data")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn deduplicates_content_across_packages_and_versions() {
        use std::os::unix::fs::MetadataExt;
        let s = Sandbox::new();
        let first = s.source("v1");
        let second = s.source("v2");
        fs::write(first.join("a"), "same").unwrap();
        fs::write(first.join("b"), "same").unwrap();
        fs::write(second.join("a"), "same").unwrap();
        fs::write(second.join("b"), "changed").unwrap();
        let a = s.import(&first, "p1");
        let b = s.import(&second, "p2");
        let inode = |p| fs::metadata(p).unwrap().ino();
        assert_eq!(inode(a.join("a")), inode(a.join("b")));
        assert_eq!(inode(a.join("a")), inode(b.join("a")));
        assert_ne!(inode(a.join("b")), inode(b.join("b")));
        // 取得元はストアへのハードリンクではないため、変更しても影響しない。
        fs::write(first.join("a"), "modified source").unwrap();
        assert_eq!(fs::read(a.join("a")).unwrap(), b"same");
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit_is_part_of_identity() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let s = Sandbox::new();
        let source = s.source("source");
        fs::write(source.join("plain"), "same").unwrap();
        fs::write(source.join("exec"), "same").unwrap();
        fs::set_permissions(source.join("exec"), fs::Permissions::from_mode(0o755)).unwrap();
        let dst = s.import(&source, "package");
        let plain = fs::metadata(dst.join("plain")).unwrap();
        let exec = fs::metadata(dst.join("exec")).unwrap();
        assert_ne!(plain.ino(), exec.ino());
        assert_eq!(plain.permissions().mode() & 0o777, 0o444);
        assert_eq!(exec.permissions().mode() & 0o777, 0o555);
    }

    #[cfg(unix)]
    #[test]
    fn preserves_internal_symlinks() {
        use std::os::unix::fs::symlink;
        let s = Sandbox::new();
        let source = s.source("source");
        fs::create_dir(source.join("src")).unwrap();
        fs::write(source.join("src/lib.hb"), "hello").unwrap();
        symlink("src/lib.hb", source.join("lib.hb")).unwrap();
        symlink("src", source.join("alias")).unwrap();
        let dst = s.import(&source, "package");
        assert_eq!(fs::read_link(dst.join("lib.hb")).unwrap(), Path::new("src/lib.hb"));
        assert_eq!(fs::read(dst.join("alias/lib.hb")).unwrap(), b"hello");
        // 最終的に取得元の内部へ戻っても、外へ出る相対リンクは移動に耐えない。
        symlink("../source/src/lib.hb", source.join("roundtrip")).unwrap();
        assert!(import_tree(&source, &s.0.join("unsafe"), &s.0.join("data")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unsafe_symlinks() {
        use std::os::unix::fs::symlink;
        let s = Sandbox::new();
        fs::write(s.0.join("outside"), "secret").unwrap();
        for (i, target) in [
            PathBuf::from("../outside"),
            s.0.join("outside"),
            PathBuf::from("missing"),
            PathBuf::from(".git/config"),
        ]
        .into_iter()
        .enumerate()
        {
            let source = s.source(&format!("source{i}"));
            fs::create_dir(source.join(".git")).unwrap();
            fs::write(source.join(".git/config"), "secret").unwrap();
            symlink(target, source.join("link")).unwrap();
            assert!(import_tree(&source, &s.0.join(format!("out{i}")), &s.0.join("data")).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_imports_share_complete_objects() {
        use std::os::unix::fs::MetadataExt;
        let s = Sandbox::new();
        let source = s.source("source");
        fs::write(source.join("file"), vec![42; 256 * 1024]).unwrap();
        std::thread::scope(|scope| {
            for i in 0..4 {
                let source = &source;
                let root = &s.0;
                scope.spawn(move || import_tree(source, &root.join(format!("p{i}")), &root.join("data")).unwrap());
            }
        });
        let first = fs::metadata(s.0.join("p0/file")).unwrap().ino();
        for i in 0..4 {
            let file = s.0.join(format!("p{i}/file"));
            assert_eq!(fs::metadata(&file).unwrap().ino(), first);
            assert_eq!(fs::read(file).unwrap(), vec![42; 256 * 1024]);
        }
    }
}
