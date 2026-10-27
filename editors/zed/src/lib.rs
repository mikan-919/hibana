// Zed の拡張機能。PATH にある hibana を hibana lsp として起動する
use zed_extension_api as zed;

struct Hibana;

impl zed::Extension for Hibana {
    fn new() -> Self {
        Hibana
    }

    fn language_server_command(
        &mut self,
        _id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let command = worktree
            .which("hibana")
            .ok_or("hibana が PATH に無い（cargo install --path . で入れる）")?;
        Ok(zed::Command {
            command,
            args: vec!["lsp".into()],
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(Hibana);
