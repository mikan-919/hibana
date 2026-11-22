//! 依存ファイルの読み込みとモジュール表。
use super::parser::{LIBS, PRELUDE_NAMES, Use, base_dir};
use crate::error::{Error, R};
use crate::lexer::{TPart, Tok, Token, lex, lex_text_apostrophes};
use std::collections::HashMap;

// use の対象の本文、依存ファイルの起点、ファイルの識別名。
fn load(
    name: &str,
    dir: &std::path::Path,
    span: crate::lexer::Span,
) -> R<(String, std::path::PathBuf, String)> {
    if let Some((_, text)) = LIBS.iter().find(|(spec, _)| *spec == name) {
        return Ok((text.to_string(), dir.to_path_buf(), super::parser::builtin_identity(name)));
    }
    let path = dir.join(name.replace("::", "/")).with_extension("hb");
    let text = std::fs::read_to_string(&path).map_err(|error| {
        Error::from(format!("ライブラリ {name} を読み込めない（{}）: {error}", path.display()))
            .at(span)
    })?;
    let directory = path.parent().unwrap().to_path_buf();
    let identity = path.canonicalize().unwrap_or(path).display().to_string();
    Ok((text, directory, identity))
}

/// 読み込んだファイルと、そのファイルで使うモジュール名。
#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub source: super::source::SourceFile,
    pub uses: Vec<Use>,
    base_dir: std::path::PathBuf,
}

/// ソースを結合せず、ファイルごとに保持した読み込み結果。
#[derive(Debug, Clone)]
pub struct ModuleSet {
    pub modules: Vec<Module>,
}

impl From<&str> for ModuleSet {
    fn from(text: &str) -> Self {
        Self {
            modules: vec![Module {
                name: String::new(),
                source: super::source::SourceFile {
                    id: Default::default(),
                    name: super::source::INPUT_NAME.into(),
                    text: text.into(),
                },
                uses: Vec::new(),
                base_dir: base_dir(),
            }],
        }
    }
}

impl From<&String> for ModuleSet {
    fn from(text: &String) -> Self {
        Self::from(text.as_str())
    }
}

pub(super) fn identify_tokens(tokens: &mut [Token], id: super::source::SourceId) {
    for token in tokens {
        token.span.source = id;
        if let Tok::Template(_, parts) = &mut token.tok {
            for part in parts {
                if let TPart::Code(tokens) = part {
                    identify_tokens(tokens, id);
                }
            }
        }
    }
}

pub(super) fn source_tokens(source: &super::source::SourceFile) -> R<Vec<Token>> {
    let mut tokens = lex(&source.text).map_err(|mut error| {
        error.source_id = source.id;
        error.with_sources([source])
    })?;
    identify_tokens(&mut tokens, source.id);
    Ok(tokens)
}

pub(super) fn text_source_tokens(source: &super::source::SourceFile) -> Option<Vec<Token>> {
    let mut tokens = lex_text_apostrophes(&source.text)?;
    identify_tokens(&mut tokens, source.id);
    Some(tokens)
}

/// use の依存を読み込み、ファイルごとの位置とモジュールを保持する。
pub fn load_modules(source: &str, directory: &std::path::Path) -> R<ModuleSet> {
    let mut loaded = ModuleSet::from(source);
    loaded.modules[0].base_dir = directory.into();
    let mut identities: HashMap<String, String> = HashMap::new();
    let mut cursor = 0;
    while cursor < loaded.modules.len() {
        let unit = &loaded.modules[cursor];
        let tokens = source_tokens(&unit.source)
            .or_else(|error| text_source_tokens(&unit.source).ok_or(error))?;
        let dir = unit.base_dir.clone();
        let mut requests = Vec::new();
        let mut depth = 0;
        for pair in tokens.windows(2) {
            match pair[0].tok {
                Tok::Punct("(" | "[" | "{") => depth += 1,
                Tok::Punct(")" | "]" | "}") => depth -= 1,
                _ => {}
            }
            if pair[0].tok != Tok::Punct("use") || depth != 0 {
                continue;
            }
            match &pair[1].tok {
                Tok::Ident(name) => requests.push((name.to_string(), pair[1].span)),
                _ => {
                    return Err(Error::from("use の後にはモジュール名が必要")
                        .at(pair[1].span)
                        .with_sources(loaded.modules.iter().map(|m| &m.source)));
                }
            }
        }
        if tokens.iter().any(|token| matches!(&token.tok, Tok::Ident(name) if PRELUDE_NAMES.contains(&name.as_str()))) {
            requests.push(("std".into(), tokens[0].span));
        }
        for (spec, span) in requests {
            let (text, sub, identity) = load(&spec, &dir, span)
                .map_err(|error| error.with_sources(loaded.modules.iter().map(|m| &m.source)))?;
            let name = spec.rsplit("::").next().unwrap().to_string();
            if let Some(previous) = identities.get(&name) {
                if previous != &identity {
                    return Err(Error::from(format!(
                        "モジュール名 {name} が異なるファイルで重複している"
                    ))
                    .at(span)
                    .with_sources(loaded.modules.iter().map(|m| &m.source)));
                }
                continue;
            }
            identities.insert(name.clone(), identity.clone());
            loaded.modules.push(Module {
                name,
                source: super::source::SourceFile {
                    id: super::source::SourceId(loaded.modules.len()),
                    name: identity,
                    text,
                },
                uses: Vec::new(),
                base_dir: sub,
            });
        }
        cursor += 1;
    }
    Ok(loaded)
}
