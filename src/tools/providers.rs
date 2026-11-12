use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub source: String,
    pub name: String,
    pub version: Option<String>,
}

impl Spec {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let (source, package) = raw
            .split_once(':')
            .ok_or("識別子は npm:名前 または cargo:名前 で指定してください")?;
        if !matches!(source, "npm" | "cargo") {
            return Err(format!("未対応の取得元です: {source}"));
        }
        let version_at = package.rfind('@').filter(|&i| i > 0);
        let (name, version) = match version_at {
            Some(i) => (&package[..i], Some(&package[i + 1..])),
            None => (package, None),
        };
        let valid_name = if source == "npm" && name.starts_with('@') {
            name[1..]
                .split_once('/')
                .is_some_and(|(scope, name)| segment(scope) && segment(name))
        } else {
            segment(name)
        };
        if !valid_name {
            return Err(format!("パッケージ名が不正です: {name}"));
        }
        if let Some(version) = version {
            if version.is_empty()
                || !version
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-+^~*=<>|".contains(&c))
                || !version.bytes().any(|c| c.is_ascii_alphanumeric() || c == b'*')
            {
                return Err(format!("バージョンが不正です: {version}"));
            }
        }
        Ok(Self {
            source: source.into(),
            name: name.into(),
            version: version.map(str::to_owned),
        })
    }

    pub fn key(&self) -> String {
        match &self.version {
            Some(version) => format!("{}:{}@{}", self.source, self.name, version),
            None => format!("{}:{}", self.source, self.name),
        }
    }
}

fn segment(name: &str) -> bool {
    name.bytes().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

pub fn install(spec: &Spec, dest: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    install_with_program(spec, dest, Path::new(&spec.source))
}

fn install_with_program(spec: &Spec, dest: &Path, program: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    // 公開構造体を直接生成した場合もコマンド引数に渡す前に検証する。
    Spec::parse(&spec.key())?;
    let dest = fs::canonicalize(dest).map_err(|e| format!("保存先を開けません: {e}"))?;
    let mut command = Command::new(program);
    match spec.source.as_str() {
        "npm" => {
            let package = match &spec.version {
                Some(version) => format!("{}@{version}", spec.name),
                None => spec.name.clone(),
            };
            command
                .args(["install", "--prefix"])
                .arg(&dest)
                .args(["--no-audit", "--no-fund", "--"])
                .arg(package);
        }
        "cargo" => {
            command
                .args(["install", "--root"])
                .arg(&dest)
                .arg("--target-dir")
                .arg(dest.join(".build"));
            if let Some(version) = &spec.version {
                command.arg("--version").arg(version);
            }
            command.arg("--").arg(&spec.name);
        }
        _ => unreachable!(),
    }
    let status = command
        .status()
        .map_err(|e| format!("{}を起動できません（PATHを確認してください）: {e}", spec.source))?;
    if !status.success() {
        return Err(format!("{}のインストールに失敗しました: {status}", spec.key()));
    }
    if spec.source == "npm" {
        npm_bins(spec, &dest)
    } else {
        let build = dest.join(".build");
        if build.exists() {
            fs::remove_dir_all(build).map_err(|e| format!("ビルド用一時ファイルを削除できません: {e}"))?;
        }
        for name in [".crates.toml", ".crates2.json"] {
            let path = dest.join(name);
            if path.exists() {
                fs::remove_file(path).map_err(|e| format!("Cargoの管理ファイルを削除できません: {e}"))?;
            }
        }
        cargo_bins(&dest)
    }
}

fn npm_bins(spec: &Spec, dest: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let package_dir = dest.join("node_modules").join(&spec.name);
    let text =
        fs::read_to_string(package_dir.join("package.json")).map_err(|e| format!("package.jsonを読めません: {e}"))?;
    let package: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("package.jsonが不正です: {e}"))?;
    let entries: Vec<(String, String)> = match package.get("bin") {
        Some(serde_json::Value::String(path)) => vec![(spec.name.rsplit('/').next().unwrap().into(), path.clone())],
        Some(serde_json::Value::Object(bins)) => bins
            .iter()
            .map(|(name, path)| {
                path.as_str()
                    .map(|path| (name.clone(), path.to_owned()))
                    .ok_or_else(|| format!("コマンドのパスが文字列ではありません: {name}"))
            })
            .collect::<Result<_, _>>()?,
        None | Some(serde_json::Value::Null) => Vec::new(),
        _ => return Err("package.jsonのbinが不正です".into()),
    };
    let mut bins = Vec::new();
    for (name, path) in entries {
        if !segment(&name) || !safe_relative(Path::new(&path)) {
            return Err(format!("コマンド名またはパスが不正です: {name}"));
        }
        let target =
            fs::canonicalize(package_dir.join(&path)).map_err(|e| format!("コマンドを開けません: {name}: {e}"))?;
        if !target.starts_with(dest) || !target.is_file() {
            return Err(format!("コマンドが保存先の外を参照しています: {name}"));
        }
        let bin_path = target.strip_prefix(dest).map_err(|e| e.to_string())?.to_path_buf();
        bins.push((name, bin_path));
    }
    require_bins(bins)
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn cargo_bins(dest: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let entries = fs::read_dir(dest.join("bin")).map_err(|e| format!("Cargoのコマンド一覧を読めません: {e}"))?;
    let mut bins = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("コマンドを読めません: {e}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "コマンド名がUTF-8ではありません")?;
        if !segment(&name) || !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            return Err(format!("Cargoのコマンドが不正です: {name}"));
        }
        bins.push((name.clone(), PathBuf::from("bin").join(name)));
    }
    require_bins(bins)
}

