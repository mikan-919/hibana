//! 依存ファイルの読み込みとモジュール表。
use super::parser::{LIBS, PRELUDE_NAMES, Use, base_dir, module_name};
use crate::error::{Error, R};
use crate::lexer::{TPart, Tok, Token, lex};
use std::collections::HashMap;

// JavaScript のパッケージ（npm・node:）の型宣言 .d.ts を node_modules から探す
fn find_dts(spec: &str, directory: &std::path::Path) -> Option<String> {
    let nm = directory.join("node_modules");
    let mut cands = Vec::new();
    if let Some(m) = spec.strip_prefix("node:") {
        cands.push(nm.join("@types/node").join(format!("{m}.d.ts")));
    } else {
        let pkg = nm.join(spec);
        if let Ok(j) = std::fs::read_to_string(pkg.join("package.json")) {
            // package.json の "types" か "typings"
            for key in ["\"types\"", "\"typings\""] {
                if let Some(rest) = j.split(key).nth(1)
                    && let Some(v) = rest.split('"').nth(1)
                {
                    cands.push(pkg.join(v));
                }
            }
        }
        cands.push(pkg.join("index.d.ts"));
        cands.push(nm.join("@types").join(spec).join("index.d.ts"));
    }
    cands
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
}

// 文字列で指定した use の名前を検査し、JavaScript / TypeScript のモジュール名を返す。
pub(super) fn external_module_name(name: &str) -> R<String> {
    if name.ends_with(".hb")
        || (name.starts_with('.') && std::path::Path::new(name).extension().is_none())
    {
        return Err(Error::from("hibana ファイルは文字列の経路で読み込めない")
            .help("use util または use foo::bar のように名前で指定する".into()));
    }
    Ok(module_name(name))
}

// use の対象のモジュール名と本文、依存ファイルの起点。
fn load(
    name: &str,
    is_str: bool,
    dir: &std::path::Path,
    span: crate::lexer::Span,
) -> R<(String, String, std::path::PathBuf)> {
    // 名前で書いたら組み込みを先に探し、なければ名前を経路にして hibana ファイルを読む。
    if !is_str {
        if let Some((_, t)) = LIBS.iter().find(|l| l.0 == name) {
            return Ok((
                name.rsplit("::").next().unwrap().to_string(),
                t.to_string(),
                dir.to_path_buf(),
            ));
        }
        let path = dir.join(name.replace("::", "/")).with_extension("hb");
        let text = std::fs::read_to_string(&path).map_err(|error| {
            Error::from(format!("ライブラリ {name} を読み込めない（{}）: {error}", path.display()))
                .at(span)
        })?;
        return Ok((
            name.rsplit("::").next().unwrap().to_string(),
            text,
            path.parent().unwrap().to_path_buf(),
        ));
    }
    let module = external_module_name(name).map_err(|error| error.at(span))?;
    let read = |p: &str| {
        std::fs::read_to_string(dir.join(p)).map_err(|e| Error::from(format!("{p}: {e}")).at(span))
    };
    // JavaScript / TypeScript は型宣言から extern の宣言を作る（JS ターゲットでだけ使える）
    let dts = if name.ends_with(".ts") {
        read(name)?
    } else if let Some(base) = name.strip_suffix(".js").or(name.strip_suffix(".mjs")) {
        read(&format!("{base}.d.ts"))
            .map_err(|_| Error::from(format!("{name} の型宣言 {base}.d.ts が無い")).at(span))?
    } else if name.starts_with("node:") || !name.contains('/') || name.starts_with('@') {
        find_dts(name, dir).ok_or_else(|| {
            Error::from(format!("{name} の型宣言（.d.ts）が node_modules に無い（node: の組み込みには @types/node が要る）"))
                .at(span)
        })?
    } else {
        return Err(Error::from(format!("ライブラリ {name} が無い")).at(span));
    };
    let text = crate::ts::module_text(&dts, name).map_err(|e| e.at(span))?;
    Ok((module, text, dir.to_path_buf()))
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
                    name: "<input>".into(),
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

pub(super) fn source_tokens(source: &super::source::SourceFile) -> R<Vec<Token>> {
    fn identify(tokens: &mut [Token], id: super::source::SourceId) {
        for token in tokens {
            token.span.source = id;
            if let Tok::Template(_, parts) = &mut token.tok {
                for part in parts {
                    if let TPart::Code(tokens) = part {
                        identify(tokens, id);
                    }
                }
            }
        }
    }
    let mut tokens = lex(&source.text).map_err(|mut error| {
        error.source_id = source.id;
        error.with_sources([source])
    })?;
    identify(&mut tokens, source.id);
    Ok(tokens)
}

/// use の依存を読み込み、ファイルごとの位置とモジュールを保持する。
pub fn load_modules(source: &str, directory: &std::path::Path) -> R<ModuleSet> {
    let mut loaded = ModuleSet::from(source);
    loaded.modules[0].base_dir = directory.into();
    let mut identities: HashMap<String, String> = HashMap::new();
    let mut cursor = 0;
    while cursor < loaded.modules.len() {
        let unit = &loaded.modules[cursor];
        let tokens = source_tokens(&unit.source)?;
        let dir = unit.base_dir.clone();
        let mut requests = Vec::new();
        for pair in tokens.windows(2) {
            if pair[0].tok != Tok::Punct("use") {
                continue;
            }
            match &pair[1].tok {
                Tok::Ident(name) => requests.push((name.to_string(), false, pair[1].span)),
                Tok::Str(name) => requests.push((name.clone().into(), true, pair[1].span)),
                _ => {
                    return Err(Error::from("use の後にはライブラリ名かファイル名が必要")
                        .at(pair[1].span)
                        .with_sources(loaded.modules.iter().map(|m| &m.source)));
                }
            }
        }
        if tokens.iter().any(|token| matches!(&token.tok, Tok::Ident(name) if PRELUDE_NAMES.contains(&name.as_str()))) {
            requests.push(("std".into(), false, tokens[0].span));
        }
        for (spec, is_str, span) in requests {
            let (name, text, sub) = load(&spec, is_str, &dir, span)
                .map_err(|error| error.with_sources(loaded.modules.iter().map(|m| &m.source)))?;
            let identity = if !is_str && LIBS.iter().any(|(name, _)| *name == spec) {
                format!("builtin:{spec}")
            } else {
                let path = if is_str {
                    dir.join(&spec)
                } else {
                    dir.join(spec.replace("::", "/")).with_extension("hb")
                };
                path.canonicalize().unwrap_or(path).display().to_string()
            };
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
