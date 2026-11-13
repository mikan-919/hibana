use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hibana-dlx-cli-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        Self(root)
    }

    fn package(&self, name: &str, main: Option<&str>) -> PathBuf {
        let root = self.0.join(name);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("hibana.toml"), format!("[package]\nname = {name:?}\n")).unwrap();
        if let Some(source) = main {
            std::fs::write(root.join("src/main.hb"), source).unwrap();
        }
        git(&root, &["init", "-q"]);
        commit(&root);
        root
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hibana"))
            .args(args)
            .current_dir(self.0.join("cwd"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("HOME", self.0.join("home"))
            .output()
            .unwrap()
    }

    fn check_clean(&self) {
        assert!(!self.0.join("cwd/config.hibana.lock.toml").exists());
        assert!(!self.0.join("cwd/hibana.toml").exists());
        assert!(!self.0.join("cwd/config.hibana.toml").exists());
        let tmp = self.0.join("data/hibana/tmp");
        if tmp.exists() {
            assert_eq!(std::fs::read_dir(tmp).unwrap().count(), 0);
        }
        let packages = self.0.join("data/hibana/packages/v1");
        if packages.exists() {
            for tree in std::fs::read_dir(packages).unwrap() {
                let tree = tree.unwrap().path();
                assert!(
                    !tree.join("config.hibana.lock.toml").exists(),
                    "ストアへlockを書いた: {}",
                    tree.display()
                );
                if tree.join("src/main.hb").exists() {
                    assert!(
                        std::fs::metadata(tree.join("src/main.hb"))
                            .unwrap()
                            .permissions()
                            .readonly()
                    );
                }
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").current_dir(root).args(args).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(root: &Path) -> String {
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

fn stdout(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn error(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(!text.is_empty());
    text
}

#[test]
fn hibana_dlx_accepts_prefix_bare_url_args_and_offline_reuse() {
    let f = Fixture::new();
    let source = f.package("app", Some("fn main() { print(\"native dlx\") }\n"));
    let url = format!("file://{}", source.display());
    let spec = format!("hibana:{url}");
    assert_eq!(
        stdout(&f.run(&["dlx", &spec, "--version", "--bin", "ignored", "a b"])),
        "native dlx\n"
    );
    std::fs::rename(&source, f.0.join("source-offline")).unwrap();
    assert_eq!(stdout(&f.run(&["dlx", &url, "--", "--rev", "HEAD"])), "native dlx\n");
    f.check_clean();
}

#[test]
fn hibana_dlx_resolves_dependencies_without_mutating_store() {
    let f = Fixture::new();
    let dep = f.package("greet", None);
    std::fs::write(dep.join("src/lib.hb"), "fn message() -> str { \"from dependency\" }\n").unwrap();
    commit(&dep);
    let source = f.package("app", Some("use greet\nfn main() { print(greet.message()) }\n"));
    std::fs::write(
        source.join("hibana.toml"),
        format!(
            "[package]\nname = \"app\"\n[dependencies]\ngreet = {{ git = \"file://{}\" }}\n",
            dep.display()
        ),
    )
    .unwrap();
    commit(&source);
    let spec = format!("hibana:file://{}", source.display());
    assert_eq!(stdout(&f.run(&["dlx", &spec])), "from dependency\n");
    std::fs::rename(&source, f.0.join("app-offline")).unwrap();
    std::fs::rename(&dep, f.0.join("greet-offline")).unwrap();
    assert_eq!(stdout(&f.run(&["dlx", &spec])), "from dependency\n");
    f.check_clean();
}

#[test]
fn hibana_dlx_revision_selection_is_isolated() {
    let f = Fixture::new();
    let source = f.package("app", Some("fn main() { print(\"first\") }\n"));
    let first = git(&source, &["rev-parse", "HEAD"]);
    std::fs::write(source.join("src/main.hb"), "fn main() { print(\"second\") }\n").unwrap();
    let second = commit(&source);
    let spec = format!("hibana:file://{}", source.display());
    assert_eq!(stdout(&f.run(&["dlx", "--rev", &first, &spec])), "first\n");
    assert_eq!(stdout(&f.run(&["dlx", "--rev", &second, &spec])), "second\n");
    assert_eq!(stdout(&f.run(&["dlx", "--rev", &first, &spec])), "first\n");
    f.check_clean();
}

#[test]
fn hibana_dlx_reports_missing_entry_manifest_revision_and_bin() {
    let f = Fixture::new();
    let source = f.package("library", None);
    let spec = format!("hibana:file://{}", source.display());
    assert!(error(&f.run(&["dlx", &spec])).contains("main.hb"));
    error(&f.run(&["dlx", "--bin", "library", &spec]));
    error(&f.run(&["dlx", "--rev", "missing-revision", &spec]));
    let missing = f.package("no-manifest", Some("fn main() { print(\"wrong\") }\n"));
    std::fs::remove_file(missing.join("hibana.toml")).unwrap();
    commit(&missing);
    let spec = format!("hibana:file://{}", missing.display());
    assert!(error(&f.run(&["dlx", &spec])).contains("hibana.toml"));
    f.check_clean();
}

#[test]
fn hibana_dlx_propagates_program_error() {
    let f = Fixture::new();
    let source = f.package("broken", Some("fn main() { missing_function() }\n"));
    let spec = format!("hibana:file://{}", source.display());
    error(&f.run(&["dlx", &spec]));
    f.check_clean();
}