fn require_bins(mut bins: Vec<(String, PathBuf)>) -> Result<Vec<(String, PathBuf)>, String> {
    if bins.is_empty() {
        return Err("このパッケージに実行可能なコマンドがありません".into());
    }
    bins.sort();
    Ok(bins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "hibana-providers-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn identifiers() {
        for id in [
            "npm:vite",
            "npm:@scope/tool",
            "npm:@scope/tool@^1.2.3",
            "cargo:ripgrep@14.1.0",
            "npm:vp@latest",
        ] {
            assert_eq!(Spec::parse(id).unwrap().key(), id);
        }
        for id in [
            "vite",
            "git:x",
            "npm:",
            "npm:-x",
            "npm:../x",
            "npm:@scope",
            "npm:@scope/x/y",
            "npm:x@",
            "npm:x@file:foo",
            "cargo:x;echo",
            "cargo:x@1 2",
            "cargo:@scope/x",
        ] {
            assert!(Spec::parse(id).is_err(), "{id}");
        }
    }

    #[cfg(unix)]
    fn link(temp: &Temp, target: &str, name: &str) {
        let path = temp.0.join(format!("node_modules/.bin/{name}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, path).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn npm_scoped_string_and_multiple_bins() {
        let temp = Temp::new();
        temp.write("node_modules/@scope/tool/package.json", r#"{"bin":"./cli.js"}"#);
        temp.write("node_modules/@scope/tool/cli.js", "#!/usr/bin/env node\n");
        link(&temp, "../@scope/tool/cli.js", "tool");
        let spec = Spec::parse("npm:@scope/tool").unwrap();
        assert_eq!(
            npm_bins(&spec, &temp.0).unwrap(),
            vec![("tool".into(), "node_modules/@scope/tool/cli.js".into())]
        );
        temp.write(
            "node_modules/@scope/tool/package.json",
            r#"{"bin":{"other":"cli.js","tool":"cli.js"}}"#,
        );
        link(&temp, "../@scope/tool/cli.js", "other");
        assert_eq!(npm_bins(&spec, &temp.0).unwrap().len(), 2);
    }

    #[test]
    fn npm_rejects_missing_and_unsafe_bins() {
        let temp = Temp::new();
        let spec = Spec::parse("npm:tool").unwrap();
        for json in [
            r#"{}"#,
            r#"{"bin":{"../bad":"cli.js"}}"#,
            r#"{"bin":"../../escape"}"#,
            r#"{"bin":{"tool":7}}"#,
        ] {
            temp.write("node_modules/tool/package.json", json);
            assert!(npm_bins(&spec, &temp.0).is_err());
        }
    }

    #[test]
    fn cargo_multiple_bins() {
        let temp = Temp::new();
        temp.write("bin/second", "binary2");
        temp.write("bin/first", "binary1");
        assert_eq!(
            cargo_bins(&temp.0).unwrap(),
            vec![
                ("first".into(), "bin/first".into()),
                ("second".into(), "bin/second".into())
            ]
        );
    }

    #[test]
    #[cfg(unix)]
    fn npm_mock_install_arguments() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        temp.write("mock", "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$3/args\"\nmkdir -p \"$3/node_modules/@scope/tool\"\nprintf '{\"bin\":{\"tool\":\"cli.js\"}}' > \"$3/node_modules/@scope/tool/package.json\"\nprintf '#!/bin/sh\\n' > \"$3/node_modules/@scope/tool/cli.js\"\n");
        fs::set_permissions(temp.0.join("mock"), fs::Permissions::from_mode(0o755)).unwrap();
        let dest = temp.0.join("dest");
        fs::create_dir(&dest).unwrap();
        let bins = install_with_program(
            &Spec::parse("npm:@scope/tool@1.2.3").unwrap(),
            &dest,
            &temp.0.join("mock"),
        )
        .unwrap();
        assert_eq!(bins, vec![("tool".into(), "node_modules/@scope/tool/cli.js".into())]);
        let args = fs::read_to_string(dest.join("args")).unwrap();
        assert!(args.ends_with("--no-audit\n--no-fund\n--\n@scope/tool@1.2.3\n"));
    }

    #[test]
    #[cfg(unix)]
    fn install_failure_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        temp.write("mock", "#!/bin/sh\nexit 19\n");
        fs::set_permissions(temp.0.join("mock"), fs::Permissions::from_mode(0o755)).unwrap();
        let error = install_with_program(&Spec::parse("npm:tool").unwrap(), &temp.0, &temp.0.join("mock")).unwrap_err();
        assert!(error.contains("インストールに失敗"));
        assert!(error.contains("19"));
    }

    #[test]
    #[cfg(unix)]
    fn cargo_mock_install_arguments_and_cleanup() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        temp.write("mock", "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$3/args\"\nmkdir -p \"$3/bin\" \"$3/.build\"\nprintf binary > \"$3/bin/tool\"\nprintf metadata > \"$3/.crates.toml\"\n");
        fs::set_permissions(temp.0.join("mock"), fs::Permissions::from_mode(0o755)).unwrap();
        let dest = temp.0.join("dest");
        fs::create_dir(&dest).unwrap();
        let spec = Spec::parse("cargo:tool@1.2.3").unwrap();
        assert_eq!(
            install_with_program(&spec, &dest, &temp.0.join("mock")).unwrap().len(),
            1
        );
        let args = fs::read_to_string(dest.join("args")).unwrap();
        assert!(args.ends_with("--version\n1.2.3\n--\ntool\n"));
        assert!(!dest.join(".build").exists());
        assert!(!dest.join(".crates.toml").exists());
    }
}
