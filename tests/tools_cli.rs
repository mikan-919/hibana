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
while [ "$#" -gt 0 ]; do
  case "$1" in
    --prefix) prefix="$2"; shift 2 ;;
    demo|demo@*|many|many@*|@scope/demo|@scope/demo@*) package="$1"; shift ;;
    *) shift ;;
  esac
done
[ -n "$prefix" ]
printf 'npm:%s\n' "$package" >> "$MOCK_LOG"
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
root="$prefix/node_modules/$name"
mkdir -p "$root"
if [ "$name" = many ]; then
  printf '%s' '{"name":"many","version":"1.0.0","bin":{"first":"cli.js","second":"cli.js"}}' > "$root/package.json"
else
  printf '{"name":"%s","version":"%s","bin":{"demo":"cli.js"}}' "$name" "$version" > "$root/package.json"
fi
cat > "$root/cli.js" <<'CLI'
#!/bin/sh
if [ "${1-}" = --fail ]; then exit 37; fi
printf npm-demo
for arg in "$@"; do printf '<%s>' "$arg"; done
CLI
chmod +x "$root/cli.js"
"##;

// Cargoのビルド成果物はnpmと同名でも別の内容にする。
const CARGO: &str = r##"#!/bin/sh
set -eu
root=''
version=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --root) root="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$root" ]
printf 'cargo:demo%s\n' "${version:+@$version}" >> "$MOCK_LOG"
mkdir -p "$root/bin"
printf '[v1]\n"demo 2.3.4 (registry+https://example.invalid/index)" = ["demo"]\n' > "$root/.crates.toml"
cat > "$root/bin/demo" <<'CLI'
#!/bin/sh
printf cargo-demo
for arg in "$@"; do printf '<%s>' "$arg"; done
CLI
chmod +x "$root/bin/demo"
"##;

#[test]
fn add_records_dependency_and_preserves_scripts() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[scripts]\nbuild = \"demo --build\"\n");
    success(&f.run(&["add", "npm:demo"]));
    let config = f.config();
    assert_eq!(config["dependencies"]["npm:demo"].as_str(), Some("*"));
    assert_eq!(config["scripts"]["build"].as_str(), Some("demo --build"));
    assert_eq!(success(&f.run(&["build"])), "npm-demo<--build>");
    assert_eq!(f.installs().lines().count(), 1);
}

#[test]
fn add_creates_config_and_records_explicit_version() {
    let f = Fixture::new();
    success(&f.run(&["add", "npm:demo@1.2.3"]));
    assert_eq!(f.config()["dependencies"]["npm:demo"].as_str(), Some("1.2.3"));
    assert!(f.installs().contains("npm:demo@1.2.3"));
}

#[test]
fn dlx_preserves_arguments_without_persisting_dependency() {
    let f = Fixture::new();
    let output = f.run(&["dlx", "npm:demo", "--", "a b", "", "$(touch injected)", "--flag"]);
    assert_eq!(success(&output), "npm-demo<a b><><$(touch injected)><--flag>");
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
    assert_eq!(f.run(&["dlx", "npm:demo", "--fail"]).status.code(), Some(37));
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
    assert_eq!(f.run(&["dlx", "npm:demo", "--", "--fail"]).status.code(), Some(37));
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
    assert_eq!(success(&f.run(&["dlx", "npm:@scope/demo@1.2.3"])), "npm-demo");
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
        "[scripts]\nbuild = \"touch script-ran\"\n[dependencies]\n\"npm:demo\" = \"*\"\n\"cargo:demo\" = \"*\"\n",
    );
    error(&f.run(&["build"]));
    assert!(!f.0.join("script-ran").exists());
}

#[test]
fn rejected_identifiers_do_not_spawn_installers() {
    let f = Fixture::new();
    for name in ["unknown:demo", "npm:", "cargo:../demo", "npm:demo;touch injected"] {
        error(&f.run(&["dlx", name]));
    }
    assert!(f.installs().is_empty());
    assert!(!f.0.join("injected").exists());
}

