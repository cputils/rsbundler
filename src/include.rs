use std::collections::{HashMap, HashSet};

use ra_ap_syntax::{
    AstNode, AstToken, NodeOrToken, SyntaxNode, ast,
    ast::{HasAttrs, HasName, LiteralKind},
};

use super::{BundledSourceKind, RustEdition};

#[derive(Clone, Debug)]
pub(super) struct MacroScope {
    defined_macros: HashSet<String>,
    has_unknown_macro_import: bool,
    std_is_available: bool,
    core_is_available: bool,
}

impl Default for MacroScope {
    fn default() -> Self {
        Self {
            defined_macros: HashSet::new(),
            has_unknown_macro_import: false,
            std_is_available: true,
            core_is_available: true,
        }
    }
}

impl MacroScope {
    pub(super) fn at(node: &SyntaxNode, root: &SyntaxNode, inherited: &Self) -> Self {
        let mut scope = inherited.clone();
        for definition in root.descendants() {
            let Some((name, textual_scope)) = ast::MacroRules::cast(definition.clone())
                .and_then(|macro_rules| {
                    let textual_scope = !has_attribute(&macro_rules, "macro_export");
                    macro_rules.name().map(|name| (name, textual_scope))
                })
                .or_else(|| {
                    ast::MacroDef::cast(definition.clone())
                        .and_then(|item| item.name())
                        .map(|name| (name, false))
                })
            else {
                continue;
            };
            if syntax_binding_is_visible(&definition, node, textual_scope) {
                scope
                    .defined_macros
                    .insert(normalize_macro_path(&name.syntax().text().to_string()));
            }
        }
        scope.has_unknown_macro_import |=
            root.descendants().filter_map(ast::Attr::cast).any(|attr| {
                attr.meta()
                    .is_some_and(|meta| meta_contains_name(meta, "macro_use"))
                    && attr
                        .syntax()
                        .parent()
                        .is_some_and(|owner| syntax_binding_is_visible(&owner, node, true))
            });
        scope
    }

    pub(super) fn with_defined_macros(mut self, names: &HashSet<String>) -> Self {
        self.defined_macros.extend(names.iter().cloned());
        self
    }

    pub(super) fn with_standard_crates(mut self, std: bool, core: bool) -> Self {
        self.std_is_available = std;
        self.core_is_available = core;
        self
    }
}

pub(super) fn implicit_standard_crates(root: &SyntaxNode) -> (bool, bool) {
    let attributes = root
        .children()
        .filter_map(ast::Attr::cast)
        .filter_map(|attr| attr.meta())
        .collect::<Vec<_>>();
    let has_attribute = |name| {
        attributes
            .iter()
            .any(|meta| meta_contains_name(meta.clone(), name))
    };
    let has_implicit_prelude = !has_attribute("no_implicit_prelude");
    (
        has_implicit_prelude && !has_attribute("no_std"),
        has_implicit_prelude,
    )
}

pub(super) fn exported_macro_names(root: &SyntaxNode) -> impl Iterator<Item = String> {
    root.descendants()
        .filter_map(ast::MacroRules::cast)
        .filter(|macro_rules| has_attribute(macro_rules, "macro_export"))
        .filter_map(|macro_rules| macro_rules.name())
        .map(|name| normalize_macro_path(&name.syntax().text().to_string()))
}

fn has_attribute(owner: &impl HasAttrs, name: &str) -> bool {
    owner
        .attrs()
        .filter_map(|attr| attr.meta())
        .any(|meta| meta_contains_name(meta, name))
}

fn meta_contains_name(meta: ast::Meta, name: &str) -> bool {
    match meta {
        ast::Meta::CfgAttrMeta(meta) => meta.metas().any(|nested| meta_contains_name(nested, name)),
        ast::Meta::UnsafeMeta(meta) => meta
            .meta()
            .is_some_and(|nested| meta_contains_name(nested, name)),
        ast::Meta::PathMeta(meta) => meta_path_contains_name(meta.path(), name),
        ast::Meta::TokenTreeMeta(meta) => meta_path_contains_name(meta.path(), name),
        ast::Meta::KeyValueMeta(meta) => meta_path_contains_name(meta.path(), name),
        ast::Meta::CfgMeta(_) => false,
    }
}

