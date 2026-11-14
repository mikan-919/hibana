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
            "hibana-tools-cli-{}-{}",
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
        std::fs::set_permissions(self.0.join(path), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    fn command(&self, executable: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(executable);
        let mut paths = vec![self.0.join("mock-bin")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
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
        self.command(env!("CARGO_BIN_EXE_hibana"))
            .args(args)
            .output()
            .unwrap()
    }

    fn config(&self) -> toml::Table {
        std::fs::read_to_string(self.0.join("config.hibana.toml"))
            .unwrap()
            .parse()
            .unwrap()
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

#[test]
fn add_records_dependency_and_preserves_scripts() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[scripts]\ndev = \"demo --build\"\n");
    success(&f.run(&["add", "npm:demo"]));
    let config = f.config();
    assert_eq!(config["dependencies"]["npm:demo"].as_str(), Some("*"));
    assert_eq!(config["scripts"]["dev"].as_str(), Some("demo --build"));
    assert_eq!(success(&f.run(&["dev"])), "npm-demo<--build>");
    assert_eq!(f.installs().lines().count(), 1);
}

#[test]
fn add_creates_config_and_records_explicit_version() {
    let f = Fixture::new();
    success(&f.run(&["add", "npm:demo@1.2.3"]));
    assert_eq!(
        f.config()["dependencies"]["npm:demo"].as_str(),
        Some("1.2.3")
    );
    assert!(f.installs().contains("npm:demo@1.2.3"));
}

#[test]
fn dlx_preserves_arguments_without_persisting_dependency() {
    let f = Fixture::new();
    let output = f.run(&[
        "dlx",
        "npm:demo",
        "--",
        "a b",
        "",
        "$(touch injected)",
        "--flag",
    ]);
    assert_eq!(
        success(&output),
        "npm-demo<a b><><$(touch injected)><--flag>"
    );
    assert!(!f.0.join("injected").exists());
    assert!(!f.0.join("config.hibana.toml").exists());
}

#[test]
fn dlx_forwards_all_options_after_spec_without_separator() {
    let f = Fixture::new();
    assert_eq!(
        success(&f.run(&[
            "dlx",
            "npm:demo",
            "--version",
            "--bin",
            "second",
            "--rev",
            "HEAD",
            "--",
            "tail"
        ])),
        "npm-demo<--version><--bin><second><--rev><HEAD><--><tail>"
    );
    assert_eq!(
        success(&f.run(&["dlx", "--bin", "second", "npm:many", "--bin", "first"])),
        "npm-demo<--bin><first>"
    );
    assert_eq!(
        success(&f.run(&["dlx", "npm:demo", "--", "--", "tail"])),
        "npm-demo<--><tail>"
    );
    assert_eq!(
        f.run(&["dlx", "npm:demo", "--fail"]).status.code(),
        Some(37)
    );
}

#[test]
fn dlx_rejects_missing_or_provider_incompatible_options() {
    let f = Fixture::new();
    for args in [
        vec!["dlx"],
        vec!["dlx", "--bin"],
        vec!["dlx", "--rev"],
        vec!["dlx", "--unknown", "npm:demo"],
        vec!["dlx", "--rev", "HEAD", "npm:demo"],
        vec!["dlx", "--rev", "HEAD", "cargo:demo"],
    ] {
        error(&f.run(&args));
    }
    assert!(f.installs().is_empty());
}

#[test]
fn dlx_reuses_install_and_preserves_exit_code() {
    let f = Fixture::new();
    assert_eq!(success(&f.run(&["dlx", "npm:demo"])), "npm-demo");
    std::fs::remove_file(f.0.join("mock-bin/npm")).unwrap();
    assert_eq!(success(&f.run(&["dlx", "npm:demo"])), "npm-demo");
    assert_eq!(
        f.run(&["dlx", "npm:demo", "--", "--fail"]).status.code(),
        Some(37)
    );
    assert_eq!(f.installs().lines().count(), 1);
}

#[test]
fn npm_and_cargo_identifiers_do_not_collide() {
    let f = Fixture::new();
    assert_eq!(success(&f.run(&["dlx", "npm:demo"])), "npm-demo");
    assert_eq!(success(&f.run(&["dlx", "cargo:demo"])), "cargo-demo");
    assert_eq!(success(&f.run(&["dlx", "npm:demo"])), "npm-demo");
    assert_eq!(f.installs().lines().count(), 2);
}

#[test]
fn global_add_exposes_command_under_data_home() {
    let f = Fixture::new();
    success(&f.run(&["add", "-g", "npm:demo"]));
    let shim = f.0.join("data/hibana/bin/demo");
    assert!(shim.exists());
    let output = f.command(&shim).arg("a b").output().unwrap();
    assert_eq!(success(&output), "npm-demo<a b>");
    assert!(!f.0.join("config.hibana.toml").exists());
}

#[test]
fn dlx_requires_selection_for_ambiguous_bins() {
    let f = Fixture::new();
    error(&f.run(&["dlx", "npm:many"]));
    assert_eq!(
        success(&f.run(&["dlx", "--bin", "second", "npm:many", "--", "ok"])),
        "npm-demo<ok>"
    );
    error(&f.run(&["dlx", "--bin", "missing", "npm:many"]));
    assert_eq!(f.installs().lines().count(), 1);
}

#[test]
fn dlx_accepts_scoped_npm_name_with_version() {
    let f = Fixture::new();
    assert_eq!(
        success(&f.run(&["dlx", "npm:@scope/demo@1.2.3"])),
        "npm-demo"
    );
    assert!(f.installs().contains("npm:@scope/demo@1.2.3"));
}

#[test]
fn global_add_does_not_overwrite_other_provider_command() {
    let f = Fixture::new();
    success(&f.run(&["add", "-g", "npm:demo"]));
    error(&f.run(&["add", "-g", "cargo:demo"]));
    let shim = f.0.join("data/hibana/bin/demo");
    assert_eq!(success(&f.command(shim).output().unwrap()), "npm-demo");
}

#[test]
fn scripts_reject_colliding_bin_names_before_running() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"touch script-ran\"\n[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
    );
    error(&f.run(&["dev"]));
    assert!(!f.0.join("script-ran").exists());
}

