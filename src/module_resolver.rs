use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ra_ap_syntax::{
    AstNode, Edition, SourceFile, ast,
    ast::{HasAttrs, HasName, LiteralKind},
};

use super::cfg::{CfgAtom, eval_cfg_predicate};

#[derive(Clone, Debug)]
pub(super) struct SourceContext {
    pub(super) physical_dir: PathBuf,
    pub(super) default_module_dir: PathBuf,
}

impl SourceContext {
    pub(super) fn for_crate_root(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("entry file has no parent directory: {}", path.display()))?;
        Ok(Self {
            physical_dir: parent.to_path_buf(),
            default_module_dir: parent.to_path_buf(),
        })
    }

    pub(super) fn for_module_file(
        path: &Path,
        loaded_through_path_attr: bool,
    ) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("module file has no parent directory: {}", path.display()))?;
        let default_module_dir =
            if loaded_through_path_attr || path.file_name().is_some_and(|name| name == "mod.rs") {
                parent.to_path_buf()
            } else {
                let stem = path.file_stem().ok_or_else(|| {
                    format!("module file has no usable file name: {}", path.display())
                })?;
                parent.join(stem)
            };
        Ok(Self {
            physical_dir: parent.to_path_buf(),
            default_module_dir,
        })
    }
}

#[derive(Debug)]
pub(super) struct ModuleLocation {
    default_dir: PathBuf,
    path_attr_dir: PathBuf,
}

#[derive(Debug)]
pub(super) struct ResolvedModuleFile {
    pub(super) path: PathBuf,
    pub(super) loaded_through_path_attr: bool,
}

#[derive(Debug)]
pub(super) enum ModuleResolutionError {
    Missing(String),
    Ambiguous(String),
    Invalid(String),
}

impl ModuleResolutionError {
    pub(super) fn into_message(self) -> String {
        match self {
            Self::Missing(message) | Self::Ambiguous(message) | Self::Invalid(message) => message,
        }
    }
}

pub(super) fn normalize_external_name(value: &str, edition: Edition) -> Result<String, String> {
    let original = value;
    let mut value = value.trim().trim_start_matches("::");
    if let Some(rest) = value.strip_prefix("crate::") {
        value = rest;
    }
    if value.is_empty()
        || value.contains("::")
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_whitespace)
    {
        return Err(format!("invalid external module name {original:?}"));
    }
    let parsed = SourceFile::parse(&format!("mod {value};"), edition);
    if !parsed.errors().is_empty() {
        return Err(format!("invalid external module name {original:?}"));
    }
    parsed
        .tree()
        .syntax()
        .descendants()
        .find_map(ast::Module::cast)
        .and_then(|module| module.name())
        .map(|name| name.syntax().text().to_string())
        .map(|name| name.strip_prefix("r#").unwrap_or(&name).to_owned())
        .ok_or_else(|| format!("invalid external module name {original:?}"))
}

pub(super) fn module_path_is_external(path: &str, external: &HashSet<String>) -> bool {
    path.strip_prefix("crate::")
        .and_then(|path| path.split("::").next())
        .is_some_and(|name| external.contains(name.strip_prefix("r#").unwrap_or(name)))
}

pub(super) fn module_location(
    module: &ast::Module,
    context: &SourceContext,
    file_path: &Path,
    cfg: &HashSet<CfgAtom>,
) -> Result<ModuleLocation, String> {
    let mut ancestors = module
        .syntax()
        .ancestors()
        .skip(1)
        .filter_map(ast::Module::cast)
        .filter(|ancestor| ancestor.item_list().is_some())
        .collect::<Vec<_>>();
    ancestors.reverse();

    let mut default_dir = context.default_module_dir.clone();
    for ancestor in &ancestors {
        if let Some(path) = explicit_path_attr(ancestor, file_path, cfg)? {
            default_dir.push(path);
        } else {
            default_dir.push(module_name(ancestor, file_path)?);
        }
    }

    let path_attr_dir = if ancestors.is_empty() {
        context.physical_dir.clone()
    } else {
        default_dir.clone()
    };
    Ok(ModuleLocation {
        default_dir,
        path_attr_dir,
    })
}

