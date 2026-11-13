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
    install_resolved(spec, dest).map(|(bins, _)| bins)
}

pub fn install_resolved(spec: &Spec, dest: &Path) -> Result<(Vec<(String, PathBuf)>, String), String> {
    install_with_program(spec, dest, Path::new(&spec.source))
}

pub(super) fn install_npm_locked(
    spec: &Spec,
    dest: &Path,
    lock: Option<&str>,
) -> Result<(Vec<(String, PathBuf)>, String, String), String> {
    install_npm_locked_with_program(spec, dest, lock, Path::new("npm"))
}

fn install_npm_locked_with_program(
    spec: &Spec,
    dest: &Path,
    lock: Option<&str>,
    program: &Path,
) -> Result<(Vec<(String, PathBuf)>, String, String), String> {
    Spec::parse(&spec.key())?;
    if spec.source != "npm" {
        return Err("npm lockはnpmパッケージにのみ使えます".into());
    }
    let dest = fs::canonicalize(dest).map_err(|e| format!("保存先を開けません: {e}"))?;
    if let Some(raw) = lock {
        let json: serde_json::Value = serde_json::from_str(raw).map_err(|e| format!("npm lockが不正です: {e}"))?;
        let version = json["packages"][format!("node_modules/{}", spec.name)]["version"]
            .as_str()
            .ok_or("npm lockに対象versionがありません")?;
        validate_npm_lock(spec, version, raw)?;
        if spec
            .version
            .as_deref()
            .is_some_and(|expected| exact_version(expected) && expected != version)
        {
            return Err("npm lockのversionが指定版と一致しません".into());
        }
        let root = &json["packages"][""];
        // lock内のroot scriptsなどを実行対象のmanifestへ持ち込まない。
        let manifest = serde_json::json!({ "dependencies": root["dependencies"] });
        fs::write(
            dest.join("package.json"),
            serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("package.jsonを書けません: {e}"))?;
        fs::write(dest.join("package-lock.json"), raw).map_err(|e| format!("npm lockを書けません: {e}"))?;
        let status = Command::new(program)
            .args(["ci", "--prefix"])
            .arg(&dest)
            .args(["--no-audit", "--no-fund"])
            .status()
            .map_err(|e| format!("npmを起動できません（PATHを確認してください）: {e}"))?;
        if !status.success() {
            return Err(format!("{}のnpm ciに失敗しました: {status}", spec.key()));
        }
        let actual = npm_version(spec, &dest)?;
        if actual != version {
            return Err("npm ciで取得したversionがnpm lockと一致しません".into());
        }
        Ok((npm_bins(spec, &dest)?, actual, raw.to_owned()))
    } else {
        let (bins, version) = install_with_program(spec, &dest, program)?;
        let raw =
            fs::read_to_string(dest.join("package-lock.json")).map_err(|e| format!("npm lockを読めません: {e}"))?;
        validate_npm_lock(spec, &version, &raw)?;
        Ok((bins, version, raw))
    }
}

pub(super) fn validate_npm_lock(spec: &Spec, version: &str, raw: &str) -> Result<(), String> {
    let fail = |reason: &str| format!("npm lockが不正です: {reason}");
    if spec.source != "npm" || !exact_version(version) {
        return Err(fail("対象パッケージまたはversion"));
    }
    let json: serde_json::Value = serde_json::from_str(raw).map_err(|e| fail(&e.to_string()))?;
    if !matches!(json["lockfileVersion"].as_u64(), Some(2 | 3)) {
        return Err(fail("lockfileVersionは2または3が必要です"));
    }
    let packages = json["packages"]
        .as_object()
        .ok_or_else(|| fail("packagesがありません"))?;
    let root = packages
        .get("")
        .and_then(|value| value.as_object())
        .ok_or_else(|| fail("rootがありません"))?;
    let deps = root
        .get("dependencies")
        .and_then(|value| value.as_object())
        .ok_or_else(|| fail("root dependenciesがありません"))?;
    if deps.len() != 1 || !deps.contains_key(&spec.name) {
        return Err(fail("root dependenciesが対象の単一依存ではありません"));
    }
    let requirement = deps[&spec.name]
        .as_str()
        .ok_or_else(|| fail("root依存の版が文字列ではありません"))?;
    Spec::parse(&format!("npm:{}@{requirement}", spec.name)).map_err(|_| fail("local/git依存には対応していません"))?;
    for field in ["devDependencies", "optionalDependencies", "peerDependencies"] {
        if root
            .get(field)
            .is_some_and(|value| value.as_object().is_none_or(|deps| !deps.is_empty()))
        {
            return Err(fail("rootに追加の依存があります"));
        }
    }
    let target = format!("node_modules/{}", spec.name);
    if packages.get(&target).and_then(|package| package["version"].as_str()) != Some(version) {
        return Err(fail("対象versionが一致しません"));
    }
    for (path, package) in packages {
        if path.is_empty() {
            continue;
        }
        if !path.starts_with("node_modules/") || !safe_relative(Path::new(path)) || path.contains('\\') {
            return Err(fail("packageのパスが不正です"));
        }
        if package.get("link").is_some_and(|link| link != false) {
            return Err(fail("link依存には対応していません"));
        }
        if !package["version"].as_str().is_some_and(exact_version) {
            return Err(fail("packageの正確なversionがありません"));
        }
        let resolved = package["resolved"]
            .as_str()
            .ok_or_else(|| fail("resolvedがありません"))?;
        let address = resolved
            .strip_prefix("https://")
            .or_else(|| resolved.strip_prefix("http://"));
        if !address.is_some_and(|address| {
            !address.is_empty() && !address.starts_with('/') && !address.chars().any(char::is_whitespace)
        }) {
            return Err(fail("local/git依存には対応していません（resolved）"));
        }
        if !package["integrity"].as_str().is_some_and(valid_integrity) {
            return Err(fail("integrityが不正です"));
        }
    }
    Ok(())
}

