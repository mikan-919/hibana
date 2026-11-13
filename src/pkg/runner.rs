// Hibana の実行パッケージを共通ストアから解決し、書き込み可能な一時ツリーで動かす。
use super::*;

fn resolve(data: &Path, url: &str, rev: Option<&str>) -> Result<PathBuf, String> {
    let key = format!("{:x}", Sha256::digest(format!("{url}\0{}", rev.unwrap_or(""))));
    let reference = data.join("dlx/v1").join(format!("{key}.toml"));
    if reference.is_file() {
        let saved = read(&reference)?;
        if let Some(sha) = saved.get("rev").and_then(|v| v.as_str()) {
            let tree = checkout(data, url, sha);
            if tree.is_dir() {
                return Ok(tree);
            }
        }
    }
    eprintln!("取得: {url}");
    let sha = clone(data, url, rev)?;
    let tree = checkout(data, url, &sha);
    validate(&tree)?;
    let tmp = TempDir::new(data)?;
    let pending = tmp.0.join("reference.toml");
    let mut saved = Table::new();
    saved.insert("rev".into(), sha.into());
    std::fs::write(&pending, saved.to_string()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(reference.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(&pending, reference).map_err(|e| e.to_string())?;
    Ok(tree)
}

fn validate(tree: &Path) -> Result<(), String> {
    if !tree.join(MANIFEST).is_file() {
        return Err(format!("Hibana パッケージに {MANIFEST} がない: {}", tree.display()));
    }
    if !tree.join("src/main.hb").is_file() {
        return Err(format!("Hibana パッケージに src/main.hb がない: {}", tree.display()));
    }
    Ok(())
}

// lock は CAS のハードリンクを切ってから書く。元のロックがあれば固定した依存の版を引き継ぐ。
fn prepare(source: &Path, destination: &Path, data: &Path, cached_lock: Option<&Path>) -> Result<(), String> {
    store::import_tree(source, destination, data)?;
    let lock = destination.join(LOCK);
    let previous = cached_lock.filter(|p| p.is_file()).unwrap_or(&lock);
    let contents = if previous.is_file() {
        Some(std::fs::read(previous).map_err(|e| e.to_string())?)
    } else {
        None
    };
    if lock.exists() {
        std::fs::remove_file(&lock).map_err(|e| e.to_string())?;
    }
    if let Some(contents) = contents {
        std::fs::write(&lock, contents).map_err(|e| e.to_string())?;
    }
    fetch_in(destination, data)
}

pub fn dlx(raw: &str, rev: Option<&str>, args: &[String]) -> Result<i32, String> {
    let url = raw.strip_prefix("hibana:").unwrap_or(raw);
    if url.is_empty() || url.starts_with('-') || rev.is_some_and(str::is_empty) {
        return Err("使い方: hibana dlx [--rev 版] hibana:<git の URL> [引数...]".into());
    }
    let data = data_root()?;
    let source = resolve(&data, url, rev)?;
    validate(&source)?;
    let temp = TempDir::new(&data)?;
    let execution = temp.0.join("package");
    // 取得元のコミットごとに依存ロックを共有し、取得元がなくても再実行できるようにする。
    let key = source.file_name().ok_or("パッケージの保存先が不正")?;
    let cached_lock = data.join("dlx/v1/locks").join(key).with_extension("toml");
    prepare(&source, &execution, &data, Some(&cached_lock))?;
    if execution.join(LOCK).is_file() {
        let pending = temp.0.join("lock.toml");
        std::fs::copy(execution.join(LOCK), &pending).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(cached_lock.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(pending, cached_lock).map_err(|e| e.to_string())?;
    }
    let executable = std::env::current_exe().map_err(|e| format!("hibana: {e}"))?;
    let status = Command::new(executable)
        .arg(execution.join("src/main.hb"))
        .arg("--")
        .args(args)
        .status()
        .map_err(|e| format!("Hibana パッケージの実行: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(1)))
    }
    #[cfg(not(unix))]
    {
        Ok(status.code().unwrap_or(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_lock_is_not_a_store_hardlink() {
        let temp = TempDir::new(&std::env::temp_dir().join("hibana-runner-tests")).unwrap();
        let source = temp.0.join("source");
        let data = temp.0.join("data");
        std::fs::create_dir_all(source.join("src")).unwrap();
        std::fs::write(source.join(MANIFEST), "[dependencies]\n").unwrap();
        std::fs::write(source.join("src/main.hb"), "fn main() {}\n").unwrap();
        std::fs::write(source.join(LOCK), "[removed]\ngit = 'old'\nrev = 'old'\n").unwrap();
        let cached = temp.0.join("cached");
        store::import_tree(&source, &cached, &data).unwrap();
        let before = std::fs::read(cached.join(LOCK)).unwrap();
        let execution = temp.0.join("execution");
        prepare(&cached, &execution, &data, None).unwrap();
        assert_eq!(std::fs::read(cached.join(LOCK)).unwrap(), before);
        assert!(read(&execution.join(LOCK)).unwrap().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(
                std::fs::metadata(cached.join(LOCK)).unwrap().ino(),
                std::fs::metadata(execution.join(LOCK)).unwrap().ino()
            );
            assert_eq!(
                std::fs::metadata(cached.join("src/main.hb")).unwrap().ino(),
                std::fs::metadata(execution.join("src/main.hb")).unwrap().ino()
            );
        }
    }
}
