// パッケージ管理。hibana.toml の [dependencies] を git から取り、hibana.lock にコミットを記録する
// ponytail: 同じ名前のパッケージは一つの版だけ。版の範囲の解決はレジストリを作るときに足す
use std::path::{Path, PathBuf};
use std::process::Command;
use toml::Table;

pub const MANIFEST: &str = "hibana.toml";
const LOCK: &str = "hibana.lock";

// dir から上へたどって hibana.toml のあるディレクトリ
pub fn root(dir: &Path) -> Option<PathBuf> {
    let dir = dir.canonicalize().ok()?;
    dir.ancestors().find(|d| d.join(MANIFEST).is_file()).map(Path::to_path_buf)
}

fn read(p: &Path) -> Result<Table, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    s.parse().map_err(|e| format!("{}: {e}", p.display()))
}

fn cache() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
    base.join("hibana")
}

fn checkout(name: &str, rev: &str) -> PathBuf {
    cache().join(format!("{name}-{rev}"))
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !o.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

// 依存の (名前, git の URL, 指定した版（tag・branch・rev、無ければ既定のブランチ）)
fn deps(manifest: &Table) -> Vec<(String, String, Option<String>)> {
    let Some(ds) = manifest.get("dependencies").and_then(|d| d.as_table()) else {
        return vec![];
    };
    ds.iter()
        .filter_map(|(n, d)| {
            let git = d.get("git")?.as_str()?.to_string();
            let rev = ["tag", "branch", "rev"]
                .iter()
                .find_map(|k| d.get(k).and_then(|v| v.as_str()).map(String::from));
            Some((n.clone(), git, rev))
        })
        .collect()
}

// git の URL を取ってきて、rev（無ければ既定のブランチの先頭）のコミットを返す
fn clone(name: &str, url: &str, rev: Option<&str>) -> Result<String, String> {
    let tmp = cache().join(format!("{name}-tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    // ponytail: 履歴ごと取る。大きいリポジトリで遅いなら --depth 1 と fetch <sha> にする
    git(&tmp, &["clone", "-q", url, "."])?;
    if let Some(r) = rev {
        git(&tmp, &["checkout", "-q", r])?;
    }
    let sha = git(&tmp, &["rev-parse", "HEAD"])?;
    let dst = checkout(name, &sha);
    if dst.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    } else {
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    }
    Ok(sha)
}

// root の依存を（依存の依存も）取ってきて、hibana.lock を書く。ロックにあるコミットはそのまま使う
pub fn fetch(root: &Path) -> Result<(), String> {
    let lock_path = root.join(LOCK);
    let old = if lock_path.is_file() { read(&lock_path)? } else { Table::new() };
    let mut lock = Table::new();
    let mut todo = deps(&read(&root.join(MANIFEST))?);
    while let Some((name, url, rev)) = todo.pop() {
        if let Some(t) = lock.get(&name) {
            if t.get("git").and_then(|g| g.as_str()) != Some(&url) {
                return Err(format!("パッケージ {name} が別の git から二つ要る（{url}）"));
            }
            continue;
        }
        let locked = old
            .get(&name)
            .filter(|t| t.get("git").and_then(|g| g.as_str()) == Some(&url))
            .and_then(|t| t.get("rev")?.as_str().map(String::from));
        let sha = match locked {
            Some(sha) if checkout(&name, &sha).is_dir() => sha,
            Some(sha) => clone(&name, &url, Some(&sha))?,
            None => {
                eprintln!("取得: {name} ({url})");
                clone(&name, &url, rev.as_deref())?
            }
        };
        let m = checkout(&name, &sha).join(MANIFEST);
        if m.is_file() {
            todo.extend(deps(&read(&m)?));
        }
        let mut t = Table::new();
        t.insert("git".into(), url.into());
        t.insert("rev".into(), sha.into());
        lock.insert(name, t.into());
    }
    if lock != old {
        std::fs::write(&lock_path, lock.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// use name の入口のファイル（取ってきたパッケージの src/lib.hb）
pub fn entry(dir: &Path, name: &str) -> Option<PathBuf> {
    let lock = read(&root(dir)?.join(LOCK)).ok()?;
    let sha = lock.get(name)?.get("rev")?.as_str()?;
    Some(checkout(name, sha).join("src/lib.hb"))
}

// hibana add <git の URL> [tag]: 名前は URL の最後の部分（hibana- は除く）
pub fn add(root: &Path, url: &str, tag: Option<&str>) -> Result<String, String> {
    let path = root.join(MANIFEST);
    let mut m = read(&path)?;
    let last = url.trim_end_matches('/').trim_end_matches(".git").rsplit('/').next().unwrap_or(url);
    let name = crate::parser::module_name(last.strip_prefix("hibana-").unwrap_or(last));
    let mut d = Table::new();
    d.insert("git".into(), url.into());
    if let Some(t) = tag {
        d.insert("tag".into(), t.into());
    }
    let ds = m.entry("dependencies").or_insert(Table::new().into());
    ds.as_table_mut().ok_or("[dependencies] が表でない")?.insert(name.clone(), d.into());
    std::fs::write(&path, m.to_string()).map_err(|e| e.to_string())?;
    fetch(root)?;
    Ok(name)
}

// hibana new <名前>: hibana.toml と src/main.hb を作る
pub fn new(name: &str) -> Result<(), String> {
    let dir = Path::new(name);
    if dir.exists() {
        return Err(format!("{name} は既にある"));
    }
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    let pkg = crate::parser::module_name(name);
    std::fs::write(dir.join(MANIFEST), format!("[package]\nname = \"{pkg}\"\nversion = \"0.1.0\"\n\n[dependencies]\n"))
        .map_err(|e| e.to_string())?;
    std::fs::write(dir.join("src/main.hb"), "use std.*\n\nfn main() {\n    print(\"hello\")\n}\n")
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ローカルの git リポジトリをパッケージにして、取得・ロック・入口の解決を確かめる
    #[test]
    fn fetch_and_entry() {
        let t = std::env::temp_dir().join(format!("hibana-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (lib, app) = (t.join("hibana-greet"), t.join("app"));
        std::fs::create_dir_all(lib.join("src")).unwrap();
        std::fs::write(lib.join("src/lib.hb"), "fn hi() -> str { \"hi\" }\n").unwrap();
        for a in [&["init", "-q"][..], &["add", "."], &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x"]] {
            git(&lib, a).unwrap();
        }
        unsafe { std::env::set_var("XDG_CACHE_HOME", t.join("cache")) };
        new(app.to_str().unwrap()).unwrap();
        let url = format!("file://{}", lib.display());
        assert_eq!(add(&app, &url, None).unwrap(), "greet");
        let e = entry(&app.join("src"), "greet").unwrap();
        assert!(std::fs::read_to_string(e).unwrap().contains("fn hi"));
        let lock = std::fs::read_to_string(app.join(LOCK)).unwrap();
        // もう一度取ってもロックは変わらない
        fetch(&app).unwrap();
        assert_eq!(std::fs::read_to_string(app.join(LOCK)).unwrap(), lock);
        let _ = std::fs::remove_dir_all(&t);
    }
}
