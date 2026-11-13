#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hibana-install-cli-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("mock-bin")).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        let fixture = Self(root);
        fixture.executable("mock-bin/npm", NPM);
        fixture.executable("mock-bin/cargo", CARGO);
        fixture
    }

    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join(path), text).unwrap();
    }

    fn executable(&self, path: &str, text: &str) {
        self.write(path, text);
        std::fs::set_permissions(self.0.join(path), std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn command(&self, executable: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(executable);
        let mut paths = vec![self.0.join("mock-bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        command
            .current_dir(&self.0)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("HOME", self.0.join("home"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .env("MOCK_LOG", self.0.join("installs.log"));
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_hibana")).args(args).output().unwrap()
    }

    fn installs(&self) -> String {
        std::fs::read_to_string(self.0.join("installs.log")).unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn success(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn error(output: &Output) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!output.stderr.is_empty(), "診断がない: {output:?}");
}

// ネットワークへ接続せず、npmが生成する配置だけを再現する。
const NPM: &str = r##"#!/bin/sh
set -eu
prefix=''
package='demo'
mode=install
while [ "$#" -gt 0 ]; do
  case "$1" in
    ci) mode=ci; shift ;;
    --prefix) prefix="$2"; shift 2 ;;
    demo|demo@*|many|many@*|@scope/demo|@scope/demo@*) package="$1"; shift ;;
    *) shift ;;
  esac
done
[ -n "$prefix" ]
if [ "$mode" = ci ]; then
  package=$(node -p 'const p=require(process.argv[1]).packages; const n=Object.keys(p[""].dependencies)[0]; n+"@"+p["node_modules/"+n].version' "$prefix/package-lock.json")
fi
printf 'npm:%s:%s\n' "$package" "$mode" >> "$MOCK_LOG"
case "$package" in
  many*) name=many ;;
  @scope/*) name=@scope/demo ;;
  *) name=demo ;;
esac
version="${MOCK_NPM_VERSION:-1.2.3}"
case "$package" in
  *@1.2.3) version=1.2.3 ;;
  *@9.9.9) version=9.9.9 ;;
esac
[ "$name" != many ] || version=1.0.0
if [ "$mode" != ci ]; then
  node - "$prefix" "$name" "$version" <<'LOCK'
const fs = require('fs');
const [prefix, name, version] = process.argv.slice(2);
const dep = process.env.MOCK_TRANSITIVE_VERSION || '4.5.6';
const entry = (n, v) => ({version:v, resolved:'https://registry.example.invalid/'+n+'/-/'+v+'.tgz', integrity:'sha512-'+Buffer.alloc(64).toString('base64')});
fs.writeFileSync(prefix+'/package-lock.json', JSON.stringify({name:'hibana-tools', version:'1.0.0', lockfileVersion:3, packages:{'':{name:'hibana-tools', version:'1.0.0', dependencies:{[name]:version}}, ['node_modules/'+name]:{...entry(name,version), dependencies:{transitive:dep}}, 'node_modules/transitive':entry('transitive',dep)}}));
LOCK
fi
transitive=$(node -p 'require(process.argv[1]).packages["node_modules/transitive"].version' "$prefix/package-lock.json")
root="$prefix/node_modules/$name"
mkdir -p "$root"
printf '%s' "$transitive" > "$root/transitive-version"
if [ "$name" = many ]; then
  printf '%s' '{"name":"many","version":"1.0.0","bin":{"first":"cli.js","second":"cli.js"}}' > "$root/package.json"
else
  printf '{"name":"%s","version":"%s","bin":{"demo":"cli.js"}}' "$name" "$version" > "$root/package.json"
fi
cat > "$root/cli.js" <<'CLI'
#!/bin/sh
if [ "${1-}" = --fail ]; then exit 37; fi
if [ "${1-}" = --transitive ]; then cat "$(dirname "$0")/transitive-version"; exit; fi
printf npm-demo
for arg in "$@"; do printf '<%s>' "$arg"; done
CLI
chmod +x "$root/cli.js"
"##;

// Cargoのビルド成果物はnpmと同名でも別の内容にする。
const CARGO: &str = r##"#!/bin/sh
set -eu
mode="$1"
root=''
version=''
manifest=''
source=''
locked=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --root) root="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --manifest-path) manifest="$2"; shift 2 ;;
    --path) source="$2"; shift 2 ;;
    --locked) locked=1; shift ;;
    *) shift ;;
  esac
done
if [ "$mode" = metadata ]; then
  node - "$manifest" <<'METADATA'
const fs = require('fs'), path = require('path');
const manifest = process.argv[2];
const dir = path.join(path.dirname(manifest), 'mock-registry', 'demo-2.3.4');
fs.mkdirSync(path.join(dir, 'src'), {recursive:true});
fs.writeFileSync(path.join(dir,'Cargo.toml'), '[package]\nname = "demo"\nversion = "2.3.4"\nedition = "2021"\n[dependencies]\ntransitive = "*"\n');
fs.writeFileSync(path.join(dir,'src/main.rs'), 'fn main() {}');
const id='registry+https://github.com/rust-lang/crates.io-index#demo@2.3.4';
console.log(JSON.stringify({packages:[{id,name:'demo',version:'2.3.4',source:'registry+https://github.com/rust-lang/crates.io-index',manifest_path:path.join(dir,'Cargo.toml')}],resolve:{root:'probe',nodes:[{id:'probe',deps:[{name:'hibana_tool',pkg:id}],dependencies:[id]}]}}));
METADATA
  exit
fi
if [ "$mode" = generate-lockfile ]; then
  node - "$manifest" <<'LOCK'
const fs=require('fs'), path=require('path');
const version=process.env.MOCK_CARGO_TRANSITIVE_VERSION || '4.5.6';
fs.writeFileSync(path.join(path.dirname(process.argv[2]),'Cargo.lock'), 'version = 4\n\n[[package]]\nname = "demo"\nversion = "2.3.4"\ndependencies = ["transitive"]\n\n[[package]]\nname = "transitive"\nversion = "'+version+'"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "'+'a'.repeat(64)+'"\n');
LOCK
  exit
fi
[ -n "$root" ]
transitive=''
if [ -n "$source" ]; then
  [ "$locked" = 1 ]
  [ -f "$source/Cargo.lock" ]
  transitive=$(node - "$source/Cargo.lock" <<'VERSION'
const fs=require('fs');
const text=fs.readFileSync(process.argv[2],'utf8');
const entry=text.split('[[package]]').find(p=>/name\s*=\s*"transitive"/.test(p));
console.log(entry.match(/version\s*=\s*"([^"]+)"/)[1]);
VERSION
)
  version='=2.3.4'
fi
printf 'cargo:demo%s\n' "${version:+@$version}" >> "$MOCK_LOG"
mkdir -p "$root/bin"
printf '[v1]\n"demo 2.3.4 (registry+https://example.invalid/index)" = ["demo"]\n' > "$root/.crates.toml"
printf '#!/bin/sh\nif [ "${1-}" = --transitive ]; then printf "%%s" "%s"; exit; fi\n' "$transitive" > "$root/bin/demo"
cat >> "$root/bin/demo" <<'CLI'
printf cargo-demo
for arg in "$@"; do printf '<%s>' "$arg"; done
CLI
chmod +x "$root/bin/demo"
"##;

const LOCK: &str = "config.hibana.lock.toml";

fn lock(f: &Fixture) -> Vec<u8> {
    std::fs::read(f.0.join(LOCK)).unwrap()
}

#[test]
fn install_fetches_all_tools_and_reuses_lock() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
    );
    success(&f.run(&["install"]));
    let bytes = lock(&f);
    let table: toml::Table = String::from_utf8(bytes.clone()).unwrap().parse().unwrap();
    assert_eq!(table["tools"]["npm:demo"]["version"].as_str(), Some("1.2.3"));
    assert_eq!(table["tools"]["cargo:demo"]["version"].as_str(), Some("2.3.4"));
    success(&f.run(&["install"]));
    assert_eq!(f.installs().lines().count(), 2);
    assert_eq!(lock(&f), bytes);
}

#[test]
fn frozen_fetches_fixed_versions_into_empty_store_without_writing_lock() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
    );
    success(&f.run(&["install"]));
    let mut bytes = "# 手動コメントも保持\n".as_bytes().to_vec();
    bytes.extend(lock(&f));
    std::fs::write(f.0.join(LOCK), &bytes).unwrap();
    let output = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .args(["install", "--frozen-lockfile"])
        .env("XDG_DATA_HOME", f.0.join("fresh-data"))
        .env("MOCK_NPM_VERSION", "9.9.9")
        .output()
        .unwrap();
    success(&output);
    assert_eq!(lock(&f), bytes);
    assert!(f.installs().contains("npm:demo@1.2.3"));
    assert!(f.installs().contains("cargo:demo@=2.3.4"));
    assert_eq!(f.installs().lines().count(), 4);
}

#[test]
fn frozen_requires_lock_and_does_not_create_it() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
    error(&f.run(&["install", "--frozen-lockfile"]));
    assert!(!f.0.join(LOCK).exists());
    assert!(f.installs().is_empty());
}

#[test]
fn frozen_rejects_changed_missing_and_removed_tool_dependencies() {
    for changed in [
        "[dependencies]\n\"npm:demo\" = \"9.9.9\"\n",
        "[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
        "[dependencies]\n",
    ] {
        let f = Fixture::new();
        f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
        success(&f.run(&["install"]));
        let bytes = lock(&f);
        f.write("config.hibana.toml", changed);
        error(&f.run(&["install", "--frozen-lockfile"]));
        assert_eq!(lock(&f), bytes);
        assert_eq!(f.installs().lines().count(), 1);
    }
}

#[test]
fn normal_install_updates_changed_requirements_and_removes_stale_tools() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
    );
    success(&f.run(&["install"]));
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"9.9.9\"\n");
    success(&f.run(&["install"]));
    let table: toml::Table = String::from_utf8(lock(&f)).unwrap().parse().unwrap();
    assert_eq!(table["tools"]["npm:demo"]["version"].as_str(), Some("9.9.9"));
    assert!(table["tools"].get("cargo:demo").is_none());
    success(&f.run(&["install", "--frozen-lockfile"]));
}

#[test]
fn nearest_manifest_sets_root_for_install() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
    std::fs::create_dir_all(f.0.join("nested/deep")).unwrap();
    f.write("nested/hibana.toml", "[package]\nname = \"nested\"\n");
    let output = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .arg("install")
        .current_dir(f.0.join("nested/deep"))
        .output()
        .unwrap();
    success(&output);
    assert!(f.installs().is_empty());
    assert!(!f.0.join(LOCK).exists());
    assert!(f.0.join("nested").join(LOCK).exists());
}

#[test]
fn install_finds_parent_config_and_rejects_invalid_arguments() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
    std::fs::create_dir_all(f.0.join("sub/deep")).unwrap();
    success(
        &f.command(env!("CARGO_BIN_EXE_hibana"))
            .arg("install")
            .current_dir(f.0.join("sub/deep"))
            .output()
            .unwrap(),
    );
    assert!(f.0.join(LOCK).exists());
    assert!(!f.0.join("sub/deep").join(LOCK).exists());
    error(&f.run(&["install", "--unknown"]));
    error(&f.run(&["install", "unexpected"]));
}

#[test]
fn install_rejects_missing_config_and_malformed_lock() {
    let f = Fixture::new();
    error(&f.run(&["install"]));
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
    f.write(LOCK, "not valid [ toml");
    for args in [vec!["install"], vec!["install", "--frozen-lockfile"]] {
        error(&f.run(&args));
        assert_eq!(lock(&f), b"not valid [ toml");
        assert!(f.installs().is_empty());
    }
}

fn git(root: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git").current_dir(root).args(args).output().unwrap();
    success(&output).trim().to_owned()
}

fn commit(root: &std::path::Path) -> String {
    git(root, &["add", "."]);
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    git(root, &["rev-parse", "HEAD"])
}

fn repository(f: &Fixture) -> (String, String) {
    let root = f.0.join("source");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("hibana.toml"), "[package]\nname = \"greet\"\n").unwrap();
    std::fs::write(root.join("src/lib.hb"), "pub fn value() -> Int { 1 }").unwrap();
    git(&root, &["init", "-q"]);
    let revision = commit(&root);
    (format!("file://{}", root.display()), revision)
}

#[test]
fn install_combines_native_and_tools_and_frozen_fetches_original_commit() {
    let f = Fixture::new();
    let (url, revision) = repository(&f);
    f.write("hibana.toml", &format!("[dependencies]\ngreet = {{ git = {url:?} }}\n"));
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
    success(&f.run(&["install"]));
    let bytes = lock(&f);
    let table: toml::Table = String::from_utf8(bytes.clone()).unwrap().parse().unwrap();
    assert_eq!(table["greet"]["rev"].as_str(), Some(revision.as_str()));
    assert!(table["tools"]["npm:demo"].is_table());
    std::fs::write(f.0.join("source/src/lib.hb"), "pub fn value() -> Int { 2 }").unwrap();
    let next = commit(&f.0.join("source"));
    assert_ne!(next, revision);
    success(
        &f.command(env!("CARGO_BIN_EXE_hibana"))
            .args(["install", "--frozen-lockfile"])
            .env("XDG_DATA_HOME", f.0.join("fresh-data"))
            .output()
            .unwrap(),
    );
    assert_eq!(lock(&f), bytes);
    let trees = f.0.join("fresh-data/hibana/packages/v1");
    let contents: Vec<_> = std::fs::read_dir(trees)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path().join("src/lib.hb")).unwrap())
        .collect();
    assert_eq!(contents, vec!["pub fn value() -> Int { 1 }"]);
}

#[test]
fn frozen_rejects_mutable_native_lock_revision_and_removed_dependencies() {
    let f = Fixture::new();
    let (url, _) = repository(&f);
    f.write("hibana.toml", &format!("[dependencies]\ngreet = {{ git = {url:?} }}\n"));
    success(&f.run(&["install"]));
    let bytes = lock(&f);
    let mut table: toml::Table = String::from_utf8(bytes.clone()).unwrap().parse().unwrap();
    table["greet"]
        .as_table_mut()
        .unwrap()
        .insert("rev".into(), "HEAD".into());
    f.write(LOCK, &table.to_string());
    let malformed = lock(&f);
    error(&f.run(&["install", "--frozen-lockfile"]));
    assert_eq!(lock(&f), malformed);
    std::fs::write(f.0.join(LOCK), &bytes).unwrap();
    f.write("hibana.toml", "[dependencies]\n");
    error(&f.run(&["install", "--frozen-lockfile"]));
    assert_eq!(lock(&f), bytes);
}

#[test]
fn frozen_rejects_native_revision_url_changes_and_missing_entries() {
    let f = Fixture::new();
    let (url, revision) = repository(&f);
    f.write("hibana.toml", &format!("[dependencies]\ngreet = {{ git = {url:?} }}\n"));
    success(&f.run(&["install"]));
    let bytes = lock(&f);
    for dependencies in [
        format!("greet = {{ git = {url:?}, rev = {revision:?} }}"),
        format!("greet = {{ git = \"{url}/changed\" }}"),
        format!("greet = {{ git = {url:?} }}\nextra = {{ git = {url:?} }}"),
    ] {
        f.write("hibana.toml", &format!("[dependencies]\n{dependencies}\n"));
        error(&f.run(&["install", "--frozen-lockfile"]));
        assert_eq!(lock(&f), bytes);
    }
    f.write(
        "hibana.toml",
        &format!("[dependencies]\ngreet = {{ git = {url:?}, rev = {revision:?} }}\n"),
    );
    success(&f.run(&["install"]));
    let updated = lock(&f);
    assert_ne!(updated, bytes);
    success(&f.run(&["install", "--frozen-lockfile"]));
    assert_eq!(lock(&f), updated);
}

#[test]
fn frozen_rejects_legacy_npm_lock_then_normal_install_upgrades() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
    let old = "# 旧形式\n[tools.\"npm:demo\"]\nrequirement = \"*\"\nversion = \"1.2.3\"\n";
    f.write(LOCK, old);
    error(&f.run(&["install", "--frozen-lockfile"]));
    assert_eq!(lock(&f), old.as_bytes());
    assert!(f.installs().is_empty());
    success(&f.run(&["install"]));
    let saved: toml::Table = String::from_utf8(lock(&f)).unwrap().parse().unwrap();
    assert!(saved["tools"]["npm:demo"]["npm_lock"].as_str().is_some());
    success(&f.run(&["install", "--frozen-lockfile"]));
}

#[test]
fn invalid_npm_integrity_fails_before_install_and_preserves_lock() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[dependencies]\n\"npm:demo\" = \"*\"\n");
    success(&f.run(&["install"]));
    let mut table: toml::Table = String::from_utf8(lock(&f)).unwrap().parse().unwrap();
    let raw = table["tools"]["npm:demo"]["npm_lock"].as_str().unwrap();
    let mut json: serde_json::Value = serde_json::from_str(raw).unwrap();
    json["packages"]["node_modules/transitive"]["integrity"] = serde_json::json!("sha512-not-an-integrity!");
    table["tools"]["npm:demo"]["npm_lock"] = toml::Value::String(json.to_string());
    f.write(LOCK, &toml::to_string(&table).unwrap());
    let before = lock(&f);
    for args in [vec!["install"], vec!["install", "--frozen-lockfile"]] {
        error(&f.run(&args));
        assert_eq!(lock(&f), before);
        assert_eq!(f.installs().lines().count(), 1);
    }
}

#[test]
fn frozen_rejects_legacy_cargo_lock_then_normal_install_upgrades() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[dependencies]\n\"cargo:demo\" = \"*\"\n");
    let old = "# 旧形式\n[tools.\"cargo:demo\"]\nrequirement = \"*\"\nversion = \"2.3.4\"\n";
    f.write(LOCK, old);
    error(&f.run(&["install", "--frozen-lockfile"]));
    assert_eq!(lock(&f), old.as_bytes());
    assert!(f.installs().is_empty());
    success(&f.run(&["install"]));
    let saved: toml::Table = String::from_utf8(lock(&f)).unwrap().parse().unwrap();
    assert!(saved["tools"]["cargo:demo"]["cargo_lock"].as_str().is_some());
    success(&f.run(&["install", "--frozen-lockfile"]));
}

#[test]
fn invalid_cargo_lock_fails_before_install_and_preserves_bytes() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[dependencies]\n\"cargo:demo\" = \"*\"\n");
    success(&f.run(&["install"]));
    let original: toml::Table = String::from_utf8(lock(&f)).unwrap().parse().unwrap();
    let raw = original["tools"]["cargo:demo"]["cargo_lock"].as_str().unwrap();
    let tree: toml::Table = raw.parse().unwrap();
    let mut bad_checksum = tree.clone();
    let package = bad_checksum["package"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["name"].as_str() == Some("transitive"))
        .unwrap();
    package["checksum"] = "not-a-checksum".into();
    let mut bad_source = tree.clone();
    let package = bad_source["package"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["name"].as_str() == Some("transitive"))
        .unwrap();
    package["source"] = "git+https://example.invalid/repo".into();
    for raw in [
        "invalid [ toml".to_owned(),
        toml::to_string(&bad_checksum).unwrap(),
        toml::to_string(&bad_source).unwrap(),
    ] {
        let mut table = original.clone();
        table["tools"]["cargo:demo"]["cargo_lock"] = raw.into();
        f.write(LOCK, &toml::to_string(&table).unwrap());
        let before = lock(&f);
        let log = f.installs();
        for args in [vec!["install"], vec!["install", "--frozen-lockfile"]] {
            error(&f.run(&args));
            assert_eq!(lock(&f), before);
            assert_eq!(f.installs(), log);
        }
    }
}
