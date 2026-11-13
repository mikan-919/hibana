// パッケージ管理。hibana.toml の [dependencies] を git から取り、config.hibana.lock.toml にコミットを記録する
// ponytail: 同じ名前のパッケージは一つの版だけ。版の範囲の解決はレジストリを作るときに足す
mod runner;
mod store;

pub use runner::dlx;

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use toml::Table;

pub const MANIFEST: &str = "hibana.toml";
const LOCK: &str = "config.hibana.lock.toml";

// dir から上へたどって hibana.toml のあるディレクトリ
pub fn root(dir: &Path) -> Option<PathBuf> {
    let dir = dir.canonicalize().ok()?;
    dir.ancestors()
        .find(|d| d.join(MANIFEST).is_file())
        .map(Path::to_path_buf)
}

fn read(p: &Path) -> Result<Table, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    s.parse().map_err(|e| format!("{}: {e}", p.display()))
}

pub(crate) fn data_root() -> Result<PathBuf, String> {
    let base = data_home(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))?;
    Ok(base.join("hibana"))
}

fn data_home(xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    if let Some(path) = xdg.map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Ok(path);
    }
    home.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(".local/share"))
        .ok_or_else(|| "XDG_DATA_HOME または HOME に絶対パスが必要".into())
}

// 外部ツールも言語パッケージと同じファイルストアへ取り込む。
pub(crate) fn import_tool_tree(source: &Path, destination: &Path, data: &Path) -> Result<(), String> {
    store::import_tree(source, destination, data)
}

