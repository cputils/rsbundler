use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ra_ap_syntax::{
    AstNode, Edition, SourceFile, ast,
    ast::{HasAttrs, HasName, LiteralKind},
};

use super::cfg::{CfgExpr, cfg_predicate_expression};

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
pub(super) struct ResolvedModuleFile {
    pub(super) path: PathBuf,
    pub(super) loaded_through_path_attr: bool,
}

#[derive(Clone, Debug)]
pub(super) enum ModuleResolutionError {
    Missing(String),
    Ambiguous(String),
    Invalid(String),
}

#[derive(Debug)]
pub(super) struct ModuleFileVariant {
    pub(super) condition: CfgExpr,
    pub(super) resolution: Result<ResolvedModuleFile, ModuleResolutionError>,
    pub(super) inline_default_dir: Option<PathBuf>,
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

pub(super) fn module_file_variants(
    module: &ast::Module,
    context: &SourceContext,
    file_path: &Path,
) -> Result<Vec<ModuleFileVariant>, String> {
    let mut ancestors = module
        .syntax()
        .ancestors()
        .skip(1)
        .filter_map(ast::Module::cast)
        .filter(|ancestor| ancestor.item_list().is_some())
        .collect::<Vec<_>>();
    ancestors.reverse();

    let mut locations = vec![LocationVariant {
        condition: CfgExpr::True,
        default_dir: Ok(context.default_module_dir.clone()),
    }];
    for ancestor in &ancestors {
        let choices = module_path_choices(ancestor, file_path)?;
        let default_name = module_name(ancestor, file_path)?;
        let mut next = Vec::new();
        for location in locations {
            let Ok(default_dir) = location.default_dir else {
                next.push(location);
                continue;
            };
            for choice in &choices {
                let condition =
                    CfgExpr::all([location.condition.clone(), choice.condition.clone()]);
                if condition.is_false() {
                    continue;
                }
                let default_dir = match &choice.selection {
                    PathSelection::Default => Ok(default_dir.join(&default_name)),
                    PathSelection::Path(path) => Ok(default_dir.join(path)),
                    PathSelection::Invalid(message) => {
                        Err(ModuleResolutionError::Invalid(message.clone()))
                    }
                };
                next.push(LocationVariant {
                    condition,
                    default_dir,
                });
            }
        }
        locations = next;
    }

    let choices = module_path_choices(module, file_path)?;
    let name = module_name(module, file_path)?;
    let mut variants = Vec::new();
    for location in locations {
        let default_dir = match location.default_dir {
            Ok(default_dir) => default_dir,
            Err(error) => {
                variants.push(ModuleFileVariant {
                    condition: location.condition,
                    resolution: Err(error),
                    inline_default_dir: None,
                });
                continue;
            }
        };
        let path_attr_dir = if ancestors.is_empty() {
            context.physical_dir.clone()
        } else {
            default_dir.clone()
        };
        for choice in &choices {
            let condition = CfgExpr::all([location.condition.clone(), choice.condition.clone()]);
            if condition.is_false() {
                continue;
            }
            let (resolution, inline_default_dir) = match &choice.selection {
                PathSelection::Default => {
                    let inline_default_dir = default_dir.join(&name);
                    (
                        resolve_default_module(&default_dir, &name, file_path),
                        Some(inline_default_dir),
                    )
                }
                PathSelection::Path(path) => {
                    let resolved = path_attr_dir.join(path);
                    (
                        Ok(ResolvedModuleFile {
                            path: resolved.clone(),
                            loaded_through_path_attr: true,
                        }),
                        Some(resolved),
                    )
                }
                PathSelection::Invalid(message) => {
                    (Err(ModuleResolutionError::Invalid(message.clone())), None)
                }
            };
            variants.push(ModuleFileVariant {
                condition,
                resolution,
                inline_default_dir,
            });
        }
    }
    Ok(variants)
}

pub(super) fn module_inlining_condition(
    module: &ast::Module,
    file_path: &Path,
) -> Result<CfgExpr, String> {
    let mut unsafe_conditions = Vec::new();
    let modules = std::iter::once(module.clone()).chain(
        module
            .syntax()
            .ancestors()
            .skip(1)
            .filter_map(ast::Module::cast)
            .filter(|ancestor| ancestor.item_list().is_some()),
    );
    for module in modules {
        for attr in module.attrs() {
            let Some(meta) = attr.meta() else {
                continue;
            };
            collect_unsafe_attribute_conditions(
                meta,
                CfgExpr::True,
                &mut unsafe_conditions,
                file_path,
            )?;
        }
    }
    Ok(CfgExpr::any(unsafe_conditions).not())
}

fn collect_unsafe_attribute_conditions(
    meta: ast::Meta,
    activation: CfgExpr,
    conditions: &mut Vec<CfgExpr>,
    file_path: &Path,
) -> Result<(), String> {
    if activation.is_false() {
        return Ok(());
    }
    let path = match &meta {
        ast::Meta::CfgAttrMeta(cfg_attr) => {
            let predicate = cfg_attr.cfg_predicate().ok_or_else(|| {
                format!(
                    "{}: cfg_attr attribute has no predicate",
                    file_path.display()
                )
            })?;
            let activation =
                CfgExpr::all([activation, cfg_predicate_expression(predicate, file_path)?]);
            for nested in cfg_attr.metas() {
                collect_unsafe_attribute_conditions(
                    nested,
                    activation.clone(),
                    conditions,
                    file_path,
                )?;
            }
            return Ok(());
        }
        ast::Meta::CfgMeta(_) => return Ok(()),
        ast::Meta::UnsafeMeta(meta) => {
            let Some(meta) = meta.meta() else {
                conditions.push(activation);
                return Ok(());
            };
            return collect_unsafe_attribute_conditions(meta, activation, conditions, file_path);
        }
        ast::Meta::PathMeta(meta) => meta.path(),
        ast::Meta::TokenTreeMeta(meta) => meta.path(),
        ast::Meta::KeyValueMeta(meta) => meta.path(),
    }
    .map(|path| path.syntax().text().to_string())
    .unwrap_or_default();
    let name = path.trim_start_matches("::");
    if !matches!(
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
    ) {
        conditions.push(activation);
    }
    Ok(())
}

fn resolve_default_module(
    default_dir: &Path,
    name: &str,
    declaring_file: &Path,
) -> Result<ResolvedModuleFile, ModuleResolutionError> {
    let flat = default_dir.join(format!("{name}.rs"));
    let directory = default_dir.join(name).join("mod.rs");
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

#[derive(Debug)]
struct LocationVariant {
    condition: CfgExpr,
    default_dir: Result<PathBuf, ModuleResolutionError>,
}

#[derive(Clone, Debug)]
struct PathCandidate {
    condition: CfgExpr,
    path: Result<PathBuf, String>,
}

#[derive(Clone, Debug)]
struct PathChoice {
    condition: CfgExpr,
    selection: PathSelection,
}

#[derive(Clone, Debug)]
enum PathSelection {
    Default,
    Path(PathBuf),
    Invalid(String),
}

fn module_path_choices(module: &ast::Module, file_path: &Path) -> Result<Vec<PathChoice>, String> {
    let mut candidates = Vec::new();
    for attr in module.attrs() {
        if let Some(meta) = attr.meta() {
            collect_path_candidates(meta, CfgExpr::True, &mut candidates, file_path)?;
        }
    }
    candidates.retain(|candidate| !candidate.condition.is_false());

    let conditions = candidates
        .iter()
        .map(|candidate| candidate.condition.clone())
        .collect::<Vec<_>>();
    let mut choices = Vec::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        let other_conditions = conditions
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, condition)| condition.clone());
        let condition = CfgExpr::all([candidate.condition, CfgExpr::any(other_conditions).not()]);
        if condition.is_false() {
            continue;
        }
        let selection = match candidate.path {
            Ok(path) => PathSelection::Path(path),
            Err(message) => PathSelection::Invalid(message),
        };
        choices.push(PathChoice {
            condition,
            selection,
        });
    }

    let any_path = CfgExpr::any(conditions.iter().cloned());
    let default_condition = any_path.clone().not();
    if !default_condition.is_false() {
        choices.push(PathChoice {
            condition: default_condition,
            selection: PathSelection::Default,
        });
    }

    let conflicts = conditions.iter().enumerate().flat_map(|(index, left)| {
        conditions[index + 1..]
            .iter()
            .map(move |right| CfgExpr::all([left.clone(), right.clone()]))
    });
    let conflict_condition = CfgExpr::any(conflicts);
    if !conflict_condition.is_false() {
        choices.push(PathChoice {
            condition: conflict_condition,
            selection: PathSelection::Invalid(format!(
                "{}: module {:?} has more than one active path attribute",
                file_path.display(),
                module_name(module, file_path)?
            )),
        });
    }

    Ok(choices)
}

