use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hibana-scripts-cli-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        run(&self.0, args)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hibana"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    assert!(
        output.status.success(),
        "終了状態: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    std::str::from_utf8(&output.stdout).unwrap()
}

fn assert_error(output: &Output) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!output.stderr.is_empty(), "診断がない: {output:?}");
}

#[test]
fn scripts_build_and_dev_execute_shell_commands() {
    let fixture = Fixture::new();
    fixture.write(
        "config.hibana.toml",
        "[scripts]\nbuild = \"printf build\"\ndev = \"printf dev\"\n",
    );
    assert_eq!(stdout(&fixture.run(&["build"])), "build");
    assert_eq!(stdout(&fixture.run(&["dev"])), "dev");
}

#[test]
fn scripts_find_parent_config_and_run_in_its_directory() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\nbuild = \"cat marker\"\n");
    fixture.write("marker", "project root\n");
    let nested = fixture.0.join("src/nested");
    std::fs::create_dir_all(&nested).unwrap();
    // パッケージ用設定はスクリプトの探索を遮らない。
    fixture.write("src/hibana.toml", "[package]\nname = \"nested\"\n");
    assert_eq!(stdout(&run(&nested, &["build"])), "project root\n");
}

#[test]
fn scripts_nearest_config_wins_without_falling_back() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\nbuild = \"printf outer\"\n");
    let nested = fixture.0.join("nested");
    std::fs::create_dir(&nested).unwrap();
    fixture.write("nested/config.hibana.toml", "[scripts]\nbuild = \"printf inner\"\n");
    assert_eq!(stdout(&run(&nested, &["build"])), "inner");
    fixture.write("nested/config.hibana.toml", "[scripts]\ndev = \"printf inner\"\n");
    assert_error(&run(&nested, &["build"]));
}

#[test]
fn scripts_named_command_preserves_arguments_without_shell_injection() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\ncheck = \"printf '<%s>\\n'\"\n");
    let args = [
        "space value",
        "",
        "$(touch injected)",
        "; touch injected",
        "a'b\"c",
        "*",
        "日本語",
    ];
    let mut command = vec!["script", "check", "--"];
    command.extend(args);
    let expected: String = args.iter().map(|arg| format!("<{arg}>\n")).collect();
    assert_eq!(stdout(&fixture.run(&command)), expected);
    assert!(!fixture.0.join("injected").exists());
}

#[test]
fn scripts_build_accepts_arguments_after_separator() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\nbuild = \"printf '<%s>'\"\n");
    assert_eq!(
        stdout(&fixture.run(&["build", "--", "a b", "--release"])),
        "<a b><--release>"
    );
}

#[test]
fn scripts_preserve_nonzero_exit_status() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\nbuild = \"exit 37\"\n");
    assert_eq!(fixture.run(&["build"]).status.code(), Some(37));
}

#[cfg(unix)]
#[test]
fn scripts_map_signal_exit_to_shell_status() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\nbuild = \"kill -TERM $$\"\n");
    assert_eq!(fixture.run(&["build"]).status.code(), Some(143));
}

#[test]
fn scripts_reject_missing_invalid_and_undefined_configuration() {
    let fixture = Fixture::new();
    // hibana.toml は config.hibana.toml の代わりにはならない。
    fixture.write("hibana.toml", "[scripts]\nbuild = \"printf wrong-config\"\n");
    assert_error(&fixture.run(&["build"]));
    for config in [
        "[scripts\n",
        "scripts = 42\n",
        "[scripts]\nbuild = 42\n",
        "[scripts]\nbuild = ['printf', 'invalid']\n",
        "[scripts]\ndev = \"printf dev\"\n",
    ] {
        fixture.write("config.hibana.toml", config);
        assert_error(&fixture.run(&["build"]));
    }
    fixture.write("config.hibana.toml", "[scripts]\nbuild = \"printf build\"\n");
    assert_error(&fixture.run(&["script", "missing"]));
}

#[test]
fn scripts_do_not_override_explicit_legacy_build_input() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\nbuild = \"touch script-ran\"\n");
    let result = fixture.run(&["build", "missing.hb"]);
    assert!(!result.status.success());
    assert!(!result.stderr.is_empty());
    assert!(!fixture.0.join("script-ran").exists());
}
