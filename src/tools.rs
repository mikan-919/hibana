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
    let data = crate::pkg::data_root()?;
    // ponytail: 版省略・範囲指定は初回取得結果を再利用する。更新コマンドと移植可能な lock は未実装。
    let key = format!("{}\0{}\0{}", spec.key(), std::env::consts::OS, std::env::consts::ARCH);
    let root = data.join("tools/v1").join(format!("{:x}", Sha256::digest(key)));
    if root.is_dir() {
        return Ok(Installed {
            bins: bins(&root)?,
            root,
        });
    }
    let temp = Temp::new(&data)?;
    let source = temp.0.join("source");
    std::fs::create_dir(&source).map_err(|e| e.to_string())?;
    eprintln!("取得: {}", spec.key());
    let installed = providers::install(spec, &source)?;
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
    if source.join(META).exists() {
        return Err(format!("予約ファイル名が含まれる: {META}"));
    }
    std::fs::write(source.join(META), meta.to_string()).map_err(|e| e.to_string())?;
    bins(&source)?;
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
        root,
    })
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
    ensure(&spec)?;
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
pub(crate) fn script_path(config: &Table) -> Result<Option<(Temp, std::ffi::OsString)>, String> {
    let specs = dependencies(config)?;
    if specs.is_empty() {
        return Ok(None);
    }
    let temp = Temp::new(&crate::pkg::data_root()?)?;
    let mut owners = BTreeMap::new();
    for spec in specs {
        let installed = ensure(&spec)?;
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
    let mut paths = vec![temp.0.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let path = std::env::join_paths(paths).map_err(|e| e.to_string())?;
    Ok(Some((temp, path)))
}

pub(crate) fn dlx(raw: &str, args: &[String]) -> Result<i32, String> {
    let spec = Spec::parse(raw)?;
    let (bin, args) = match args {
        [flag, name, rest @ ..] if flag == "--bin" => (Some(name.as_str()), rest),
        [flag, ..] if flag == "--bin" => return Err("--bin にコマンド名が必要".into()),
        _ => (None, args),
    };
    let args = if args.first().is_some_and(|s| s == "--") {
        &args[1..]
    } else {
        args
    };
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
