use std::collections::HashSet;
use std::path::Path;

use ra_ap_syntax::{AstNode, AstToken, Edition, SyntaxNode, ast, ast::HasAttrs, ast::LiteralKind};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) enum CfgAtom {
    Flag(String),
    KeyValue(String, String),
}

pub(super) fn parse_cfg_spec(spec: &str) -> Result<CfgAtom, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("cfg option must not be empty".to_owned());
    }
    let (key, value) = match spec.split_once('=') {
        Some((key, value)) => (key.trim(), Some(value.trim())),
        None => (spec, None),
    };
    if !is_cfg_identifier(key) {
        return Err(format!("invalid cfg option {spec:?}: invalid name {key:?}"));
    }
    let Some(value) = value else {
        return Ok(CfgAtom::Flag(key.to_owned()));
    };
    if value.is_empty() {
        return Err(format!(
            "invalid cfg option {spec:?}: value must not be empty"
        ));
    }
    let value = if value.starts_with('"') {
        let parsed = ast::Expr::parse(value, Edition::Edition2024);
        if !parsed.errors().is_empty() {
            return Err(format!("invalid cfg option {spec:?}: invalid string value"));
        }
        match parsed.tree() {
            ast::Expr::Literal(literal) => match literal.kind() {
                LiteralKind::String(string) => string
                    .value()
                    .map_err(|error| format!("invalid cfg option {spec:?}: {error:?}"))?
                    .into_owned(),
                _ => return Err(format!("invalid cfg option {spec:?}: invalid string value")),
            },
            _ => return Err(format!("invalid cfg option {spec:?}: invalid string value")),
        }
    } else {
        value.to_owned()
    };
    Ok(CfgAtom::KeyValue(key.to_owned(), value))
}

fn is_cfg_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

pub(super) fn syntax_is_active(
    node: &SyntaxNode,
    cfg: &HashSet<CfgAtom>,
    file_path: &Path,
) -> Result<bool, String> {
    for ancestor in node.ancestors() {
        let Some(owner) = ast::AnyHasAttrs::cast(ancestor) else {
            continue;
        };
        for attr in owner.attrs() {
            if let Some(meta) = attr.meta()
                && !meta_keeps_owner(meta, cfg, file_path)?
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn meta_keeps_owner(
    meta: ast::Meta,
    cfg: &HashSet<CfgAtom>,
    file_path: &Path,
) -> Result<bool, String> {
    match meta {
        ast::Meta::CfgMeta(meta) => {
            let predicate = meta.cfg_predicate().ok_or_else(|| {
                format!("{}: cfg attribute has no predicate", file_path.display())
            })?;
            eval_cfg_predicate(predicate, cfg, file_path)
        }
        ast::Meta::CfgAttrMeta(meta) => {
            let predicate = meta.cfg_predicate().ok_or_else(|| {
                format!(
                    "{}: cfg_attr attribute has no predicate",
                    file_path.display()
                )
            })?;
            if !eval_cfg_predicate(predicate, cfg, file_path)? {
                return Ok(true);
            }
            for nested in meta.metas() {
                if !meta_keeps_owner(nested, cfg, file_path)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(true),
    }
}

pub(super) fn eval_cfg_predicate(
    predicate: ast::CfgPredicate,
    cfg: &HashSet<CfgAtom>,
    file_path: &Path,
) -> Result<bool, String> {
    match predicate {
        ast::CfgPredicate::CfgAtom(atom) => match atom.key() {
            Some(ast::CfgAtomKey::True) => Ok(true),
            Some(ast::CfgAtomKey::False) => Ok(false),
            Some(ast::CfgAtomKey::Ident(key)) => {
                let key = key.text().to_string();
                if let Some(token) = atom.string_token() {
                    let value = ast::String::cast(token)
                        .ok_or_else(|| {
                            format!("{}: invalid cfg string value", file_path.display())
                        })?
                        .value()
                        .map_err(|error| {
                            format!(
                                "{}: invalid cfg string value: {error:?}",
                                file_path.display()
                            )
                        })?
                        .into_owned();
                    Ok(cfg.contains(&CfgAtom::KeyValue(key, value)))
                } else {
                    Ok(cfg.contains(&CfgAtom::Flag(key)))
                }
            }
            None => Err(format!(
                "{}: invalid cfg predicate {:?}",
                file_path.display(),
                atom.syntax().text()
            )),
        },
        ast::CfgPredicate::CfgComposite(composite) => {
            let keyword = composite
                .keyword()
                .ok_or_else(|| format!("{}: cfg predicate has no operator", file_path.display()))?;
            let predicates = composite.cfg_predicates().collect::<Vec<_>>();
            match keyword.text() {
                "all" => {
                    for predicate in predicates {
                        if !eval_cfg_predicate(predicate, cfg, file_path)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                "any" => {
                    for predicate in predicates {
                        if eval_cfg_predicate(predicate, cfg, file_path)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                "not" if predicates.len() == 1 => {
                    Ok(!eval_cfg_predicate(predicates[0].clone(), cfg, file_path)?)
                }
                "not" => Err(format!(
                    "{}: cfg not(...) requires exactly one predicate",
                    file_path.display()
                )),
                operator => Err(format!(
                    "{}: unsupported cfg operator {operator:?}",
                    file_path.display()
                )),
            }
        }
    }
}