fn lock(f: &Fixture) -> toml::Table {
    std::fs::read_to_string(f.0.join("hibana.lock"))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn project_lock_pins_npm_across_data_homes_and_registry_changes() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[scripts]\nbuild = \"demo\"\n");
    f.write(
        "hibana.lock",
        "[greet]\ngit = \"file:///preserved\"\nrev = \"abc123\"\n",
    );
    success(&f.run(&["add", "npm:demo"]));
    let initial = std::fs::read_to_string(f.0.join("hibana.lock")).unwrap();
    let saved = lock(&f);
    assert_eq!(saved["tools"]["npm:demo"]["requirement"].as_str(), Some("*"));
    assert_eq!(saved["tools"]["npm:demo"]["version"].as_str(), Some("1.2.3"));
    assert_eq!(saved["greet"]["rev"].as_str(), Some("abc123"));
    let output = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("XDG_DATA_HOME", f.0.join("other-data"))
        .env("MOCK_NPM_VERSION", "9.9.9")
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(success(&output), "npm-demo");
    assert!(f.installs().contains("npm:demo@1.2.3"));
    assert_eq!(std::fs::read_to_string(f.0.join("hibana.lock")).unwrap(), initial);
    assert_eq!(f.installs().lines().count(), 2);
}

#[test]
fn project_lock_uses_exact_cargo_requirement_on_new_machine() {
    let f = Fixture::new();
    f.write("config.hibana.toml", "[scripts]\nbuild = \"demo\"\n");
    success(&f.run(&["add", "cargo:demo"]));
    let saved = lock(&f);
    assert_eq!(saved["tools"]["cargo:demo"]["requirement"].as_str(), Some("*"));
    assert_eq!(saved["tools"]["cargo:demo"]["version"].as_str(), Some("2.3.4"));
    let output = f
        .command(env!("CARGO_BIN_EXE_hibana"))
        .env("XDG_DATA_HOME", f.0.join("other-data"))
        .arg("build")
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
        "[scripts]\nbuild = \"demo\"\n[dependencies]\n\"npm:demo\" = \"9.9.9\"\n",
    );
    assert_eq!(success(&f.run(&["build"])), "npm-demo");
    let saved = lock(&f);
    assert_eq!(saved["tools"]["npm:demo"]["requirement"].as_str(), Some("9.9.9"));
    assert_eq!(saved["tools"]["npm:demo"]["version"].as_str(), Some("9.9.9"));
    assert_eq!(f.installs().lines().count(), 2);
}

#[test]
fn malformed_project_lock_fails_before_script_and_install() {
    let f = Fixture::new();
    f.write(
        "config.hibana.toml",
        "[scripts]\nbuild = \"touch script-ran\"\n[dependencies]\n\"npm:demo\" = \"*\"\n",
    );
    for text in [
        "not valid toml = [",
        "[tools]\n\"npm:demo\" = { requirement = \"*\" }\n",
        "[tools]\n\"npm:demo\" = { requirement = \"*\", version = 123 }\n",
    ] {
        f.write("hibana.lock", text);
        error(&f.run(&["build"]));
        assert!(!f.0.join("script-ran").exists());
        assert!(f.installs().is_empty());
        assert_eq!(std::fs::read_to_string(f.0.join("hibana.lock")).unwrap(), text);
    }
}

#[test]
fn project_add_rejects_native_tools_name_collision_without_overwriting_lock() {
    let f = Fixture::new();
    let old = "[tools]\ngit = \"file:///native-tools\"\nrev = \"abc\"\n";
    f.write("hibana.lock", old);
    error(&f.run(&["add", "npm:demo"]));
    assert_eq!(std::fs::read_to_string(f.0.join("hibana.lock")).unwrap(), old);
}