fn valid_integrity(raw: &str) -> bool {
    raw.split_whitespace().next().is_some()
        && raw.split_whitespace().all(|entry| {
            let Some((algorithm, hash)) = entry.split_once('-') else {
                return false;
            };
            let bytes = match algorithm {
                "sha1" => 20,
                "sha256" => 32,
                "sha384" => 48,
                "sha512" => 64,
                _ => return false,
            };
            let encoded = (bytes + 2) / 3 * 4;
            let padding = (3 - bytes % 3) % 3;
            hash.len() == encoded
                && hash.as_bytes()[..encoded - padding]
                    .iter()
                    .all(|c| c.is_ascii_alphanumeric() || b"+/".contains(c))
                && hash.as_bytes()[encoded - padding..].iter().all(|c| *c == b'=')
        })
}

fn npm_version(spec: &Spec, dest: &Path) -> Result<String, String> {
    let text = fs::read_to_string(dest.join("node_modules").join(&spec.name).join("package.json"))
        .map_err(|e| format!("package.jsonを読めません: {e}"))?;
    let package: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("package.jsonが不正です: {e}"))?;
    package["version"]
        .as_str()
        .filter(|version| exact_version(version))
        .map(str::to_owned)
        .ok_or_else(|| "package.jsonに正確なversionがありません".into())
}

fn install_with_program(spec: &Spec, dest: &Path, program: &Path) -> Result<(Vec<(String, PathBuf)>, String), String> {
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
        let text = fs::read_to_string(dest.join("node_modules").join(&spec.name).join("package.json"))
            .map_err(|e| format!("package.jsonを読めません: {e}"))?;
        let package: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("package.jsonが不正です: {e}"))?;
        let version = package
            .get("version")
            .and_then(|value| value.as_str())
            .filter(|version| exact_version(version))
            .ok_or("package.jsonに正確なversionがありません")?
            .to_owned();
        Ok((npm_bins(spec, &dest)?, version))
    } else {
        let version = cargo_version(spec, &dest)?;
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
        Ok((cargo_bins(&dest)?, version))
    }
}

// 範囲やタグではなく、取得済みパッケージの版だけを受け入れる。
pub(super) fn exact_version(version: &str) -> bool {
    fn numeric(part: &str) -> bool {
        !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()) && (part.len() == 1 || !part.starts_with('0'))
    }
    fn suffix(text: &str, prerelease: bool) -> bool {
        text.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                && (!prerelease || !part.bytes().all(|c| c.is_ascii_digit()) || numeric(part))
        })
    }
    let (version, build) = version.split_once('+').map_or((version, None), |(v, b)| (v, Some(b)));
    let (core, pre) = version.split_once('-').map_or((version, None), |(v, p)| (v, Some(p)));
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| numeric(part))
        && build.is_none_or(|build| suffix(build, false))
        && pre.is_none_or(|pre| suffix(pre, true))
}

