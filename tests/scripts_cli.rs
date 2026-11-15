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
fn dev_executes_shell_commands() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\ndev = \"printf dev\"\n");
    assert_eq!(stdout(&fixture.run(&["dev"])), "dev");
}

#[test]
fn scripts_find_parent_config_and_run_in_its_directory() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\ndev = \"cat marker\"\n");
    fixture.write("marker", "project root\n");
    let nested = fixture.0.join("src/nested");
    std::fs::create_dir_all(&nested).unwrap();
    // パッケージ用設定はスクリプトの探索を遮らない。
    fixture.write("src/hibana.toml", "[package]\nname = \"nested\"\n");
    assert_eq!(stdout(&run(&nested, &["dev"])), "project root\n");
}

#[test]
fn scripts_nearest_config_wins_without_falling_back() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\ndev = \"printf outer\"\n");
    let nested = fixture.0.join("nested");
    std::fs::create_dir(&nested).unwrap();
    fixture.write(
        "nested/config.hibana.toml",
        "[scripts]\ndev = \"printf inner\"\n",
    );
    assert_eq!(stdout(&run(&nested, &["dev"])), "inner");
    fixture.write("nested/config.hibana.toml", "[scripts]\n");
    assert_error(&run(&nested, &["dev"]));
}

#[test]
fn dev_preserves_arguments_without_shell_injection() {
    let fixture = Fixture::new();
    fixture.write(
        "config.hibana.toml",
        "[scripts]\ndev = \"printf '<%s>\\n'\"\n",
    );
    let args = [
        "space value",
        "",
        "$(touch injected)",
        "; touch injected",
        "a'b\"c",
        "*",
        "日本語",
    ];
    let mut command = vec!["dev", "--"];
    command.extend(args);
    let expected: String = args.iter().map(|arg| format!("<{arg}>\n")).collect();
    assert_eq!(stdout(&fixture.run(&command)), expected);
    assert!(!fixture.0.join("injected").exists());
}

#[test]
fn dev_accepts_arguments_after_separator() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\ndev = \"printf '<%s>'\"\n");
    assert_eq!(
        stdout(&fixture.run(&["dev", "--", "a b", "--release"])),
        "<a b><--release>"
    );
}

#[test]
fn scripts_preserve_nonzero_exit_status() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\ndev = \"exit 37\"\n");
    assert_eq!(fixture.run(&["dev"]).status.code(), Some(37));
}

#[cfg(unix)]
#[test]
fn scripts_map_signal_exit_to_shell_status() {
    let fixture = Fixture::new();
    fixture.write("config.hibana.toml", "[scripts]\ndev = \"kill -TERM $$\"\n");
    assert_eq!(fixture.run(&["dev"]).status.code(), Some(143));
}

#[test]
fn scripts_reject_missing_invalid_and_undefined_configuration() {
    let fixture = Fixture::new();
    // hibana.toml は config.hibana.toml の代わりにはならない。
    fixture.write("hibana.toml", "[scripts]\ndev = \"printf wrong-config\"\n");
    assert_error(&fixture.run(&["dev"]));
    for config in [
        "[scripts\n",
        "scripts = 42\n",
        "[scripts]\ndev = 42\n",
        "[scripts]\ndev = ['printf', 'invalid']\n",
        "[scripts]\nbuild = \"printf build\"\n",
    ] {
        fixture.write("config.hibana.toml", config);
        assert_error(&fixture.run(&["dev"]));
    }
    fixture.write("config.hibana.toml", "[scripts]\n");
    assert_error(&fixture.run(&["dev"]));
}

#[test]
fn build_runs_script_and_script_command_is_rejected() {
    let fixture = Fixture::new();
    fixture.write(
        "config.hibana.toml",
        "[scripts]\nbuild = \"printf '<%s>'\"\ncheck = \"touch script-ran\"\n",
    );
    fixture.write("build", "must not execute");
    assert_eq!(stdout(&fixture.run(&["build", "--", "a b", "$(touch injected)"])), "<a b><$(touch injected)>");
    assert!(!fixture.0.join("injected").exists());
    fixture.write("config.hibana.toml", "[scripts]\n");
    assert_error(&fixture.run(&["build"]));
    fixture.write("script", "println(\"must not execute\")\n");
    for args in [
        vec!["script"],
        vec!["script", "build"],
        vec!["script", "check", "--", "arg"],
    ] {
        let result = fixture.run(&args);
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        assert!(!result.stderr.is_empty());
        assert!(result.stdout.is_empty());
        assert!(!fixture.0.join("script-ran").exists());
    }
}