#[test]
fn rejected_identifiers_do_not_spawn_installers() {
    let f = Fixture::new();
    for name in [
        "unknown:demo",
        "npm:",
        "cargo:../demo",
        "npm:demo;touch injected",
    ] {
        error(&f.run(&["dlx", name]));
    }
    assert!(f.installs().is_empty());
    assert!(!f.0.join("injected").exists());
}

fn lock(f: &Fixture) -> toml::Table {
    std::fs::read_to_string(f.0.join("config.hibana.lock.toml"))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn project_lock_pins_npm_across_data_homes_and_registry_changes() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[scripts]\ndev = \"demo\"\n");
    f.write(
        "config.hibana.lock.toml",
        "[greet]\ngit = \"file:///preserved\"\nrev = \"abc123\"\n",
    );
    success(&f.run(&["add", "npm:demo"]));
    let initial = std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap();
    let saved = lock(&f);
    assert_eq!(
        saved["tools"]["npm:demo"]["requirement"].as_str(),
        Some("*")
    );
    assert_eq!(
        saved["tools"]["npm:demo"]["version"].as_str(),
        Some("1.2.3")
    );
    assert_eq!(saved["greet"]["rev"].as_str(), Some("abc123"));
    let output = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("XDG_DATA_HOME", f.0.join("other-data"))
        .env("MOCK_NPM_VERSION", "9.9.9")
        .args(["dev"])
        .output()
        .unwrap();
    assert_eq!(success(&output), "npm-demo");
    assert!(f.installs().contains("npm:demo@1.2.3"));
    assert_eq!(
        std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap(),
        initial
    );
    assert_eq!(f.installs().lines().count(), 2);
}

#[test]
fn project_lock_uses_exact_cargo_requirement_on_new_machine() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[scripts]\ndev = \"demo\"\n");
    success(&f.run(&["add", "cargo:demo"]));
    let saved = lock(&f);
    assert_eq!(
        saved["tools"]["cargo:demo"]["requirement"].as_str(),
        Some("*")
    );
    assert_eq!(
        saved["tools"]["cargo:demo"]["version"].as_str(),
        Some("2.3.4")
    );
    let output = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("XDG_DATA_HOME", f.0.join("other-data"))
        .args(["dev"])
        .output()
        .unwrap();
    assert_eq!(success(&output), "cargo-demo");
    assert!(f.installs().contains("cargo:demo@=2.3.4"));
}

#[test]
fn changed_project_requirement_re_resolves_lock() {
    let f = Fixture::new();
    success(&f.run(&["add", "npm:demo"]));
    f.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"demo\"\n[dependencies]\n\"npm:demo\" = \"9.9.9\"\n",
    );
    assert_eq!(success(&f.run(&["dev"])), "npm-demo");
    let saved = lock(&f);
    assert_eq!(
        saved["tools"]["npm:demo"]["requirement"].as_str(),
        Some("9.9.9")
    );
    assert_eq!(
        saved["tools"]["npm:demo"]["version"].as_str(),
        Some("9.9.9")
    );
    assert_eq!(f.installs().lines().count(), 2);
}