fn collect_path_candidates(
    meta: ast::Meta,
    activation: CfgExpr,
    candidates: &mut Vec<PathCandidate>,
    file_path: &Path,
) -> Result<(), String> {
    match meta {
        ast::Meta::KeyValueMeta(meta)
            if meta
                .path()
                .and_then(|path| path.as_single_name_ref())
                .is_some_and(|name| name.text() == "path") =>
        {
            let path = match meta.expr() {
                Some(ast::Expr::Literal(literal)) => match literal.kind() {
                    LiteralKind::String(value) => value
                        .value()
                        .map_err(|error| {
                            format!("invalid path string in {}: {error:?}", file_path.display())
                        })
                        .map(|value| PathBuf::from(value.into_owned())),
                    _ => Err(format!(
                        "{}: path attribute must be a string literal",
                        file_path.display()
                    )),
                },
                _ => Err(format!(
                    "{}: path attribute must be a string literal",
                    file_path.display()
                )),
            };
            candidates.push(PathCandidate {
                condition: activation,
                path,
            });
            Ok(())
        }
        ast::Meta::CfgAttrMeta(meta) => {
            let predicate = meta.cfg_predicate().ok_or_else(|| {
                format!(
                    "{}: cfg_attr attribute has no predicate",
                    file_path.display()
                )
            })?;
            let activation =
                CfgExpr::all([activation, cfg_predicate_expression(predicate, file_path)?]);
            for nested in meta.metas() {
                collect_path_candidates(nested, activation.clone(), candidates, file_path)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