pub(super) fn inline_module_default_dir(
    module: &ast::Module,
    location: &ModuleLocation,
    file_path: &Path,
    cfg: &HashSet<CfgAtom>,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit_path_attr(module, file_path, cfg)? {
        return Ok(location.path_attr_dir.join(path));
    }
    Ok(location.default_dir.join(module_name(module, file_path)?))
}

pub(super) fn module_attributes_allow_inlining(
    module: &ast::Module,
    cfg: &HashSet<CfgAtom>,
    file_path: &Path,
) -> Result<bool, String> {
    for attr in module.attrs() {
        let Some(meta) = attr.meta() else {
            continue;
        };
        if !meta_allows_inlining(meta, cfg, file_path)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn meta_allows_inlining(
    meta: ast::Meta,
    cfg: &HashSet<CfgAtom>,
    file_path: &Path,
) -> Result<bool, String> {
    if let ast::Meta::CfgAttrMeta(cfg_attr) = meta {
        let predicate = cfg_attr.cfg_predicate().ok_or_else(|| {
            format!(
                "{}: cfg_attr attribute has no predicate",
                file_path.display()
            )
        })?;
        if !eval_cfg_predicate(predicate, cfg, file_path)? {
            return Ok(true);
        }
        for nested in cfg_attr.metas() {
            if !meta_allows_inlining(nested, cfg, file_path)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }

    let path = match meta {
        ast::Meta::PathMeta(meta) => meta.path(),
        ast::Meta::TokenTreeMeta(meta) => meta.path(),
        ast::Meta::KeyValueMeta(meta) => meta.path(),
        ast::Meta::CfgMeta(_) => return Ok(true),
        ast::Meta::CfgAttrMeta(_) => unreachable!(),
        ast::Meta::UnsafeMeta(meta) => {
            return meta
                .meta()
                .map_or(Ok(false), |meta| meta_allows_inlining(meta, cfg, file_path));
        }
    }
    .map(|path| path.syntax().text().to_string())
    .unwrap_or_default();
    let name = path.trim_start_matches("::");
    Ok(matches!(
        name,
        "path"
            | "doc"
            | "allow"
            | "warn"
            | "deny"
            | "forbid"
            | "expect"
            | "deprecated"
            | "must_use"
            | "macro_use"
            | "macro_escape"
            | "no_implicit_prelude"
            | "test"
            | "bench"
            | "ignore"
            | "should_panic"
            | "rustfmt::skip"
            | "rust_analyzer::skip"
    ))
}

pub(super) fn resolve_module_file(
    module: &ast::Module,
    location: &ModuleLocation,
    declaring_file: &Path,
    cfg: &HashSet<CfgAtom>,
) -> Result<ResolvedModuleFile, ModuleResolutionError> {
    if let Some(path) =
        explicit_path_attr(module, declaring_file, cfg).map_err(ModuleResolutionError::Invalid)?
    {
        return Ok(ResolvedModuleFile {
            path: location.path_attr_dir.join(path),
            loaded_through_path_attr: true,
        });
    }

    let name = module_name(module, declaring_file).map_err(ModuleResolutionError::Invalid)?;
    let flat = location.default_dir.join(format!("{name}.rs"));
    let directory = location.default_dir.join(&name).join("mod.rs");
    match (flat.is_file(), directory.is_file()) {
        (true, false) => Ok(ResolvedModuleFile {
            path: flat,
            loaded_through_path_attr: false,
        }),
        (false, true) => Ok(ResolvedModuleFile {
            path: directory,
            loaded_through_path_attr: false,
        }),
        (true, true) => Err(ModuleResolutionError::Ambiguous(format!(
            "ambiguous module {name:?} declared in {}: both {} and {} exist",
            declaring_file.display(),
            flat.display(),
            directory.display()
        ))),
        (false, false) => Err(ModuleResolutionError::Missing(format!(
            "failed to resolve module {name:?} declared in {}: expected {} or {}",
            declaring_file.display(),
            flat.display(),
            directory.display()
        ))),
    }
}

pub(super) fn qualified_module_path(
    module: &ast::Module,
    source_module_path: &str,
    file_path: &Path,
) -> Result<String, String> {
    let mut ancestors = module
        .syntax()
        .ancestors()
        .skip(1)
        .filter_map(ast::Module::cast)
        .filter(|ancestor| ancestor.item_list().is_some())
        .collect::<Vec<_>>();
    ancestors.reverse();

    let mut path = source_module_path.to_owned();
    for ancestor in ancestors {
        path.push_str("::");
        path.push_str(&module_name(&ancestor, file_path)?);
    }
    path.push_str("::");
    path.push_str(&module_name(module, file_path)?);
    Ok(path)
}

pub(super) fn containing_module_path(
    node: &ra_ap_syntax::SyntaxNode,
    source_module_path: &str,
    file_path: &Path,
) -> Result<String, String> {
    let mut ancestors = node
        .ancestors()
        .skip(1)
        .filter_map(ast::Module::cast)
        .filter(|ancestor| ancestor.item_list().is_some())
        .collect::<Vec<_>>();
    ancestors.reverse();

    let mut path = source_module_path.to_owned();
    for ancestor in ancestors {
        path.push_str("::");
        path.push_str(&module_name(&ancestor, file_path)?);
    }
    Ok(path)
}

fn module_name(module: &ast::Module, file_path: &Path) -> Result<String, String> {
    let name = module
        .name()
        .ok_or_else(|| format!("unnamed module declaration in {}", file_path.display()))?
        .syntax()
        .text()
        .to_string();
    Ok(name.strip_prefix("r#").unwrap_or(&name).to_owned())
}

fn collect_active_path_values(
    meta: ast::Meta,
    cfg: &HashSet<CfgAtom>,
    values: &mut Vec<String>,
    file_path: &Path,
) -> Result<(), String> {
    match meta {
        ast::Meta::KeyValueMeta(meta)
            if meta
                .path()
                .and_then(|path| path.as_single_name_ref())
                .is_some_and(|name| name.text() == "path") =>
        {
            let value = match meta.expr() {
                Some(ast::Expr::Literal(literal)) => match literal.kind() {
                    LiteralKind::String(value) => value
                        .value()
                        .map_err(|error| {
                            format!("invalid path string in {}: {error:?}", file_path.display())
                        })?
                        .into_owned(),
                    _ => {
                        return Err(format!(
                            "{}: path attribute must be a string literal",
                            file_path.display()
                        ));
                    }
                },
                _ => {
                    return Err(format!(
                        "{}: path attribute must be a string literal",
                        file_path.display()
                    ));
                }
            };
            values.push(value);
            Ok(())
        }
        ast::Meta::CfgAttrMeta(meta) => {
            let predicate = meta.cfg_predicate().ok_or_else(|| {
                format!(
                    "{}: cfg_attr attribute has no predicate",
                    file_path.display()
                )
            })?;
            if eval_cfg_predicate(predicate, cfg, file_path)? {
                for nested in meta.metas() {
                    collect_active_path_values(nested, cfg, values, file_path)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn explicit_path_attr(
    module: &ast::Module,
    file_path: &Path,
    cfg: &HashSet<CfgAtom>,
) -> Result<Option<PathBuf>, String> {
    let mut values = Vec::new();
    for attr in module.attrs() {
        if let Some(meta) = attr.meta() {
            collect_active_path_values(meta, cfg, &mut values, file_path)?;
        }
    }
    match values.len() {
        0 => Ok(None),
        1 => Ok(values.pop().map(PathBuf::from)),
        _ => Err(format!(
            "{}: module {:?} has more than one active path attribute",
            file_path.display(),
            module_name(module, file_path)?
        )),
    }
}