#[test]
fn malformed_project_lock_fails_before_script_and_install() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"touch script-ran\"\n[dependencies]\n\"npm:demo\" = \"*\"\n",
    );
    for text in [
        "not valid toml = [",
        "[tools]\n\"npm:demo\" = { requirement = \"*\" }\n",
        "[tools]\n\"npm:demo\" = { requirement = \"*\", version = 123 }\n",
    ] {
        f.write("config.hibana.lock.toml", text);
        error(&f.run(&["dev"]));
        assert!(!f.0.join("script-ran").exists());
        assert!(f.installs().is_empty());
        assert_eq!(
            std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap(),
            text
        );
    }
}

#[test]
fn project_add_rejects_native_tools_name_collision_without_overwriting_lock() {
    let f = Fixture::new();
    let old = "[tools]\ngit = \"file:///native-tools\"\nrev = \"abc\"\n";
    f.write("config.hibana.lock.toml", old);
    error(&f.run(&["add", "npm:demo"]));
    assert_eq!(
        std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap(),
        old
    );
}

#[test]
fn npm_tree_lock_restores_transitive_version_with_ci() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"demo --transitive\"\n",
    );
    success(&f.run(&["add", "npm:demo"]));
    let before = std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap();
    let saved = lock(&f);
    let raw = saved["tools"]["npm:demo"]["npm_lock"].as_str().unwrap();
    let json: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(
        json["packages"]["node_modules/transitive"]["version"],
        "4.5.6"
    );
    let out = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("XDG_DATA_HOME", f.0.join("fresh-data"))
        .env("MOCK_TRANSITIVE_VERSION", "9.9.9")
        .args(["dev"])
        .output()
        .unwrap();
    assert_eq!(success(&out), "4.5.6");
    assert!(f.installs().contains("npm:demo@1.2.3:ci"));
    assert_eq!(
        std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap(),
        before
    );
}

#[test]
fn distinct_npm_trees_with_same_direct_version_do_not_share_cache() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"demo --transitive\"\n",
    );
    success(&f.run(&["add", "npm:demo"]));
    assert_eq!(success(&f.run(&["dev"])), "4.5.6");
    let original = lock(&f);
    let mut changed = original.clone();
    let raw = changed["tools"]["npm:demo"]["npm_lock"].as_str().unwrap();
    let new_raw = raw.replace("4.5.6", "7.8.9");
    changed["tools"]["npm:demo"]["npm_lock"] = toml::Value::String(new_raw);
    f.write(
        "config.hibana.lock.toml",
        &toml::to_string(&changed).unwrap(),
    );
    assert_eq!(success(&f.run(&["dev"])), "7.8.9");
    f.write(
        "config.hibana.lock.toml",
        &toml::to_string(&original).unwrap(),
    );
    assert_eq!(success(&f.run(&["dev"])), "4.5.6");
    assert_eq!(f.installs().lines().count(), 2);
}

#[test]
fn cargo_tree_lock_restores_transitive_version_with_locked_build() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"demo --transitive\"\n",
    );
    success(&f.run(&["add", "cargo:demo"]));
    let before = std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap();
    let saved = lock(&f);
    let raw = saved["tools"]["cargo:demo"]["cargo_lock"].as_str().unwrap();
    let tree: toml::Table = raw.parse().unwrap();
    assert!(tree["package"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |p| p["name"].as_str() == Some("transitive") && p["version"].as_str() == Some("4.5.6")
        ));
    success(
        &f.command(env!("CARGO_BIN_EXE_hibana"))
            .env("XDG_DATA_HOME", f.0.join("fresh-data"))
            .env("MOCK_CARGO_TRANSITIVE_VERSION", "9.9.9")
            .args(["install", "--frozen-lockfile"])
            .output()
            .unwrap(),
    );
    let output = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("XDG_DATA_HOME", f.0.join("fresh-data"))
        .env("MOCK_CARGO_TRANSITIVE_VERSION", "9.9.9")
        .args(["dev"])
        .output()
        .unwrap();
    assert_eq!(success(&output), "4.5.6");
    assert_eq!(
        std::fs::read_to_string(f.0.join("config.hibana.lock.toml")).unwrap(),
        before
    );
}