fn checkout(data: &Path, url: &str, rev: &str) -> PathBuf {
    // URL と版はパスに直接使わない。同名・同コミットでも別の取得元とは分ける。
    let id = format!("{:x}", Sha256::digest(format!("{url}\0{rev}")));
    data.join("packages/v1").join(id)
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(data: &Path) -> Result<Self, String> {
        let parent = data.join("tmp");
        std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        loop {
            let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("{}-{n}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        ));
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
fn clone(data: &Path, url: &str, rev: Option<&str>) -> Result<String, String> {
    let tmp = TempDir::new(data)?;
    let source = tmp.0.join("source");
    std::fs::create_dir(&source).map_err(|e| e.to_string())?;
    // ponytail: 取得中は履歴ごと取る。保存時には .git を除く。大きい取得は shallow fetch を検討する。
    git(&source, &["clone", "-q", "--", url, "."])?;
    if let Some(r) = rev {
        git(&source, &["checkout", "-q", "--detach", r])?;
    }
    let sha = git(&source, &["rev-parse", "HEAD"])?;
    let dst = checkout(data, url, &sha);
    if !dst.is_dir() {
        let tree = tmp.0.join("tree");
        store::import_tree(&source, &tree, data)?;
        std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(&tree, &dst) {
            // 別プロセスが同じパッケージを先に公開した場合は、その完成済みツリーを使う。
            if !dst.is_dir() {
                return Err(format!("{}: {e}", dst.display()));
            }
        }
    }
    Ok(sha)
}

// root の依存を（依存の依存も）取ってきて、config.hibana.lock.toml を書く。ロックにあるコミットはそのまま使う
pub fn fetch(root: &Path) -> Result<(), String> {
    fetch_in(root, &data_root()?)
}

fn fetch_in(root: &Path, data: &Path) -> Result<(), String> {
    install_in(root, data, false)
}

// install は同じプロジェクトの言語依存を取得する。frozen ではロックを一切書き換えない。
pub fn install(root: &Path, frozen: bool) -> Result<(), String> {
    install_in(root, &data_root()?, frozen)
}

fn requirements(manifest: &Table) -> Result<Vec<(String, String, Option<String>, toml::Value)>, String> {
    if let Some(dependencies) = manifest.get("dependencies") {
        let dependencies = dependencies.as_table().ok_or("[dependencies] はテーブルが必要")?;
        for (name, value) in dependencies {
            let table = value
                .as_table()
                .ok_or_else(|| format!("依存 {name} はgit指定のテーブルが必要"))?;
            if table.get("git").and_then(|v| v.as_str()).is_none_or(str::is_empty) {
                return Err(format!("依存 {name} のgit URLがない"));
            }
            let mut selectors = 0;
            for key in ["tag", "branch", "rev"] {
                if let Some(value) = table.get(key) {
                    if value.as_str().is_none_or(str::is_empty) {
                        return Err(format!("依存 {name} の{key}には空でない文字列が必要"));
                    }
                    selectors += 1;
                }
            }
            if selectors > 1 {
                return Err(format!("依存 {name} のtag・branch・revは一つだけ指定できる"));
            }
        }
    }
    Ok(deps(manifest)
        .into_iter()
        .map(|(name, url, rev)| {
            let mut requirement = Table::new();
            for key in ["git", "tag", "branch", "rev"] {
                if let Some(value) = manifest["dependencies"][&name].get(key) {
                    requirement.insert(key.into(), value.clone());
                }
            }
            (name, url, rev, requirement.into())
        })
        .collect())
}

fn install_in(root: &Path, data: &Path, frozen: bool) -> Result<(), String> {
    let lock_path = root.join(LOCK);
    if frozen && !lock_path.is_file() {
        return Err(format!("--frozen-lockfile には {} が必要", lock_path.display()));
    }
    let old = if lock_path.is_file() {
        read(&lock_path)?
    } else {
        Table::new()
    };
    let manifest = if root.join(MANIFEST).is_file() {
        read(&root.join(MANIFEST))?
    } else {
        Table::new()
    };
    let mut lock = Table::new();
    // 外部ツールのロックはHibana依存の再取得で消さない。
    if let Some(tools) = old
        .get("tools")
        .filter(|t| t.get("git").and_then(|g| g.as_str()).is_none())
    {
        lock.insert("tools".into(), tools.clone());
    }
    let mut todo = requirements(&manifest)?;
    while let Some((name, url, rev, requirement)) = todo.pop() {
        if let Some(t) = lock.get(&name) {
            if name == "tools" && t.get("git").and_then(|g| g.as_str()).is_none() {
                return Err("Hibana依存名 tools は外部ツールの config.hibana.lock.toml [tools] と衝突します".into());
            }
            if t.get("git").and_then(|g| g.as_str()) != Some(&url)
                || t.get("requirement").is_some_and(|r| r != &requirement)
            {
                return Err(format!("パッケージ {name} の依存指定が二つある（{url}）"));
            }
            continue;
        }
        let previous = old.get(&name).filter(|t| {
            t.get("git").and_then(|g| g.as_str()) == Some(&url)
                && match t.get("requirement") {
                    Some(r) => r == &requirement,
                    // 旧形式でも版指定なしならURLだけで一致を確認できる。
                    None => rev.is_none(),
                }
        });
        let locked = previous.and_then(|t| t.get("rev")?.as_str().map(String::from));
        if let Some(sha) = &locked {
            if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("{LOCK} の {name}.rev は完全なコミットIDが必要"));
            }
        }
        if frozen && locked.is_none() {
            return Err(format!(
                "--frozen-lockfile: {name} が {LOCK} にないか依存指定が一致しない"
            ));
        }
        let sha = match locked {
            Some(sha) if checkout(data, &url, &sha).is_dir() => sha,
            Some(sha) => {
                let actual = clone(data, &url, Some(&sha))?;
                if !actual.eq_ignore_ascii_case(&sha) {
                    return Err(format!("{name} の取得コミットが lock の {sha} と一致しない"));
                }
                actual
            }
            None => {
                eprintln!("取得: {name} ({url})");
                clone(data, &url, rev.as_deref())?
            }
        };
        let m = checkout(data, &url, &sha).join(MANIFEST);
        if m.is_file() {
            todo.extend(requirements(&read(&m)?)?);
        }
        let mut t = Table::new();
        t.insert("git".into(), url.into());
        t.insert("rev".into(), sha.into());
        // 旧形式の無指定ロックはそのまま保持し、既存fetchの互換性を保つ。
        if previous.is_none_or(|t| t.get("requirement").is_some()) {
            t.insert("requirement".into(), requirement);
        }
        lock.insert(name, t.into());
    }
    if frozen {
        // 削除された依存も設定との食い違いとして扱う。
        if old
            .iter()
            .any(|(name, value)| value.get("git").is_some() && !lock.contains_key(name))
        {
            return Err(format!(
                "--frozen-lockfile: {LOCK} に設定から削除されたHibana依存がある"
            ));
        }
    } else if lock != old {
        std::fs::write(&lock_path, lock.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// use name の入口のファイル（取ってきたパッケージの src/lib.hb）
pub fn entry(dir: &Path, name: &str) -> Option<PathBuf> {
    entry_in(dir, name, &data_root().ok()?)
}

fn entry_in(dir: &Path, name: &str, data: &Path) -> Option<PathBuf> {
    let lock = read(&root(dir)?.join(LOCK)).ok()?;
    let sha = lock.get(name)?.get("rev")?.as_str()?;
    let url = lock.get(name)?.get("git")?.as_str()?;
    Some(checkout(data, url, sha).join("src/lib.hb"))
}

// hibana add <git の URL> [tag]: 名前は URL の最後の部分（hibana- は除く）
pub fn add(root: &Path, url: &str, tag: Option<&str>) -> Result<String, String> {
    let path = root.join(MANIFEST);
    let mut m = read(&path)?;
    let last = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .unwrap_or(url);
    let name = crate::parser::module_name(last.strip_prefix("hibana-").unwrap_or(last));
    let mut d = Table::new();
    d.insert("git".into(), url.into());
    if let Some(t) = tag {
        d.insert("tag".into(), t.into());
    }
    let ds = m.entry("dependencies").or_insert(Table::new().into());
    ds.as_table_mut()
        .ok_or("[dependencies] が表でない")?
        .insert(name.clone(), d.into());
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
    std::fs::write(
        dir.join(MANIFEST),
        format!("[package]\nname = \"{pkg}\"\nversion = \"0.1.0\"\n\n[dependencies]\n"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("src/main.hb"),
        "use std.*\n\nfn main() {\n    print(\"hello\")\n}\n",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(lib: &Path) -> String {
        git(lib, &["add", "."]).unwrap();
        git(
            lib,
            &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x"],
        )
        .unwrap();
        git(lib, &["rev-parse", "HEAD"]).unwrap()
    }

    fn app(dir: &Path, url: &str) {
        new(dir.to_str().unwrap()).unwrap();
        std::fs::write(
            dir.join(MANIFEST),
            format!("[dependencies]\ngreet = {{ git = {url:?} }}\n"),
        )
        .unwrap();
    }

    #[test]
    fn xdg_data_home() {
        let home = std::env::temp_dir();
        assert_eq!(
            data_home(Some(home.join("data").into()), Some(home.clone().into())).unwrap(),
            home.join("data")
        );
        for xdg in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                data_home(xdg, Some(home.clone().into())).unwrap(),
                home.join(".local/share")
            );
        }
        assert!(data_home(None, None).is_err());
    }

    // 複数プロジェクト・複数版で共有し、ロックを保持し、ストアだけで再取得できる。
    #[test]
    fn fetch_and_entry() {
        let t = TempDir::new(&std::env::temp_dir().join("hibana-pkg-tests")).unwrap();
        let data = t.0.join("data");
        let (lib, first, second) = (t.0.join("hibana-greet"), t.0.join("app"), t.0.join("other"));
        std::fs::create_dir_all(lib.join("src")).unwrap();
        std::fs::write(lib.join("src/lib.hb"), "fn hi() -> str { \"hi\" }\n").unwrap();
        git(&lib, &["init", "-q"]).unwrap();
        let rev1 = commit(&lib);
        let url = format!("file://{}", lib.display());
        app(&first, &url);
        app(&second, &url);
        let tools = "[tools.\"npm:demo\"]\nrequirement = \"*\"\nversion = \"1.2.3\"\n";
        std::fs::write(first.join(LOCK), tools).unwrap();
        fetch_in(&first, &data).unwrap();
        assert_eq!(
            read(&first.join(LOCK)).unwrap()["tools"],
            tools.parse::<Table>().unwrap()["tools"]
        );
        fetch_in(&second, &data).unwrap();
        let e1 = entry_in(&first.join("src"), "greet", &data).unwrap();
        assert_eq!(e1, entry_in(&second, "greet", &data).unwrap());
        assert!(std::fs::read_to_string(&e1).unwrap().contains("fn hi"));
        assert!(!checkout(&data, &url, &rev1).join(".git").exists());
        let lock = std::fs::read_to_string(first.join(LOCK)).unwrap();
        std::fs::write(lib.join("README"), "new version").unwrap();
        let rev2 = commit(&lib);
        fetch_in(&first, &data).unwrap();
        assert_eq!(std::fs::read_to_string(first.join(LOCK)).unwrap(), lock);
        std::fs::remove_file(second.join(LOCK)).unwrap();
        fetch_in(&second, &data).unwrap();
        let e2 = entry_in(&second, "greet", &data).unwrap();
        assert_ne!(e1, e2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let (a, b) = (std::fs::metadata(&e1).unwrap(), std::fs::metadata(&e2).unwrap());
            assert_eq!((a.dev(), a.ino()), (b.dev(), b.ino()));
        }
        assert_ne!(checkout(&data, &url, &rev2), checkout(&data, "other-url", &rev2));
        std::fs::remove_dir_all(checkout(&data, &url, &rev1)).unwrap();
        fetch_in(&first, &data).unwrap();
        assert!(e1.is_file());
        // 保存済みのロックは取得元がなくても使える。
        std::fs::remove_dir_all(&lib).unwrap();
        fetch_in(&first, &data).unwrap();
        assert_eq!(std::fs::read_to_string(first.join(LOCK)).unwrap(), lock);
        assert_eq!(std::fs::read_dir(data.join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn fetch_rejects_external_tools_collision_and_keeps_native_tools_compatible() {
        let t = TempDir::new(&std::env::temp_dir().join("hibana-pkg-tests")).unwrap();
        let data = t.0.join("data");
        let root = t.0.join("app");
        std::fs::create_dir(&root).unwrap();
        let url = "file:///unused-native-tools";
        let sha = "abc1230000000000000000000000000000000000";
        std::fs::write(
            root.join(MANIFEST),
            format!("[dependencies]\ntools = {{ git = {url:?} }}\n"),
        )
        .unwrap();
        let external = "[tools.\"npm:demo\"]\nrequirement = \"*\"\nversion = \"1.2.3\"\n";
        std::fs::write(root.join(LOCK), external).unwrap();
        assert!(fetch_in(&root, &data).unwrap_err().contains("衝突"));
        assert_eq!(std::fs::read_to_string(root.join(LOCK)).unwrap(), external);
        // 以前からあるHibana依存名toolsだけのロックはそのまま扱う。
        std::fs::create_dir_all(checkout(&data, url, sha)).unwrap();
        let native = format!("[tools]\ngit = {url:?}\nrev = {sha:?}\n");
        std::fs::write(root.join(LOCK), &native).unwrap();
        fetch_in(&root, &data).unwrap();
        assert_eq!(std::fs::read_to_string(root.join(LOCK)).unwrap(), native);
    }

    #[test]
    fn install_frozen_checks_requirements_and_recovers_fixed_commit() {
        let t = TempDir::new(&std::env::temp_dir().join("hibana-pkg-tests")).unwrap();
        let data = t.0.join("data");
        let lib = t.0.join("lib");
        let root = t.0.join("app");
        std::fs::create_dir_all(lib.join("src")).unwrap();
        std::fs::write(lib.join("src/lib.hb"), "fn hi() -> str { \"first\" }\n").unwrap();
        git(&lib, &["init", "-q"]).unwrap();
        let first = commit(&lib);
        let url = format!("file://{}", lib.display());
        app(&root, &url);
        assert!(install_in(&root, &data, true).unwrap_err().contains("必要"));
        let manifest = |selector: &str, revision: &str| {
            format!("[dependencies]\ngreet = {{ git = {url:?}, {selector} = {revision:?} }}\n")
        };
        std::fs::write(root.join(MANIFEST), manifest("rev", &first)).unwrap();
        install_in(&root, &data, false).unwrap();
        let lock = std::fs::read(root.join(LOCK)).unwrap();
        std::fs::write(lib.join("src/lib.hb"), "fn hi() -> str { \"second\" }\n").unwrap();
        let second = commit(&lib);
        let empty_data = t.0.join("fresh-data");
        install_in(&root, &empty_data, true).unwrap();
        assert!(checkout(&empty_data, &url, &first).is_dir());
        assert_eq!(std::fs::read(root.join(LOCK)).unwrap(), lock);
        // 同じコミットでも指定方法が変われば不一致。
        git(&lib, &["tag", "fixed", &first]).unwrap();
        std::fs::write(root.join(MANIFEST), manifest("tag", "fixed")).unwrap();
        assert!(install_in(&root, &data, true).unwrap_err().contains("一致"));
        assert_eq!(std::fs::read(root.join(LOCK)).unwrap(), lock);
        std::fs::write(root.join(MANIFEST), manifest("rev", &second)).unwrap();
        assert!(install_in(&root, &data, true).is_err());
        install_in(&root, &data, false).unwrap();
        assert_eq!(
            read(&root.join(LOCK)).unwrap()["greet"]["rev"].as_str(),
            Some(second.as_str())
        );
        let updated = std::fs::read(root.join(LOCK)).unwrap();
        std::fs::write(root.join(MANIFEST), "[dependencies]\n").unwrap();
        assert!(install_in(&root, &data, true).unwrap_err().contains("削除"));
        assert_eq!(std::fs::read(root.join(LOCK)).unwrap(), updated);
        // マニフェスト自体を削除しても残存ロックを見逃さない。
        std::fs::remove_file(root.join(MANIFEST)).unwrap();
        assert!(install_in(&root, &data, true).unwrap_err().contains("削除"));
        assert_eq!(std::fs::read(root.join(LOCK)).unwrap(), updated);
        install_in(&root, &data, false).unwrap();
        assert!(read(&root.join(LOCK)).unwrap().is_empty());
    }

    #[test]
    fn install_frozen_checks_transitive_dependencies_and_preserves_bytes() {
        let t = TempDir::new(&std::env::temp_dir().join("hibana-pkg-tests")).unwrap();
        let data = t.0.join("data");
        let root = t.0.join("app");
        let parent_url = "file:///parent";
        let child_url = "file:///child";
        app(&root, parent_url);
        let parent = checkout(&data, parent_url, "1111111111111111111111111111111111111111");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::write(
            parent.join(MANIFEST),
            format!("[dependencies]\nchild = {{ git = {child_url:?} }}\n"),
        )
        .unwrap();
        std::fs::create_dir_all(checkout(&data, child_url, "2222222222222222222222222222222222222222")).unwrap();
        let partial = format!("# preserve comment\n[greet]\ngit = {parent_url:?}\nrev = '1111111111111111111111111111111111111111'\n");
        std::fs::write(root.join(LOCK), &partial).unwrap();
        assert!(install_in(&root, &data, true).unwrap_err().contains("child"));
        assert_eq!(std::fs::read_to_string(root.join(LOCK)).unwrap(), partial);
        let complete = format!("{partial}\n[child]\ngit = {child_url:?}\nrev = '2222222222222222222222222222222222222222'\n");
        std::fs::write(root.join(LOCK), &complete).unwrap();
        install_in(&root, &data, true).unwrap();
        assert_eq!(std::fs::read_to_string(root.join(LOCK)).unwrap(), complete);
        std::fs::write(root.join(MANIFEST), "[dependencies]\ngreet = {git=42}\n").unwrap();
        assert!(install_in(&root, &data, true).unwrap_err().contains("git"));
        assert_eq!(std::fs::read_to_string(root.join(LOCK)).unwrap(), complete);
    }

    #[test]
    fn failed_fetch_cleans_staging() {
        let t = TempDir::new(&std::env::temp_dir().join("hibana-pkg-tests")).unwrap();
        let data = t.0.join("data");
        assert!(clone(&data, "/nonexistent/hibana-test-repository", None).is_err());
        assert_eq!(std::fs::read_dir(data.join("tmp")).unwrap().count(), 0);
    }
}