fn meta_path_contains_name(path: Option<ast::Path>, name: &str) -> bool {
    path.and_then(|path| path.as_single_name_ref())
        .is_some_and(|path| raw_identifier_text(path.text()) == name)
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) enum IncludeKind {
    Source,
    Str,
    Bytes,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) enum LocationMacroKind {
    File,
    Line,
    Column,
}

impl IncludeKind {
    pub(super) fn macro_name(self) -> &'static str {
        match self {
            Self::Source => "include!",
            Self::Str => "include_str!",
            Self::Bytes => "include_bytes!",
        }
    }

    pub(super) fn file_description(self) -> &'static str {
        match self {
            Self::Source => "include file",
            Self::Str => "include_str file",
            Self::Bytes => "include_bytes file",
        }
    }

    pub(super) fn source_kind(self) -> BundledSourceKind {
        match self {
            Self::Source => BundledSourceKind::Include,
            Self::Str => BundledSourceKind::IncludeStr,
            Self::Bytes => BundledSourceKind::IncludeBytes,
        }
    }
}

pub(super) fn include_macro_kind(
    call: &ast::MacroCall,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> Option<(IncludeKind, bool)> {
    let original_path = call.path()?.syntax().text().to_string();
    resolve_include_macro_path(&original_path, call.syntax(), root, scope)
}

pub(super) fn resolve_include_macro_path(
    original_path: &str,
    scope_node: &SyntaxNode,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> Option<(IncludeKind, bool)> {
    let path = normalize_macro_path(original_path.strip_prefix("::").unwrap_or(original_path));
    let path = path.as_str();
    if let Some(kind) = builtin_include_path(path) {
        let qualified = path.contains("::");
        if qualified {
            return Some((
                kind,
                qualified_builtin_is_unambiguous(original_path, scope_node, root, scope),
            ));
        }
        return Some((
            kind,
            !scope.has_unknown_macro_import
                && !macro_name_may_be_shadowed(path, scope_node, root, scope),
        ));
    }
    if path.contains("::") {
        return None;
    }

    let mut bindings = Vec::new();
    let mut has_unknown_glob = false;
    for use_item in visible_use_items(scope_node, root) {
        if let Some(tree) = use_item.use_tree() {
            collect_use_bindings(&tree, "", &mut bindings, &mut has_unknown_glob);
        }
    }
    let candidates = bindings
        .iter()
        .filter(|binding| binding.local == path)
        .filter_map(|binding| {
            let kind = builtin_include_path(&binding.full_path)?;
            qualified_builtin_is_unambiguous(&binding.full_path, scope_node, root, scope)
                .then_some(kind)
        })
        .collect::<HashSet<_>>();
    if candidates.len() != 1 {
        return None;
    }
    let kind = *candidates.iter().next()?;
    let ambiguous_binding = bindings.iter().any(|binding| {
        binding.local == path && builtin_include_path(&binding.full_path) != Some(kind)
    });
    Some((
        kind,
        !scope.has_unknown_macro_import
            && !has_unknown_glob
            && !ambiguous_binding
            && !macro_name_is_defined(path, scope),
    ))
}

pub(super) fn location_macro_kind(
    call: &ast::MacroCall,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> Option<(LocationMacroKind, bool)> {
    let original_path = call.path()?.syntax().text().to_string();
    if !token_tree_inner_text(&call.token_tree()?)
        .ok()?
        .trim()
        .is_empty()
    {
        return None;
    }
    resolve_location_macro_path(&original_path, call.syntax(), root, scope)
}

pub(super) fn resolve_location_macro_path(
    original_path: &str,
    scope_node: &SyntaxNode,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> Option<(LocationMacroKind, bool)> {
    let path = normalize_macro_path(original_path.strip_prefix("::").unwrap_or(original_path));
    let path = path.as_str();
    if let Some(kind) = builtin_location_macro_path(path) {
        if path.contains("::") {
            return Some((
                kind,
                qualified_builtin_is_unambiguous(original_path, scope_node, root, scope),
            ));
        }
        return Some((
            kind,
            !scope.has_unknown_macro_import
                && !location_macro_name_may_be_shadowed(path, kind, scope_node, root, scope),
        ));
    }
    if path.contains("::") {
        return None;
    }

    let mut bindings = Vec::new();
    let mut has_unknown_glob = false;
    for use_item in visible_use_items(scope_node, root) {
        if let Some(tree) = use_item.use_tree() {
            collect_use_bindings(&tree, "", &mut bindings, &mut has_unknown_glob);
        }
    }
    let candidates = bindings
        .iter()
        .filter(|binding| binding.local == path)
        .filter_map(|binding| {
            let kind = builtin_location_macro_path(&binding.full_path)?;
            qualified_builtin_is_unambiguous(&binding.full_path, scope_node, root, scope)
                .then_some(kind)
        })
        .collect::<HashSet<_>>();
    if candidates.len() != 1 {
        return None;
    }
    let kind = *candidates.iter().next()?;
    let ambiguous = bindings.iter().any(|binding| {
        binding.local == path && builtin_location_macro_path(&binding.full_path) != Some(kind)
    });
    Some((
        kind,
        !scope.has_unknown_macro_import
            && !has_unknown_glob
            && !ambiguous
            && !macro_name_is_defined(path, scope),
    ))
}

fn builtin_location_macro_path(path: &str) -> Option<LocationMacroKind> {
    match path {
        "file" | "std::file" | "core::file" => Some(LocationMacroKind::File),
        "line" | "std::line" | "core::line" => Some(LocationMacroKind::Line),
        "column" | "std::column" | "core::column" => Some(LocationMacroKind::Column),
        _ => None,
    }
}

fn builtin_include_path(path: &str) -> Option<IncludeKind> {
    match path {
        "include" | "std::include" | "core::include" => Some(IncludeKind::Source),
        "include_str" | "std::include_str" | "core::include_str" => Some(IncludeKind::Str),
        "include_bytes" | "std::include_bytes" | "core::include_bytes" => Some(IncludeKind::Bytes),
        _ => None,
    }
}

fn qualified_builtin_is_unambiguous(
    original_path: &str,
    scope_node: &SyntaxNode,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> bool {
    let normalized_path = normalize_macro_path(original_path.trim_start_matches("::"));
    let Some(prefix) = normalized_path.split("::").next() else {
        return false;
    };
    if prefix == "std" && !scope.std_is_available {
        return false;
    }
    if prefix == "core" && !scope.core_is_available {
        return false;
    }
    let scope_module = containing_module(scope_node);
    let module_shadow = root
        .descendants()
        .filter_map(ast::Module::cast)
        .any(|module| {
            containing_module(module.syntax()) == scope_module
                && module.name().is_some_and(|name| {
                    raw_identifier_text(&name.syntax().text().to_string()) == prefix
                })
        });
    if module_shadow {
        return false;
    }
    let extern_crate_shadow = root
        .descendants()
        .filter_map(ast::ExternCrate::cast)
        .filter(|item| containing_module(item.syntax()) == scope_module)
        .filter(|item| syntax_binding_is_visible(item.syntax(), scope_node, false))
        .any(|item| {
            item.rename()
                .and_then(|rename| rename.name())
                .map(|name| name.syntax().text().to_string())
                .or_else(|| item.name_ref().map(|name| name.syntax().text().to_string()))
                .is_some_and(|name| raw_identifier_text(&name) == prefix)
        });
    if extern_crate_shadow {
        return false;
    }
    let mut bindings = Vec::new();
    let mut has_unknown_glob = false;
    for use_item in visible_use_items(scope_node, root) {
        if let Some(tree) = use_item.use_tree() {
            collect_use_bindings(&tree, "", &mut bindings, &mut has_unknown_glob);
        }
    }
    !has_unknown_glob && !bindings.iter().any(|binding| binding.local == prefix)
}

fn raw_identifier_text(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

fn normalize_macro_path(path: &str) -> String {
    path.split("::")
        .map(raw_identifier_text)
        .collect::<Vec<_>>()
        .join("::")
}

#[derive(Debug)]
struct UseBinding {
    full_path: String,
    local: String,
}

fn collect_use_bindings(
    tree: &ast::UseTree,
    prefix: &str,
    bindings: &mut Vec<UseBinding>,
    has_unknown_glob: &mut bool,
) {
    let own_path = tree
        .path()
        .map(|path| path.syntax().text().to_string())
        .unwrap_or_default();
    let full_path = join_use_path(prefix, &own_path);
    if let Some(list) = tree.use_tree_list() {
        for child in list.use_trees() {
            collect_use_bindings(&child, &full_path, bindings, has_unknown_glob);
        }
        return;
    }
    if tree.star_token().is_some() {
        let normalized = full_path.trim_start_matches("::");
        if !normalized.starts_with("std::") && !normalized.starts_with("core::") {
            *has_unknown_glob = true;
        }
        return;
    }
    let full_path = if full_path.ends_with("::self") {
        full_path.trim_end_matches("::self").to_owned()
    } else {
        full_path
    };
    if full_path.is_empty() {
        return;
    }
    let local = tree
        .rename()
        .and_then(|rename| rename.name())
        .map(|name| name.syntax().text().to_string())
        .or_else(|| full_path.rsplit("::").next().map(str::to_owned));
    if let Some(local) = local
        && local != "_"
    {
        bindings.push(UseBinding {
            full_path: normalize_macro_path(full_path.trim_start_matches("::")),
            local: local.strip_prefix("r#").unwrap_or(&local).to_owned(),
        });
    }
}

fn join_use_path(prefix: &str, path: &str) -> String {
    let path = path.trim_start_matches("::");
    match (prefix.is_empty(), path.is_empty()) {
        (true, _) => path.to_owned(),
        (_, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}::{path}"),
    }
}

fn visible_use_items(scope_node: &SyntaxNode, root: &SyntaxNode) -> Vec<ast::Use> {
    let scope_module = containing_module(scope_node);
    root.descendants()
        .filter_map(ast::Use::cast)
        .filter(|use_item| containing_module(use_item.syntax()) == scope_module)
        .filter(|use_item| syntax_binding_is_visible(use_item.syntax(), scope_node, false))
        .collect()
}

fn containing_module(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.ancestors()
        .skip(1)
        .find_map(ast::Module::cast)
        .map(|module| module.syntax().clone())
}

fn syntax_binding_is_visible(
    binding: &SyntaxNode,
    scope_node: &SyntaxNode,
    textual_scope: bool,
) -> bool {
    if textual_scope && binding.text_range().start() >= scope_node.text_range().start() {
        return false;
    }
    let Some(parent) = binding.parent() else {
        return false;
    };
    scope_node.ancestors().any(|ancestor| ancestor == parent)
}

fn macro_name_may_be_shadowed(
    name: &str,
    scope_node: &SyntaxNode,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> bool {
    if macro_name_is_defined(name, scope) {
        return true;
    }
    let mut bindings = Vec::new();
    let mut has_unknown_glob = false;
    for use_item in visible_use_items(scope_node, root) {
        if let Some(tree) = use_item.use_tree() {
            collect_use_bindings(&tree, "", &mut bindings, &mut has_unknown_glob);
        }
    }
    has_unknown_glob
        || bindings.iter().any(|binding| {
            binding.local == name && builtin_include_path(&binding.full_path).is_none()
        })
}

fn location_macro_name_may_be_shadowed(
    local: &str,
    kind: LocationMacroKind,
    scope_node: &SyntaxNode,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> bool {
    if macro_name_is_defined(local, scope) {
        return true;
    }
    let mut bindings = Vec::new();
    let mut has_unknown_glob = false;
    for use_item in visible_use_items(scope_node, root) {
        if let Some(tree) = use_item.use_tree() {
            collect_use_bindings(&tree, "", &mut bindings, &mut has_unknown_glob);
        }
    }
    has_unknown_glob
        || bindings.iter().any(|binding| {
            binding.local == local && builtin_location_macro_path(&binding.full_path) != Some(kind)
        })
}

fn macro_name_is_defined(name: &str, scope: &MacroScope) -> bool {
    scope.defined_macros.contains(name)
}

pub(super) fn static_include_argument(
    call: &ast::MacroCall,
    edition: RustEdition,
    environment: &HashMap<String, String>,
    source_root: &SyntaxNode,
    scope_node: &SyntaxNode,
    trust_unqualified_macros: bool,
    scope: &MacroScope,
) -> Result<String, String> {
    let tree = call
        .token_tree()
        .ok_or_else(|| "missing macro argument".to_owned())?;
    let expression_text = token_tree_inner_text(&tree)?;
    eval_static_string_expression(
        &expression_text,
        edition,
        environment,
        source_root,
        scope_node,
        trust_unqualified_macros,
        scope,
    )
}

fn eval_static_string_expression(
    source: &str,
    edition: RustEdition,
    environment: &HashMap<String, String>,
    source_root: &SyntaxNode,
    scope_node: &SyntaxNode,
    trust_unqualified_macros: bool,
    scope: &MacroScope,
) -> Result<String, String> {
    let parsed = ast::Expr::parse(source.trim(), edition.into());
    if !parsed.errors().is_empty() {
        return Err("path is not a valid Rust expression".to_owned());
    }
    match parsed.tree() {
        ast::Expr::Literal(literal) => match literal.kind() {
            LiteralKind::String(value) => string_literal_value(&value, "path string"),
            _ => Err("path must evaluate to a string".to_owned()),
        },
        ast::Expr::MacroExpr(expression) => {
            let call = expression
                .macro_call()
                .ok_or_else(|| "invalid macro path expression".to_owned())?;
            let path = call
                .path()
                .ok_or_else(|| "macro path expression has no path".to_owned())?
                .syntax()
                .text()
                .to_string();
            let tree = call
                .token_tree()
                .ok_or_else(|| format!("{path}! has no arguments"))?;
            match resolve_static_macro(
                &path,
                source_root,
                scope_node,
                trust_unqualified_macros,
                scope,
            ) {
                Some("concat") => {
                    let parts = split_token_tree_arguments(&tree)
                        .ok_or_else(|| "concat! has invalid arguments".to_owned())?;
                    let mut result = String::new();
                    for part in parts {
                        result.push_str(&eval_concat_part(
                            &part,
                            edition,
                            environment,
                            source_root,
                            scope_node,
                            trust_unqualified_macros,
                            scope,
                        )?);
                    }
                    Ok(result)
                }
                Some("env") => {
                    let parts = split_token_tree_arguments(&tree)
                        .ok_or_else(|| "env! has invalid arguments".to_owned())?;
                    if !(1..=2).contains(&parts.len()) {
                        return Err("env! requires one or two string arguments".to_owned());
                    }
                    let key = eval_plain_string_literal(&parts[0], edition)?;
                    if parts.len() == 2 {
                        let _ = eval_plain_string_literal(&parts[1], edition)?;
                    }
                    environment
                        .get(&key)
                        .cloned()
                        .or_else(|| std::env::var(&key).ok())
                        .ok_or_else(|| {
                            format!(
                                "environment variable {key:?} is unavailable; pass it with --env"
                            )
                        })
                }
                Some("stringify") => eval_stringify(&tree),
                _ => Err(format!(
                    "path macro {path}! cannot be evaluated without macro expansion"
                )),
            }
        }
        _ => Err("path must be a string literal, concat!, or env! expression".to_owned()),
    }
}

fn eval_stringify(tree: &ast::TokenTree) -> Result<String, String> {
    let left = tree
        .left_delimiter_token()
        .ok_or_else(|| "stringify! has no opening delimiter".to_owned())?;
    let right = tree
        .right_delimiter_token()
        .ok_or_else(|| "stringify! has no closing delimiter".to_owned())?;
    let mut saw_significant = false;
    let mut trivia_after_significant = false;
    for token in tree
        .syntax()
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
    {
        if token == left || token == right {
            continue;
        }
        if ast::Comment::cast(token.clone()).is_some() {
            return Err("stringify! paths containing comments are retained because Rust normalizes comment trivia".to_owned());
        }
        if token.kind().is_trivia() {
            trivia_after_significant |= saw_significant;
        } else {
            if trivia_after_significant {
                return Err("stringify! paths containing internal whitespace are retained because Rust normalizes token spacing".to_owned());
            }
            saw_significant = true;
        }
    }
    Ok(token_tree_inner_text(tree)?.trim().to_owned())
}

fn eval_concat_part(
    source: &str,
    edition: RustEdition,
    environment: &HashMap<String, String>,
    source_root: &SyntaxNode,
    scope_node: &SyntaxNode,
    trust_unqualified_macros: bool,
    scope: &MacroScope,
) -> Result<String, String> {
    let parsed = ast::Expr::parse(source.trim(), edition.into());
    if !parsed.errors().is_empty() {
        return Err(format!("invalid concat! argument {source:?}"));
    }
    match parsed.tree() {
        ast::Expr::Literal(literal) => match literal.kind() {
            LiteralKind::String(value) => string_literal_value(&value, "concat! string"),
            LiteralKind::Char(value) => char_literal_value(&value),
            LiteralKind::IntNumber(value) => concat_integer(&value),
            LiteralKind::FloatNumber(value) => concat_float(&value),
            LiteralKind::Bool(value) => Ok(value.to_string()),
            LiteralKind::ByteString(_) | LiteralKind::CString(_) | LiteralKind::Byte(_) => {
                Err("byte and C string literals are not supported by concat!".to_owned())
            }
        },
        ast::Expr::PrefixExpr(prefix) if prefix.op_kind() == Some(ast::UnaryOp::Neg) => {
            let expression = prefix
                .expr()
                .ok_or_else(|| "concat! negation has no operand".to_owned())?;
            match expression {
                ast::Expr::Literal(literal) => match literal.kind() {
                    LiteralKind::IntNumber(value) => {
                        concat_integer(&value).map(|value| format!("-{value}"))
                    }
                    LiteralKind::FloatNumber(value) => {
                        concat_float(&value).map(|value| format!("-{value}"))
                    }
                    _ => Err(format!(
                        "concat! argument {source:?} is not a negative number literal"
                    )),
                },
                _ => Err(format!(
                    "concat! argument {source:?} is not a negative number literal"
                )),
            }
        }
        ast::Expr::MacroExpr(_) => eval_static_string_expression(
            source,
            edition,
            environment,
            source_root,
            scope_node,
            trust_unqualified_macros,
            scope,
        ),
        _ => Err(format!(
            "concat! argument {source:?} requires unsupported macro expansion"
        )),
    }
}

fn concat_integer(value: &ast::IntNumber) -> Result<String, String> {
    if value.suffix().is_some_and(|suffix| {
        !matches!(
            suffix,
            "u8" | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "f32"
                | "f64"
        )
    }) {
        return Err(format!("invalid concat! integer suffix in {value}"));
    }
    value
        .value()
        .map(|value| value.to_string())
        .map_err(|error| format!("invalid concat! integer: {error}"))
}

fn concat_float(value: &ast::FloatNumber) -> Result<String, String> {
    if value
        .suffix()
        .is_some_and(|suffix| !matches!(suffix, "f32" | "f64"))
    {
        return Err(format!("invalid concat! float suffix in {value}"));
    }
    Ok(value.value_string())
}

fn string_literal_value(value: &ast::String, description: &str) -> Result<String, String> {
    if string_literal_suffix(value.text()).is_some() {
        return Err(format!("invalid {description} suffix in {value}"));
    }
    value
        .value()
        .map(|value| value.into_owned())
        .map_err(|error| format!("invalid {description}: {error:?}"))
}

fn string_literal_suffix(text: &str) -> Option<&str> {
    let quote = text.rfind('"')?;
    let mut suffix_start = quote + 1;
    if text.starts_with('r') {
        let opening_quote = text.find('"')?;
        suffix_start += text[1..opening_quote]
            .chars()
            .take_while(|character| *character == '#')
            .count();
    }
    (suffix_start < text.len()).then(|| &text[suffix_start..])
}

fn char_literal_value(value: &ast::Char) -> Result<String, String> {
    let text = value.text();
    let quote = text.rfind('\'').unwrap_or_default();
    if quote + 1 < text.len() {
        return Err(format!("invalid concat! character suffix in {value}"));
    }
    value
        .value()
        .map(|value| value.to_string())
        .map_err(|error| format!("invalid concat! character: {error:?}"))
}

fn eval_plain_string_literal(source: &str, edition: RustEdition) -> Result<String, String> {
    let parsed = ast::Expr::parse(source.trim(), edition.into());
    if !parsed.errors().is_empty() {
        return Err(format!("expected a string literal, found {source:?}"));
    }
    match parsed.tree() {
        ast::Expr::Literal(literal) => match literal.kind() {
            LiteralKind::String(value) => string_literal_value(&value, "string literal"),
            _ => Err(format!("expected a string literal, found {source:?}")),
        },
        _ => Err(format!("expected a string literal, found {source:?}")),
    }
}

fn resolve_static_macro(
    original_path: &str,
    root: &SyntaxNode,
    scope_node: &SyntaxNode,
    trust_unqualified: bool,
    scope: &MacroScope,
) -> Option<&'static str> {
    let path = normalize_macro_path(original_path.strip_prefix("::").unwrap_or(original_path));
    let path = path.as_str();
    if path.contains("::") {
        return qualified_builtin_is_unambiguous(original_path, scope_node, root, scope)
            .then(|| builtin_static_macro_path(path))
            .flatten();
    }

    if let Some(canonical @ ("concat" | "env" | "stringify")) = builtin_static_macro_path(path) {
        if trust_unqualified
            || (!scope.has_unknown_macro_import
                && !static_macro_name_may_be_shadowed(path, canonical, scope_node, root, scope))
        {
            return Some(canonical);
        }
        return None;
    }

    let mut bindings = Vec::new();
    let mut has_unknown_glob = false;
    for use_item in visible_use_items(scope_node, root) {
        if let Some(tree) = use_item.use_tree() {
            collect_use_bindings(&tree, "", &mut bindings, &mut has_unknown_glob);
        }
    }
    let candidates = bindings
        .iter()
        .filter(|binding| binding.local == path)
        .filter_map(|binding| {
            let canonical = builtin_static_macro_path(&binding.full_path)?;
            qualified_builtin_is_unambiguous(&binding.full_path, scope_node, root, scope)
                .then_some(canonical)
        })
        .filter(|canonical| matches!(*canonical, "concat" | "env" | "stringify"))
        .collect::<HashSet<_>>();
    if candidates.len() != 1 {
        return None;
    }
    let canonical = *candidates.iter().next()?;
    let ambiguous = bindings.iter().any(|binding| {
        binding.local == path && builtin_static_macro_path(&binding.full_path) != Some(canonical)
    });
    (trust_unqualified
        || (!scope.has_unknown_macro_import
            && !has_unknown_glob
            && !ambiguous
            && !macro_name_is_defined(path, scope)))
    .then_some(canonical)
}

fn builtin_static_macro_path(path: &str) -> Option<&'static str> {
    match path {
        "concat" | "std::concat" | "core::concat" => Some("concat"),
        "env" | "std::env" | "core::env" => Some("env"),
        "stringify" | "std::stringify" | "core::stringify" => Some("stringify"),
        _ => None,
    }
}

fn static_macro_name_may_be_shadowed(
    local: &str,
    canonical: &str,
    scope_node: &SyntaxNode,
    root: &SyntaxNode,
    scope: &MacroScope,
) -> bool {
    if macro_name_is_defined(local, scope) {
        return true;
    }
    let mut bindings = Vec::new();
    let mut has_unknown_glob = false;
    for use_item in visible_use_items(scope_node, root) {
        if let Some(tree) = use_item.use_tree() {
            collect_use_bindings(&tree, "", &mut bindings, &mut has_unknown_glob);
        }
    }
    has_unknown_glob
        || bindings.iter().any(|binding| {
            binding.local == local
                && builtin_static_macro_path(&binding.full_path) != Some(canonical)
        })
}

fn token_tree_inner_text(tree: &ast::TokenTree) -> Result<String, String> {
    let left = tree
        .left_delimiter_token()
        .ok_or_else(|| "missing opening delimiter".to_owned())?;
    let right = tree
        .right_delimiter_token()
        .ok_or_else(|| "missing closing delimiter".to_owned())?;
    let text = tree.syntax().text().to_string();
    let left_len = left.text().len();
    let right_len = right.text().len();
    if text.len() < left_len + right_len {
        return Err("invalid macro delimiters".to_owned());
    }
    Ok(text[left_len..text.len() - right_len].to_owned())
}

fn split_token_tree_arguments(tree: &ast::TokenTree) -> Option<Vec<String>> {
    let mut parts = vec![String::new()];
    for element in tree.token_trees_and_tokens() {
        match element {
            NodeOrToken::Token(token)
                if tree.left_delimiter_token().as_ref() == Some(&token)
                    || tree.right_delimiter_token().as_ref() == Some(&token) => {}
            NodeOrToken::Token(token) if token.text() == "," => parts.push(String::new()),
            NodeOrToken::Token(token) => parts.last_mut()?.push_str(token.text()),
            NodeOrToken::Node(node) => parts
                .last_mut()?
                .push_str(&node.syntax().text().to_string()),
        }
    }
    if parts.last().is_some_and(|part| part.trim().is_empty()) {
        parts.pop();
    }
    Some(parts)
}

pub(super) fn byte_string_expression(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 8 + 40);
    output.push_str("(&[");
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&format!("0x{byte:02x}u8"));
    }
    output.push_str(&format!("] as &'static [u8; {}])", bytes.len()));
    output
}