#[test]
fn distinct_cargo_trees_with_same_direct_version_do_not_share_cache() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"demo --transitive\"\n",
    );
    success(&f.run(&["add", "cargo:demo"]));
    assert_eq!(success(&f.run(&["dev"])), "4.5.6");
    let original = lock(&f);
    let mut changed = original.clone();
    let raw = changed["tools"]["cargo:demo"]["cargo_lock"]
        .as_str()
        .unwrap();
    changed["tools"]["cargo:demo"]["cargo_lock"] =
        toml::Value::String(raw.replace("4.5.6", "7.8.9"));
    f.write(
        "config.hibana.lock.toml",
        &toml::to_string(&changed).unwrap(),
    );
    assert_eq!(success(&f.run(&["dev"])), "7.8.9");
    f.write(
        "config.hibana.lock.toml",
        &toml::to_string(&original).unwrap(),
    );
    assert_eq!(success(&f.run(&["dev"])), "4.5.6");
    assert_eq!(f.installs().lines().count(), 2);
}

#[test]
fn update_refreshes_only_named_project_tool_trees() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
    );
    success(&f.run(&["install"]));
    let mut original = lock(&f);
    let mut native = toml::Table::new();
    native.insert("git".into(), "file:///preserved".into());
    native.insert("rev".into(), "abc123".into());
    original.insert("greet".into(), native.into());
    f.write(
        "config.hibana.lock.toml",
        &toml::to_string(&original).unwrap(),
    );

    let npm = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("MOCK_NPM_VERSION", "9.9.9")
        .env("MOCK_TRANSITIVE_VERSION", "8.8.8")
        .args(["update", "npm:demo"])
        .output()
        .unwrap();
    success(&npm);
    let after_npm = lock(&f);
    assert_eq!(
        after_npm["tools"]["npm:demo"]["version"].as_str(),
        Some("9.9.9")
    );
    let npm_tree: serde_json::Value =
        serde_json::from_str(after_npm["tools"]["npm:demo"]["npm_lock"].as_str().unwrap()).unwrap();
    assert_eq!(
        npm_tree["packages"]["node_modules/transitive"]["version"],
        "8.8.8"
    );
    assert_eq!(
        after_npm["tools"]["cargo:demo"],
        original["tools"]["cargo:demo"]
    );
    assert_eq!(after_npm["greet"], original["greet"]);

    let cargo = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("MOCK_CARGO_TRANSITIVE_VERSION", "7.7.7")
        .args(["update", "cargo:demo"])
        .output()
        .unwrap();
    success(&cargo);
    let after_cargo = lock(&f);
    assert_eq!(
        after_cargo["tools"]["npm:demo"],
        after_npm["tools"]["npm:demo"]
    );
    assert_eq!(after_cargo["greet"], original["greet"]);
    let cargo_tree: toml::Table = after_cargo["tools"]["cargo:demo"]["cargo_lock"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(cargo_tree["package"]
        .as_array()
        .unwrap()
        .iter()
        .any(|package| {
            package["name"].as_str() == Some("transitive")
                && package["version"].as_str() == Some("7.7.7")
        }));
}

#[test]
fn failed_project_tool_update_preserves_lock_bytes() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
    );
    success(&f.run(&["install"]));
    let before = std::fs::read(f.0.join("config.hibana.lock.toml")).unwrap();
    f.write("mock-bin/npm", "#!/bin/sh\nexit 23\n");
    error(&f.run(&["update", "npm:demo"]));
    assert_eq!(
        std::fs::read(f.0.join("config.hibana.lock.toml")).unwrap(),
        before
    );
    f.write("mock-bin/cargo", "#!/bin/sh\nexit 24\n");
    error(&f.run(&["update", "cargo:demo"]));
    assert_eq!(
        std::fs::read(f.0.join("config.hibana.lock.toml")).unwrap(),
        before
    );
}

#[test]
fn project_tool_update_requires_declared_unversioned_identifier() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[dependencies]\n\"npm:demo\" = \"*\"\n",
    );
    success(&f.run(&["install"]));
    let before = std::fs::read(f.0.join("config.hibana.lock.toml")).unwrap();
    for args in [
        vec!["update", "npm:missing"],
        vec!["update", "npm:demo@1.2.3"],
        vec!["update", "npm:../demo"],
    ] {
        error(&f.run(&args));
        assert_eq!(
            std::fs::read(f.0.join("config.hibana.lock.toml")).unwrap(),
            before
        );
    }
}
