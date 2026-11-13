//! npm/Cargo の実行ツールを共通 CAS に保存して実行する。
mod providers;

use providers::Spec;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use toml::Table;

const CONFIG: &str = "config.hibana.toml";
const META: &str = ".hibana-tools.toml";
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Temp(PathBuf);
impl Temp {
    fn new(data: &Path) -> Result<Self, String> {
        let parent = data.join("tmp");
        std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        Self::in_parent(&parent)
    }
    fn in_parent(parent: &Path) -> Result<Self, String> {
        loop {
            let path = parent.join(format!(
                "tools-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Installed {
    root: PathBuf,
    bins: BTreeMap<String, PathBuf>,
    version: Option<String>,
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('-')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
}

fn read_table(path: &Path) -> Result<Table, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn bins(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let meta = read_table(&root.join(META))?;
    let table = meta
        .get("bin")
        .and_then(|v| v.as_table())
        .ok_or("ツールの bin 記録が無い")?;
    let mut bins = BTreeMap::new();
    for (name, value) in table {
        let relative = Path::new(value.as_str().ok_or("bin のパスは文字列が必要")?);
        if !safe_name(name)
            || relative.as_os_str().is_empty()
            || relative.components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(format!("不正なツールの実行パス: {name}"));
        }
        let path = root.join(relative);
        let resolved = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        if !resolved.starts_with(root.canonicalize().map_err(|e| e.to_string())?) || !resolved.is_file() {
            return Err(format!("ツールの実行パスが保存先の外を指す: {name}"));
        }
        bins.insert(name.clone(), path);
    }
    Ok(bins)
}

fn ensure(spec: &Spec) -> Result<Installed, String> {
    ensure_mode(spec, false)
}

fn ensure_mode(spec: &Spec, resolved: bool) -> Result<Installed, String> {
    let data = crate::pkg::data_root()?;
    // dlx/global の版省略・範囲指定は初回取得結果を再利用する。
    // プロジェクト用は解決版を持つ別世代に保存し、古い CAS は書き換えない。
    let key = format!("{}\0{}\0{}", spec.key(), std::env::consts::OS, std::env::consts::ARCH);
    let root = data
        .join(if resolved { "tools/v2" } else { "tools/v1" })
        .join(format!("{:x}", Sha256::digest(key)));
    if root.is_dir() {
        return Ok(Installed {
            bins: bins(&root)?,
            version: if resolved { Some(cached_version(&root)?) } else { None },
            root,
        });
    }
    let temp = Temp::new(&data)?;
    let source = temp.0.join("source");
    std::fs::create_dir(&source).map_err(|e| e.to_string())?;
    eprintln!("取得: {}", spec.key());
    let (installed, version) = if resolved {
        let (bins, version) = providers::install_resolved(spec, &source)?;
        exact_version(&version)?;
        (bins, Some(version))
    } else {
        (providers::install(spec, &source)?, None)
    };
    let mut table = Table::new();
    for (name, path) in installed {
        if !safe_name(&name) || table.contains_key(&name) {
            return Err(format!("不正または重複したコマンド名: {name}"));
        }
        table.insert(name, path.to_str().ok_or("実行パスが UTF-8 ではない")?.into());
    }
    let mut meta = Table::new();
    meta.insert("spec".into(), spec.key().into());
    meta.insert("bin".into(), table.into());
    if let Some(version) = &version {
        meta.insert("version".into(), version.clone().into());
    }
    if source.join(META).exists() {
        return Err(format!("予約ファイル名が含まれる: {META}"));
    }
    std::fs::write(source.join(META), meta.to_string()).map_err(|e| e.to_string())?;
    bins(&source)?;
    // 範囲で解決した初回も正確版のキーで公開し、次回の lock 取得で再利用する。
    let root = if let Some(version) = &version {
        let mut exact = spec.clone();
        exact.version = Some(if spec.source == "cargo" {
            format!("={version}")
        } else {
            version.clone()
        });
        let key = format!("{}\0{}\0{}", exact.key(), std::env::consts::OS, std::env::consts::ARCH);
        data.join("tools/v2").join(format!("{:x}", Sha256::digest(key)))
    } else {
        root
    };
    let tree = temp.0.join("tree");
    crate::pkg::import_tool_tree(&source, &tree, &data)?;
    std::fs::create_dir_all(root.parent().unwrap()).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&tree, &root) {
        if !root.is_dir() {
            return Err(format!("{}: {e}", root.display()));
        }
    }
    Ok(Installed {
        bins: bins(&root)?,
        version: if resolved { Some(cached_version(&root)?) } else { None },
        root,
    })
}

// lock の版は範囲・タグではなく、取得元が返した正確な版だけを受け付ける。
fn exact_version(version: &str) -> Result<(), String> {
    if !providers::exact_version(version) {
        return Err(format!("lock の正確なバージョンが不正: {version}"));
    }
    Ok(())
}

fn cached_version(root: &Path) -> Result<String, String> {
    let meta = read_table(&root.join(META))?;
    let version = meta
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or("ツールの解決版記録が無い")?;
    exact_version(version)?;
    Ok(version.into())
}

fn read_lock(path: &Path) -> Result<Table, String> {
    let lock = match std::fs::symlink_metadata(path) {
        Ok(_) => read_table(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Table::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if let Some(value) = lock.get("tools") {
        let tools = value.as_table().ok_or("config.hibana.lock.toml の [tools] はテーブルが必要")?;
        if tools.get("git").is_some_and(|v| v.is_str()) {
            return Err("config.hibana.lock.toml の tools は Hibana 依存名と衝突しています".into());
        }
        for (name, entry) in tools {
            let spec = Spec::parse(name)?;
            if spec.version.is_some() {
                return Err(format!("lock のキーに版を含めないでください: {name}"));
            }
            let requirement = entry
                .get("requirement")
                .and_then(|v| v.as_str())
                .ok_or("lock の requirement は文字列が必要")?;
            Spec::parse(&format!("{name}@{requirement}"))?;
            let version = entry
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or("lock の version は文字列が必要")?;
            exact_version(version)?;
        }
    }
    Ok(lock)
}

// ponytail: 直接依存の版だけを固定する。推移依存・integrity の固定には取得元ごとの lock 保存が必要。
fn ensure_locked(spec: &Spec, lock: &mut Table) -> Result<Installed, String> {
    let name = identity(spec);
    let requirement = spec.version.as_deref().unwrap_or("*");
    let locked = lock
        .get("tools")
        .and_then(|t| t.get(&name))
        .filter(|entry| entry.get("requirement").and_then(|v| v.as_str()) == Some(requirement))
        .and_then(|entry| entry.get("version"))
        .and_then(|v| v.as_str());
    let installed = if let Some(version) = locked {
        let mut exact = spec.clone();
        exact.version = Some(if spec.source == "cargo" {
            format!("={version}")
        } else {
            version.into()
        });
        let installed = ensure_mode(&exact, true)?;
        if installed.version.as_deref() != Some(version) {
            return Err(format!(
                "{} の取得版が lock の {version} と一致しません",
                identity(spec)
            ));
        }
        installed
    } else {
        ensure_mode(spec, true)?
    };
    let mut entry = Table::new();
    entry.insert("requirement".into(), requirement.into());
    entry.insert(
        "version".into(),
        installed.version.clone().ok_or("ツールの解決版記録が無い")?.into(),
    );
    lock.entry("tools")
        .or_insert(Table::new().into())
        .as_table_mut()
        .ok_or("[tools] はテーブルが必要")?
        .insert(name, entry.into());
    Ok(installed)
}

fn write_lock(path: &Path, old: &Table, lock: &Table) -> Result<(), String> {
    if old != lock {
        atomic_write(path, &lock.to_string())?;
    }
    Ok(())
}

fn identity(spec: &Spec) -> String {
    format!("{}:{}", spec.source, spec.name)
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn shim(path: &Path, target: &Path, owner: &str) -> Result<(), String> {
    let target = target.to_str().ok_or("実行パスが UTF-8 ではない")?;
    std::fs::write(
        path,
        format!("#!/bin/sh\n# hibana: {owner}\nexec {} \"$@\"\n", quote(target)),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn add(raw: &str, global: bool) -> Result<(), String> {
    let spec = Spec::parse(raw)?;
    if global {
        let installed = ensure(&spec)?;
        if installed.bins.is_empty() {
            return Err(format!("{} には実行コマンドが無い", spec.key()));
        }
        let data = crate::pkg::data_root()?;
        let bin = data.join("bin");
        std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
        let marker = format!("# hibana: {}", identity(&spec));
        // 他のパッケージやユーザーのファイルは上書きしない。
        for name in installed.bins.keys() {
            let dest = bin.join(name);
            if std::fs::symlink_metadata(&dest).is_ok() {
                let owned = std::fs::read_to_string(&dest)
                    .ok()
                    .is_some_and(|s| s.lines().nth(1) == Some(&marker));
                if !owned {
                    return Err(format!(
                        "コマンド名が衝突: {}（dlx --bin で実行できます）",
                        dest.display()
                    ));
                }
            }
        }
        let temp = Temp::new(&data)?;
        for (name, target) in &installed.bins {
            let staged = temp.0.join(name);
            shim(&staged, target, &identity(&spec))?;
            std::fs::rename(&staged, bin.join(name)).map_err(|e| e.to_string())?;
        }
        eprintln!("追加: {}（{} を PATH に追加してください）", spec.key(), bin.display());
        return Ok(());
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let path = crate::scripts::find_config(&cwd)?.unwrap_or_else(|| cwd.join(CONFIG));
    let mut config = if path.exists() {
        read_table(&path)?
    } else {
        Table::new()
    };
    let deps = config
        .entry("dependencies")
        .or_insert(Table::new().into())
        .as_table_mut()
        .ok_or("[dependencies] はテーブルが必要")?;
    deps.insert(identity(&spec), spec.version.as_deref().unwrap_or("*").into());
    // 設定の型と識別子を先に検査し、取得失敗時には設定を書き換えない。
    dependencies(&config)?;
    let lock_path = path.parent().unwrap().join("config.hibana.lock.toml");
    let old = read_lock(&lock_path)?;
    let mut lock = old.clone();
    ensure_locked(&spec, &mut lock)?;
    write_lock(&lock_path, &old, &lock)?;
    atomic_write(&path, &config.to_string())?;
    eprintln!("追加: {}", spec.key());
    Ok(())
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let temp = Temp::in_parent(path.parent().unwrap())?;
    let staged = temp.0.join("config");
    std::fs::write(&staged, content).map_err(|e| e.to_string())?;
    std::fs::rename(staged, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn dependencies(config: &Table) -> Result<Vec<Spec>, String> {
    let Some(value) = config.get("dependencies") else {
        return Ok(Vec::new());
    };
    let table = value.as_table().ok_or("[dependencies] はテーブルが必要")?;
    table
        .iter()
        .map(|(name, version)| {
            let base = Spec::parse(name)?;
            if base.version.is_some() {
                return Err(format!("dependencies のキーに版を含めないでください: {name}"));
            }
            let version = version
                .as_str()
                .ok_or_else(|| format!("dependencies.{name} は版の文字列が必要"))?;
            if version == "*" {
                Ok(base)
            } else {
                Spec::parse(&format!("{name}@{version}"))
            }
        })
        .collect()
}

// スクリプトの実行中だけ bin ディレクトリを保持する。
pub(crate) fn script_path(config: &Table, config_path: &Path) -> Result<Option<(Temp, std::ffi::OsString)>, String> {
    let specs = dependencies(config)?;
    if specs.is_empty() {
        return Ok(None);
    }
    let temp = Temp::new(&crate::pkg::data_root()?)?;
    let lock_path = config_path.parent().unwrap().join("config.hibana.lock.toml");
    let old = read_lock(&lock_path)?;
    let mut lock = old.clone();
    let mut owners = BTreeMap::new();
    for spec in specs {
        let installed = ensure_locked(&spec, &mut lock)?;
        for (name, target) in installed.bins {
            if let Some(other) = owners.insert(name.clone(), spec.key()) {
                return Err(format!(
                    "コマンド名 {name} が衝突: {other} と {}（hibana dlx で識別子を指定してください）",
                    spec.key()
                ));
            }
            shim(&temp.0.join(name), &target, &identity(&spec))?;
        }
    }
    write_lock(&lock_path, &old, &lock)?;
    let mut paths = vec![temp.0.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let path = std::env::join_paths(paths).map_err(|e| e.to_string())?;
    Ok(Some((temp, path)))
}

pub(crate) fn dlx(raw: &str, bin: Option<&str>, args: &[String]) -> Result<i32, String> {
    let spec = Spec::parse(raw)?;
    let installed = ensure(&spec)?;
    let name = select_bin(&spec, &installed.bins, bin)?;
    let mut command = Command::new(&installed.bins[&name]);
    command.args(args);
    // 同じパッケージの別 bin を子プロセスからも呼べるようにする。
    let temp = Temp::new(&crate::pkg::data_root()?)?;
    for (name, target) in &installed.bins {
        shim(&temp.0.join(name), target, &identity(&spec))?;
    }
    let mut paths = vec![
        temp.0.clone(),
        installed.root.join("node_modules/.bin"),
        installed.root.join("bin"),
    ];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    command.env("PATH", std::env::join_paths(paths).map_err(|e| e.to_string())?);
    let status = command.status().map_err(|e| format!("{name} を起動できない: {e}"))?;
    Ok(exit_code(status))
}

fn select_bin(spec: &Spec, bins: &BTreeMap<String, PathBuf>, requested: Option<&str>) -> Result<String, String> {
    if let Some(name) = requested {
        if bins.contains_key(name) {
            return Ok(name.to_string());
        }
        return Err(format!("{} にコマンド {name} が無い", spec.key()));
    }
    if bins.len() == 1 {
        return Ok(bins.keys().next().unwrap().clone());
    }
    let basename = spec.name.rsplit('/').next().unwrap();
    if bins.contains_key(basename) {
        return Ok(basename.into());
    }
    Err(format!(
        "{} のコマンドを --bin で指定してください（{}）",
        spec.key(),
        bins.keys().cloned().collect::<Vec<_>>().join(", ")
    ))
}

pub(crate) fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(1)
    }
    #[cfg(not(unix))]
    {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_command_not_package_identifier() {
        let spec = Spec::parse("npm:@scope/pkg").unwrap();
        let mut bins = BTreeMap::from([("other".into(), PathBuf::from("cli"))]);
        assert_eq!(select_bin(&spec, &bins, None).unwrap(), "other");
        bins.insert("second".into(), PathBuf::from("cli2"));
        assert!(select_bin(&spec, &bins, None).is_err());
        assert_eq!(select_bin(&spec, &bins, Some("second")).unwrap(), "second");
        assert!(select_bin(&spec, &bins, Some("missing")).is_err());
        bins.insert("pkg".into(), PathBuf::from("cli3"));
        assert_eq!(select_bin(&spec, &bins, None).unwrap(), "pkg");
    }
    #[test]
    fn lock_validation_preserves_native_entries_and_rejects_non_exact_versions() {
        let temp = Temp::in_parent(&std::env::temp_dir()).unwrap();
        let path = temp.0.join("config.hibana.lock.toml");
        assert!(read_lock(&path).unwrap().is_empty());
        let valid = "[lib]\ngit = 'file:///lib'\nrev = 'abc'\n[tools.\"npm:demo\"]\nrequirement = '^1.0.0'\nversion = '1.2.3-beta.1+build.2'\n";
        std::fs::write(&path, valid).unwrap();
        let old = read_lock(&path).unwrap();
        write_lock(&path, &old, &old).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), valid);
        assert_eq!(old["lib"]["rev"].as_str(), Some("abc"));
        for version in ["latest", "^1.2.3", "1.2", "01.2.3", "1.2.3-", "1.2.3+"] {
            std::fs::write(
                &path,
                format!("[tools.\"npm:demo\"]\nrequirement = '*'\nversion = {version:?}\n"),
            )
            .unwrap();
            assert!(read_lock(&path).is_err(), "{version}");
        }
        std::fs::write(&path, "[tools]\ngit = 'file:///lib'\nrev = 'abc'\n").unwrap();
        assert!(read_lock(&path).unwrap_err().contains("衝突"));
    }

    #[test]
    fn config_dependency_validation() {
        let config: Table = "[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"1.2.3\""
            .parse()
            .unwrap();
        let specs = dependencies(&config).unwrap();
        assert_eq!(specs.len(), 2);
        for invalid in [
            "dependencies = []",
            "[dependencies]\nfoo = '1'",
            "[dependencies]\n\"npm:demo\" = 1",
            "[dependencies]\n\"npm:demo@1\" = '*'",
        ] {
            assert!(dependencies(&invalid.parse().unwrap()).is_err());
        }
        assert!(!safe_name("../bad"));
        assert!(!safe_name("--bad"));
        assert_eq!(quote("a'b"), "'a'\\''b'");
    }
}