fn cargo_version(spec: &Spec, dest: &Path) -> Result<String, String> {
    let text =
        fs::read_to_string(dest.join(".crates.toml")).map_err(|e| format!("Cargoの管理ファイルを読めません: {e}"))?;
    let table: toml::Table = toml::from_str(&text).map_err(|e| format!("Cargoの管理ファイルが不正です: {e}"))?;
    let entries = table
        .get("v1")
        .and_then(|value| value.as_table())
        .ok_or("Cargoの管理ファイルにv1がありません")?;
    let mut versions = entries.keys().filter_map(|key| {
        let mut parts = key.split_whitespace();
        let name = parts.next()?;
        let version = parts.next()?;
        (name == spec.name && exact_version(version)).then_some(version)
    });
    let version = versions
        .next()
        .ok_or("Cargoの管理ファイルに対象パッケージの版がありません")?;
    if versions.next().is_some() {
        return Err("Cargoの管理ファイルに対象パッケージの版が複数あります".into());
    }
    Ok(version.to_owned())
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
    fn resolved_version_validation() {
        for version in ["1.2.3", "0.0.0", "1.2.3-rc.1+build.42"] {
            assert!(exact_version(version), "{version}");
        }
        for version in [
            "latest",
            "^1.2.3",
            "1.2",
            "01.2.3",
            "1.2.3-",
            "1.2.3+",
            "1.2.3-01",
            "1.2.3+a+b",
        ] {
            assert!(!exact_version(version), "{version}");
        }
    }

    fn npm_lock_fixture() -> serde_json::Value {
        let integrity = format!("sha512-{}==", "A".repeat(86));
        serde_json::json!({
            "lockfileVersion": 3,
            "packages": {
                "": { "dependencies": { "tool": "^1.0.0" } },
                "node_modules/tool": { "version": "1.2.3", "resolved": "https://registry.npmjs.org/tool/-/tool-1.2.3.tgz", "integrity": integrity },
                "node_modules/dep": { "version": "4.5.6", "resolved": "https://registry.npmjs.org/dep/-/dep-4.5.6.tgz", "integrity": integrity }
            }
        })
    }

    #[test]
    fn npm_lock_validation() {
        let spec = Spec::parse("npm:tool@1.2.3").unwrap();
        for format in [2, 3] {
            let mut lock = npm_lock_fixture();
            lock["lockfileVersion"] = format.into();
            validate_npm_lock(&spec, "1.2.3", &lock.to_string()).unwrap();
        }
        let cases = [
            ("lockfileVersion", serde_json::json!(1)),
            ("packages", serde_json::json!({})),
        ];
        for (key, value) in cases {
            let mut lock = npm_lock_fixture();
            lock[key] = value;
            assert!(validate_npm_lock(&spec, "1.2.3", &lock.to_string()).is_err());
        }
        for (key, value) in [
            ("version", serde_json::json!("latest")),
            ("resolved", serde_json::json!("file:../dep")),
            ("resolved", serde_json::json!("git+https://example.com/dep")),
            ("integrity", serde_json::json!("sha512-bogus")),
            ("integrity", serde_json::Value::Null),
            ("link", serde_json::json!(true)),
        ] {
            let mut lock = npm_lock_fixture();
            lock["packages"]["node_modules/dep"][key] = value;
            assert!(validate_npm_lock(&spec, "1.2.3", &lock.to_string()).is_err(), "{key}");
        }
        let mut lock = npm_lock_fixture();
        lock["packages"][""]["dependencies"]["tool"] = "file:../tool".into();
        assert!(validate_npm_lock(&spec, "1.2.3", &lock.to_string()).is_err());
        assert!(validate_npm_lock(&spec, "9.9.9", &npm_lock_fixture().to_string()).is_err());
        assert!(validate_npm_lock(&spec, "1.2.3", "not json").is_err());
    }

    #[test]
    fn npm_integrity_validation() {
        for (algorithm, chars, padding) in [
            ("sha1", 27, "="),
            ("sha256", 43, "="),
            ("sha384", 64, ""),
            ("sha512", 86, "=="),
        ] {
            assert!(valid_integrity(&format!("{algorithm}-{}{padding}", "A".repeat(chars))));
        }
        for raw in ["", " \n\t", "sha512-no", "sha0-AAAA", "sha512-", "sha512-💥"] {
            assert!(!valid_integrity(raw), "{raw}");
        }
        assert!(!valid_integrity(&format!("sha512-{}é", "A".repeat(86))));
    }

    #[test]
    #[cfg(unix)]
    fn npm_locked_install_and_ci() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        let raw = npm_lock_fixture().to_string();
        temp.write("fixture.json", &raw);
        temp.write("mock", "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$3/args\"\nmkdir -p \"$3/node_modules/tool\"\nprintf '{\"version\":\"1.2.3\",\"bin\":\"cli.js\"}' > \"$3/node_modules/tool/package.json\"\nprintf '#!/bin/sh\\n' > \"$3/node_modules/tool/cli.js\"\nif [ \"$1\" = install ]; then cp \"$(dirname \"$0\")/fixture.json\" \"$3/package-lock.json\"; fi\n");
        fs::set_permissions(temp.0.join("mock"), fs::Permissions::from_mode(0o755)).unwrap();
        let spec = Spec::parse("npm:tool@1.2.3").unwrap();
        let dest = temp.0.join("first");
        fs::create_dir(&dest).unwrap();
        let (_, version, saved) = install_npm_locked_with_program(&spec, &dest, None, &temp.0.join("mock")).unwrap();
        assert_eq!(version, "1.2.3");
        assert_eq!(saved, raw);
        assert!(fs::read_to_string(dest.join("args")).unwrap().starts_with("install\n"));
        let dest = temp.0.join("second");
        fs::create_dir(&dest).unwrap();
        let (_, version, saved) =
            install_npm_locked_with_program(&spec, &dest, Some(&raw), &temp.0.join("mock")).unwrap();
        assert_eq!(version, "1.2.3");
        assert_eq!(saved, raw);
        assert!(fs::read_to_string(dest.join("args")).unwrap().starts_with("ci\n"));
        assert_eq!(fs::read_to_string(dest.join("package-lock.json")).unwrap(), raw);
        let mut invalid = npm_lock_fixture();
        invalid["packages"]["node_modules/dep"]["integrity"] = "invalid".into();
        fs::remove_file(dest.join("args")).unwrap();
        assert!(
            install_npm_locked_with_program(&spec, &dest, Some(&invalid.to_string()), &temp.0.join("mock")).is_err()
        );
        assert!(!dest.join("args").exists());
    }

    #[test]
    fn cargo_version_requires_unique_matching_package() {
        let temp = Temp::new();
        let spec = Spec::parse("cargo:tool").unwrap();
        assert!(cargo_version(&spec, &temp.0).is_err());
        for text in [
            "[v1]\n",
            "[v1]\n\"other 1.2.3 (registry+x)\"=[]\n",
            "[v1]\n\"tool latest (registry+x)\"=[]\n",
            "[v1]\n\"tool 1.2.3 (registry+x)\"=[]\n\"tool 2.0.0 (registry+x)\"=[]\n",
        ] {
            temp.write(".crates.toml", text);
            assert!(cargo_version(&spec, &temp.0).is_err());
        }
        temp.write(".crates.toml", "[v1]\n\"tool 1.2.3-rc.1 (registry+x)\"=[]\n");
        assert_eq!(cargo_version(&spec, &temp.0).unwrap(), "1.2.3-rc.1");
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
        temp.write("mock", "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$3/args\"\nmkdir -p \"$3/node_modules/@scope/tool\"\nprintf '{\"version\":\"1.2.3\",\"bin\":{\"tool\":\"cli.js\"}}' > \"$3/node_modules/@scope/tool/package.json\"\nprintf '#!/bin/sh\\n' > \"$3/node_modules/@scope/tool/cli.js\"\n");
        fs::set_permissions(temp.0.join("mock"), fs::Permissions::from_mode(0o755)).unwrap();
        let dest = temp.0.join("dest");
        fs::create_dir(&dest).unwrap();
        let (bins, version) = install_with_program(
            &Spec::parse("npm:@scope/tool@1.2.3").unwrap(),
            &dest,
            &temp.0.join("mock"),
        )
        .unwrap();
        assert_eq!(version, "1.2.3");
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
        temp.write("mock", "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$3/args\"\nmkdir -p \"$3/bin\" \"$3/.build\"\nprintf binary > \"$3/bin/tool\"\nprintf '[v1]\\n\"tool 1.2.3 (registry+https://example.invalid/index)\" = [\"tool\"]\\n' > \"$3/.crates.toml\"\n");
        fs::set_permissions(temp.0.join("mock"), fs::Permissions::from_mode(0o755)).unwrap();
        let dest = temp.0.join("dest");
        fs::create_dir(&dest).unwrap();
        let spec = Spec::parse("cargo:tool@=1.2.3").unwrap();
        let (bins, version) = install_with_program(&spec, &dest, &temp.0.join("mock")).unwrap();
        assert_eq!(bins.len(), 1);
        assert_eq!(version, "1.2.3");
        let args = fs::read_to_string(dest.join("args")).unwrap();
        assert!(args.ends_with("--version\n=1.2.3\n--\ntool\n"));
        assert!(!dest.join(".build").exists());
        assert!(!dest.join(".crates.toml").exists());
    }
}
