// config.hibana.toml の [scripts] をシェルで実行する。
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG: &str = "config.hibana.toml";

fn find_config(start: &Path) -> Result<PathBuf, String> {
    for dir in start.ancestors() {
        let path = dir.join(CONFIG);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Err(format!("{CONFIG} が無い（[scripts] にコマンドを定義してください）"))
}

pub(crate) fn run(name: &str, args: &[String]) -> Result<i32, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("現在のディレクトリを読めない: {e}"))?;
    let path = find_config(&cwd)?;
    let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let config: toml::Table = source.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let scripts = config
        .get("scripts")
        .and_then(|v| v.as_table())
        .ok_or_else(|| format!("{}: [scripts] テーブルが必要", path.display()))?;
    for (key, value) in scripts {
        if !value.is_str() {
            return Err(format!(
                "{}: scripts.{key} はシェルコマンドの文字列が必要",
                path.display()
            ));
        }
    }
    let script = scripts
        .get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{}: スクリプト {name} が未定義", path.display()))?;
    // 引数をシェルのソースへ埋め込まない。空白やメタ文字を含む引数も一つの値として渡す。
    let command = if args.is_empty() {
        script.to_string()
    } else {
        format!("{script} \"$@\"")
    };
    // ponytail: PATH は親プロセスを継承する。依存パッケージの bin 解決は未実装。
    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .arg(name)
        .args(args)
        .current_dir(path.parent().unwrap())
        .status()
        .map_err(|e| format!("スクリプト {name} のシェルを起動できない: {e}"))?;
    if let Some(code) = status.code() {
        return Ok(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        Ok(128 + status.signal().unwrap_or(1))
    }
    #[cfg(not(unix))]
    {
        Ok(1)
    }
}
