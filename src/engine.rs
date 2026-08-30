use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use ra_ap_proc_macro_srv::EnvSnapshot;
use ra_ap_syntax::{AstNode, SourceFile, SyntaxNode, ast};

use super::cfg::{CfgExpr, syntax_is_active};
use super::directive::{BundleDirective, bundle_directive};
use super::edit::{ByteRange, Edit, apply_edits, byte_range, overlaps_any};
use super::include::{
    IncludeKind, LocationMacroKind, MacroScope, byte_string_expression, exported_macro_names,
    implicit_standard_crates, include_macro_kind, location_macro_kind, resolve_include_macro_path,
    resolve_location_macro_path, static_include_argument,
};
use super::macro_rules::{hidden_location_macros, hidden_macro_calls, relevant_transcribers};
use super::module_resolver::{
    ModuleFileVariant, SourceContext, containing_module_path, module_file_variants,
    module_inlining_condition, module_path_is_external, normalize_external_name,
    qualified_module_path,
};
use super::proc_macro::ProcMacroExpander;
use super::source::{
    canonical_file, display_path, format_parse_errors, read_rust_source, resolve_entry_file,
    source_position,
};
use super::{BundleOptions, BundleResult, BundledSource, BundledSourceKind};

pub(super) fn bundle_file(
    entry_file: &Path,
    options: BundleOptions,
) -> Result<BundleResult, String> {
    let entry = resolve_entry_file(entry_file)?;
    let context = SourceContext::for_crate_root(&entry.logical)?;
    let proc_macro_environment = EnvSnapshot::default();
    let mut state = Bundler::new(
        options,
        context.physical_dir.clone(),
        &proc_macro_environment,
    )?;
    state.sources.push(BundledSource {
        file_path: display_path(&entry.canonical),
        module_path: "crate".to_owned(),
        kind: BundledSourceKind::Entry,
    });

    let source = read_rust_source(&entry.canonical, true)?;
    state.discover_exported_macros(&entry.canonical, &context, &source, &mut HashSet::new());
    let entry_scope = {
        let parsed = SourceFile::parse(&source, state.options.edition.into());
        let tree = parsed.tree();
        let (std, core) = implicit_standard_crates(tree.syntax());
        MacroScope::default().with_standard_crates(std, core)
    };
    state.active_modules.push(entry.canonical.clone());
    let code = state
        .bundle_rust_source(
            &entry.canonical,
            &source,
            &SourceSite {
                logical_file_path: &entry.logical,
                context: &context,
                module_path: "crate",
                external_admitted: false,
                macro_scope: &entry_scope,
                retention: RetentionSafety::entry(),
            },
        )
        .map_err(ExpansionError::into_message)?;
    state.active_modules.pop();

    Ok(BundleResult {
        code,
        entry_file: display_path(&entry.canonical),
        bundled_source_list: state.sources,
    })
}

struct Bundler<'env> {
    options: BundleOptions,
    environment: HashMap<String, String>,
    external: HashSet<String>,
    sources: Vec<BundledSource>,
    source_file_count: usize,
    active_modules: Vec<PathBuf>,
    active_includes: Vec<PathBuf>,
    exported_macros: HashSet<String>,
    bundle_dir: PathBuf,
    proc_macros: ProcMacroExpander<'env>,
}

#[derive(Debug)]
enum ExpansionError {
    Recoverable(String),
    Atomic(String),
    Preserve(String),
    Fatal(String),
}

impl ExpansionError {
    fn recoverable(message: impl Into<String>) -> Self {
        Self::Recoverable(message.into())
    }

    fn fatal(message: impl Into<String>) -> Self {
        Self::Fatal(message.into())
    }

    fn preserve(message: impl Into<String>) -> Self {
        Self::Preserve(message.into())
    }

    fn required(self) -> Self {
        match self {
            Self::Preserve(message) => Self::Preserve(message),
            Self::Recoverable(message) | Self::Atomic(message) | Self::Fatal(message) => {
                Self::Fatal(message)
            }
        }
    }

    fn preserved(self) -> Self {
        match self {
            Self::Recoverable(message) | Self::Atomic(message) | Self::Preserve(message) => {
                Self::Preserve(message)
            }
            Self::Fatal(message) => Self::Fatal(message),
        }
    }

    fn is_recoverable(&self) -> bool {
        matches!(self, Self::Recoverable(_) | Self::Atomic(_))
    }

    fn is_atomic(&self) -> bool {
        matches!(self, Self::Atomic(_))
    }

    fn is_preserve(&self) -> bool {
        matches!(self, Self::Preserve(_))
    }

    fn into_message(self) -> String {
        match self {
            Self::Recoverable(message)
            | Self::Atomic(message)
            | Self::Preserve(message)
            | Self::Fatal(message) => message,
        }
    }
}

type ExpansionResult<T> = Result<T, ExpansionError>;

#[derive(Clone, Debug)]
struct ExpansionCheckpoint {
    source_count: usize,
    source_list_len: usize,
}

#[derive(Clone, Copy, Debug)]
struct SourceSite<'a> {
    logical_file_path: &'a Path,
    context: &'a SourceContext,
    module_path: &'a str,
    external_admitted: bool,
    macro_scope: &'a MacroScope,
    retention: RetentionSafety,
}

#[derive(Clone, Copy, Debug)]
struct RetentionSafety {
    module_paths: bool,
    include_paths: bool,
    source_positions: bool,
}

impl RetentionSafety {
    fn entry() -> Self {
        Self {
            module_paths: true,
            include_paths: true,
            source_positions: true,
        }
    }
}

impl<'env> Bundler<'env> {
    fn new(
        options: BundleOptions,
        bundle_dir: PathBuf,
        proc_macro_environment: &'env EnvSnapshot,
    ) -> Result<Self, String> {
        let environment = options.environment.iter().cloned().collect();
        let external = options
            .external
            .iter()
            .map(|name| normalize_external_name(name, options.edition.into()))
            .collect::<Result<HashSet<_>, _>>()?;
        let proc_macros = ProcMacroExpander::new(
            &options.proc_macros,
            options.environment.clone(),
            bundle_dir.clone(),
            proc_macro_environment,
            options.edition.into(),
        )?;
        Ok(Self {
            options,
            environment,
            external,
            sources: Vec::new(),
            source_file_count: 0,
            active_modules: Vec::new(),
            active_includes: Vec::new(),
            exported_macros: HashSet::new(),
            bundle_dir,
            proc_macros,
        })
    }

    fn discover_exported_macros(
        &mut self,
        file_path: &Path,
        context: &SourceContext,
        source: &str,
        visited: &mut HashSet<PathBuf>,
    ) {
        if !visited.insert(file_path.to_path_buf()) {
            return;
        }
        let parsed = SourceFile::parse(source, self.options.edition.into());
        if !parsed.errors().is_empty() {
            return;
        }
        let tree = parsed.tree();
        let root = tree.syntax();
        self.exported_macros.extend(exported_macro_names(root));

        for module in root.descendants().filter_map(ast::Module::cast) {
            if module.semicolon_token().is_none() {
                continue;
            }
            let Ok(variants) = module_file_variants(&module, context, file_path) else {
                continue;
            };
            for variant in variants {
                let Ok(resolved) = variant.resolution else {
                    continue;
                };
                let Ok(canonical) = canonical_file(&resolved.path, "module file") else {
                    continue;
                };
                let Ok(child_source) = read_rust_source(&canonical, false) else {
                    continue;
                };
                let Ok(child_context) = SourceContext::for_module_file(
                    &resolved.path,
                    resolved.loaded_through_path_attr,
                ) else {
                    continue;
                };
                self.discover_exported_macros(&canonical, &child_context, &child_source, visited);
            }
        }

        for call in root.descendants().filter_map(ast::MacroCall::cast) {
            let scope = self.macro_scope_at(call.syntax(), root, &MacroScope::default());
            if !matches!(
                include_macro_kind(&call, root, &scope),
                Some((IncludeKind::Source, _))
            ) {
                continue;
            }
            let Ok(argument) = static_include_argument(
                &call,
                self.options.edition,
                &self.environment,
                root,
                call.syntax(),
                false,
                &scope,
            ) else {
                continue;
            };
            let included_path = context.physical_dir.join(argument);
            let Ok(canonical) = canonical_file(&included_path, "include file") else {
                continue;
            };
            let Ok(included_source) = read_rust_source(&canonical, false) else {
                continue;
            };
            let include_dir = included_path
                .parent()
                .unwrap_or(&context.physical_dir)
                .to_path_buf();
            let include_context = SourceContext {
                physical_dir: include_dir.clone(),
                default_module_dir: include_dir,
            };
            self.discover_exported_macros(&canonical, &include_context, &included_source, visited);
        }
    }

    fn bundle_rust_source(
        &mut self,
        file_path: &Path,
        source: &str,
        site: &SourceSite<'_>,
    ) -> ExpansionResult<String> {
        let edition = self.options.edition.into();
        let source = self
            .proc_macros
            .expand_source(file_path, source, edition)
            .map_err(ExpansionError::fatal)?;
        let parsed = SourceFile::parse(&source, edition);
        if !parsed.errors().is_empty() {
            return Err(ExpansionError::recoverable(format_parse_errors(
                file_path,
                &source,
                &parsed.errors(),
            )));
        }
        self.bundle_syntax(file_path, &source, parsed.tree().syntax(), site)
    }

    fn bundle_syntax(
        &mut self,
        file_path: &Path,
        source: &str,
        root: &SyntaxNode,
        site: &SourceSite<'_>,
    ) -> ExpansionResult<String> {
        let logical_file_path = site.logical_file_path;
        let context = site.context;
        let module_path = site.module_path;
        let external_admitted = site.external_admitted;
        let macro_scope = site.macro_scope;
        let retained_module_paths_are_stable = site.retention.module_paths;
        let retained_include_paths_are_stable = site.retention.include_paths;
        let source_positions_are_stable = site.retention.source_positions;
        self.exported_macros.extend(exported_macro_names(root));
        let transcribers = relevant_transcribers(root);
        if transcribers.iter().any(|transcriber| {
            let transcriber_scope = self.macro_scope_at(&transcriber.definition, root, macro_scope);
            hidden_location_macros(&transcriber.definition)
                .into_iter()
                .filter(|call| call.start >= transcriber.start && call.end <= transcriber.end)
                .any(|call| {
                    resolve_location_macro_path(
                        &call.path,
                        &transcriber.definition,
                        root,
                        &transcriber_scope,
                    )
                    .is_some()
                })
        }) {
            if source_positions_are_stable {
                return Ok(source.to_owned());
            }
            return Err(ExpansionError::recoverable(format!(
                "location-sensitive macro inside macro_rules! in {} cannot be moved safely",
                file_path.display()
            )));
        }

        for outer_call in root.descendants().filter_map(ast::MacroCall::cast) {
            if !syntax_is_active(outer_call.syntax(), file_path).map_err(ExpansionError::fatal)? {
                continue;
            }
            let Some(arguments) = outer_call.token_tree() else {
                continue;
            };
            let outer_scope = self.macro_scope_at(outer_call.syntax(), root, macro_scope);
            for hidden in hidden_macro_calls(arguments.syntax()) {
                if hidden.arguments_are_empty
                    && resolve_location_macro_path(
                        &hidden.path,
                        outer_call.syntax(),
                        root,
                        &outer_scope,
                    )
                    .is_some()
                {
                    if source_positions_are_stable {
                        return Ok(source.to_owned());
                    }
                    return Err(ExpansionError::recoverable(format!(
                        "location-sensitive macro inside another macro in {} cannot be moved safely",
                        file_path.display()
                    )));
                }
                if !retained_include_paths_are_stable
                    && resolve_include_macro_path(
                        &hidden.path,
                        outer_call.syntax(),
                        root,
                        &outer_scope,
                    )
                    .is_some()
                {
                    return Err(ExpansionError::recoverable(format!(
                        "include macro inside another macro in {} cannot be moved safely",
                        file_path.display()
                    )));
                }
            }
        }

        for call in root.descendants().filter_map(ast::MacroCall::cast) {
            let call_macro_scope = self.macro_scope_at(call.syntax(), root, macro_scope);
            let Some((_, name_is_unambiguous)) =
                location_macro_kind(&call, root, &call_macro_scope)
            else {
                continue;
            };
            if !syntax_is_active(call.syntax(), file_path).map_err(ExpansionError::fatal)? {
                continue;
            }
            let directive = bundle_directive(call.syntax(), root, source, file_path)
                .map_err(ExpansionError::fatal)?;
            if directive == BundleDirective::NoBundle
                || (!name_is_unambiguous && directive != BundleDirective::Bundle)
            {
                if source_positions_are_stable {
                    return Ok(source.to_owned());
                }
                return Err(ExpansionError::preserve(format!(
                    "location-sensitive macro retained in {}",
                    file_path.display()
                )));
            }
        }
        if !self.options.inline_includes
            && !retained_include_paths_are_stable
            && root
                .descendants()
                .filter_map(ast::MacroCall::cast)
                .any(|call| {
                    let scope = self.macro_scope_at(call.syntax(), root, macro_scope);
                    include_macro_kind(&call, root, &scope).is_some()
                })
        {
            return Err(ExpansionError::preserve(format!(
                "nested include retained by no-inline-includes in {}",
                file_path.display()
            )));
        }
        let mut edits = Vec::new();
        let mut pending_module_edits = Vec::new();
        let mut occupied_ranges = HashSet::new();

        for transcriber in transcribers {
            let transcriber_scope = self.macro_scope_at(&transcriber.definition, root, macro_scope);
            let transcriber_source = transcriber.source.clone();
            let parsed = ast::Expr::parse(&transcriber_source, self.options.edition.into());
            if !parsed.errors().is_empty() {
                if !transcriber.has_dependency
                    || (retained_module_paths_are_stable && retained_include_paths_are_stable)
                {
                    if transcriber_source != transcriber.source {
                        edits.push(Edit {
                            start: transcriber.start,
                            end: transcriber.end,
                            replacement: transcriber_source,
                        });
                    }
                    continue;
                }
                return Err(ExpansionError::recoverable(format!(
                    "dependency inside macro_rules! in {} cannot be parsed safely",
                    file_path.display()
                )));
            }
            let bundled = self.bundle_syntax(
                file_path,
                &transcriber_source,
                parsed.tree().syntax(),
                &SourceSite {
                    macro_scope: &transcriber_scope,
                    ..*site
                },
            )?;
            edits.push(Edit {
                start: transcriber.start,
                end: transcriber.end,
                replacement: bundled,
            });
        }

        for module in root.descendants().filter_map(ast::Module::cast) {
            let Some(semicolon) = module.semicolon_token() else {
                continue;
            };
            let directive = bundle_directive(module.syntax(), root, source, file_path)
                .map_err(ExpansionError::fatal)?;
            if directive == BundleDirective::NoBundle {
                if !retained_module_paths_are_stable {
                    return Err(ExpansionError::preserve(format!(
                        "nested module marked no-bundle in {}",
                        file_path.display()
                    )));
                }
                continue;
            }
            if !syntax_is_active(module.syntax(), file_path).map_err(ExpansionError::fatal)? {
                continue;
            }
            let module_range = byte_range(module.syntax().text_range());
            if !occupied_ranges.insert((module_range.start, module_range.end)) {
                continue;
            }

            let child_module_path = qualified_module_path(&module, module_path, file_path)
                .map_err(ExpansionError::recoverable)?;
            if !external_admitted
                && directive != BundleDirective::Bundle
                && module_path_is_external(&child_module_path, &self.external)
            {
                if !retained_module_paths_are_stable {
                    return Err(ExpansionError::preserve(format!(
                        "external nested module retained in {}",
                        file_path.display()
                    )));
                }
                continue;
            }

            let inline_condition = if directive == BundleDirective::Bundle {
                CfgExpr::True
            } else {
                match module_inlining_condition(&module, file_path) {
                    Ok(condition) => condition,
                    Err(_) if retained_module_paths_are_stable => continue,
                    Err(error) => return Err(ExpansionError::preserve(error)),
                }
            };
            if inline_condition.is_false() {
                if !retained_module_paths_are_stable {
                    return Err(ExpansionError::preserve(format!(
                        "nested module with an expansion-sensitive attribute retained in {}",
                        file_path.display()
                    )));
                }
                continue;
            }

            let variants = match module_file_variants(&module, context, file_path) {
                Ok(variants) => variants,
                Err(error) if directive != BundleDirective::Bundle => {
                    if !retained_module_paths_are_stable {
                        return Err(ExpansionError::preserve(error));
                    }
                    continue;
                }
                Err(error) => return Err(ExpansionError::fatal(error)),
            };
            let mut branches = Vec::new();
            for variant in variants {
                let variant_condition = variant.condition.clone();
                let expansion_condition =
                    CfgExpr::all([variant_condition.clone(), inline_condition.clone()]);
                if directive != BundleDirective::Bundle {
                    let retention_condition =
                        CfgExpr::all([variant_condition, inline_condition.clone().not()]);
                    if !retention_condition.is_false() {
                        if !retained_module_paths_are_stable {
                            return Err(ExpansionError::preserve(format!(
                                "conditional expansion-sensitive module retained in {}",
                                file_path.display()
                            )));
                        }
                        branches.push(ModuleBranch::Retained(retention_condition));
                    }
                }
                if expansion_condition.is_false() {
                    continue;
                }

                let checkpoint = self.checkpoint();
                let expansion = self.expand_module_variant(
                    variant,
                    &module,
                    root,
                    file_path,
                    &child_module_path,
                    macro_scope,
                    external_admitted,
                    directive,
                );
                match expansion {
                    Ok(bundled) => branches.push(ModuleBranch::Inlined {
                        condition: expansion_condition,
                        source: bundled,
                    }),
                    Err(error) if error.is_preserve() => {
                        self.rollback(checkpoint);
                        if !retained_module_paths_are_stable {
                            return Err(error);
                        }
                        branches.push(ModuleBranch::Retained(expansion_condition));
                    }
                    Err(error)
                        if directive != BundleDirective::Bundle && error.is_recoverable() =>
                    {
                        self.rollback(checkpoint);
                        if error.is_atomic() && !source_positions_are_stable {
                            return Err(error);
                        }
                        if external_admitted {
                            return Err(error);
                        }
                        if !retained_module_paths_are_stable {
                            return Err(error.preserved());
                        }
                        branches.push(ModuleBranch::Retained(expansion_condition));
                    }
                    Err(error) if directive == BundleDirective::Bundle => {
                        return Err(error.required());
                    }
                    Err(error) => return Err(error),
                }
            }

            branches.retain(|branch| !branch.condition().is_false());
            match branches.as_slice() {
                [] => {}
                [ModuleBranch::Retained(condition)] if condition.is_true() => {}
                [
                    ModuleBranch::Inlined {
                        condition,
                        source: child_source,
                    },
                ] if condition.is_true() => edits.push(Edit {
                    start: byte_range(semicolon.text_range()).start,
                    end: byte_range(semicolon.text_range()).end,
                    replacement: module_replacement(
                        source,
                        byte_range(semicolon.text_range()).start,
                        child_source,
                    ),
                }),
                _ => pending_module_edits.push(PendingModuleEdit {
                    module_range,
                    semicolon_range: byte_range(semicolon.text_range()),
                    branches,
                }),
            }
        }

        for call in root.descendants().filter_map(ast::MacroCall::cast) {
            let call_macro_scope = self.macro_scope_at(call.syntax(), root, macro_scope);
            let Some((kind, name_is_unambiguous)) =
                location_macro_kind(&call, root, &call_macro_scope)
            else {
                continue;
            };
            let directive = bundle_directive(call.syntax(), root, source, file_path)
                .map_err(ExpansionError::fatal)?;
            if directive == BundleDirective::NoBundle
                || (!name_is_unambiguous && directive != BundleDirective::Bundle)
            {
                continue;
            }
            if !syntax_is_active(call.syntax(), file_path).map_err(ExpansionError::fatal)? {
                continue;
            }
            let range = byte_range(call.syntax().text_range());
            if overlaps_any(&edits, &range) {
                continue;
            }
            edits.push(Edit {
                start: range.start,
                end: range.end,
                replacement: location_macro_replacement(
                    kind,
                    logical_file_path,
                    source,
                    range.start,
                ),
            });
        }

        if self.options.inline_includes {
            for call in root.descendants().filter_map(ast::MacroCall::cast) {
                let call_macro_scope = self.macro_scope_at(call.syntax(), root, macro_scope);
                let Some((kind, name_is_unambiguous)) =
                    include_macro_kind(&call, root, &call_macro_scope)
                else {
                    continue;
                };
                let directive = bundle_directive(call.syntax(), root, source, file_path)
                    .map_err(ExpansionError::fatal)?;
                if directive == BundleDirective::NoBundle {
                    if !retained_include_paths_are_stable {
                        return Err(ExpansionError::preserve(format!(
                            "nested include marked no-bundle in {}",
                            file_path.display()
                        )));
                    }
                    continue;
                }
                if !name_is_unambiguous && directive != BundleDirective::Bundle {
                    if !retained_include_paths_are_stable {
                        return Err(ExpansionError::preserve(format!(
                            "ambiguous nested include macro in {}",
                            file_path.display()
                        )));
                    }
                    continue;
                }
                if !syntax_is_active(call.syntax(), file_path).map_err(ExpansionError::fatal)? {
                    continue;
                }
                let range = byte_range(call.syntax().text_range());
                if overlaps_any(&edits, &range) {
                    continue;
                }
                let checkpoint = self.checkpoint();
                let expansion: ExpansionResult<Edit> = (|| {
                    let argument = static_include_argument(
                        &call,
                        self.options.edition,
                        &self.environment,
                        root,
                        call.syntax(),
                        directive == BundleDirective::Bundle,
                        &call_macro_scope,
                    )
                    .map_err(|error| {
                        ExpansionError::recoverable(format!(
                            "{} at {}: {error}",
                            kind.macro_name(),
                            source_position(file_path, source, range.start)
                        ))
                    })?;
                    let included_path = context.physical_dir.join(argument);
                    let canonical = canonical_file(&included_path, kind.file_description())
                        .map_err(ExpansionError::recoverable)?;
                    let include_module_path =
                        containing_module_path(call.syntax(), module_path, file_path)
                            .map_err(ExpansionError::recoverable)?;
                    let site = IncludeSite {
                        surrounding_context: context,
                        module_path: &include_module_path,
                        external_admitted: external_admitted
                            || directive == BundleDirective::Bundle,
                        macro_scope: &call_macro_scope,
                    };
                    let replacement =
                        self.expand_include(kind, &canonical, &included_path, &site)?;
                    Ok(Edit {
                        start: range.start,
                        end: range.end,
                        replacement,
                    })
                })();
                match expansion {
                    Ok(edit) => edits.push(edit),
                    Err(error) if error.is_preserve() => {
                        self.rollback(checkpoint);
                        if !retained_include_paths_are_stable {
                            return Err(error);
                        }
                    }
                    Err(error)
                        if directive != BundleDirective::Bundle && error.is_recoverable() =>
                    {
                        self.rollback(checkpoint);
                        if error.is_atomic() && !source_positions_are_stable {
                            return Err(error);
                        }
                        if external_admitted {
                            return Err(error);
                        }
                        if !retained_include_paths_are_stable {
                            return Err(error.preserved());
                        }
                    }
                    Err(error) if directive == BundleDirective::Bundle => {
                        return Err(error.required());
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        for pending in pending_module_edits {
            let mut internal_edits = Vec::new();
            let mut remaining_edits = Vec::new();
            for edit in edits {
                if edit.start >= pending.module_range.start && edit.end <= pending.module_range.end
                {
                    internal_edits.push(edit);
                } else {
                    remaining_edits.push(edit);
                }
            }
            remaining_edits.push(
                module_branches_edit(source, file_path, pending, internal_edits)
                    .map_err(ExpansionError::fatal)?,
            );
            edits = remaining_edits;
        }

        apply_edits(source, edits, file_path).map_err(ExpansionError::fatal)
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_module_variant(
        &mut self,
        variant: ModuleFileVariant,
        module: &ast::Module,
        root: &SyntaxNode,
        file_path: &Path,
        child_module_path: &str,
        macro_scope: &MacroScope,
        external_admitted: bool,
        directive: BundleDirective,
    ) -> ExpansionResult<String> {
        let resolved = variant
            .resolution
            .map_err(|error| ExpansionError::recoverable(error.into_message()))?;
        let inline_default_dir = variant.inline_default_dir.ok_or_else(|| {
            ExpansionError::recoverable(format!(
                "failed to determine inline module directory for {child_module_path} in {}",
                file_path.display()
            ))
        })?;
        let canonical =
            canonical_file(&resolved.path, "module file").map_err(ExpansionError::recoverable)?;
        self.check_module_cycle(&canonical, child_module_path)?;
        self.track_source(&canonical, child_module_path, BundledSourceKind::Module)?;

        let child_source =
            read_rust_source(&canonical, false).map_err(ExpansionError::recoverable)?;
        let child_context =
            SourceContext::for_module_file(&resolved.path, resolved.loaded_through_path_attr)
                .map_err(ExpansionError::recoverable)?;
        let child_macro_scope = self.macro_scope_at(module.syntax(), root, macro_scope);
        let child_module_paths_are_stable = inline_default_dir == child_context.default_module_dir;
        let child_include_paths_are_stable = child_context.physical_dir == self.bundle_dir;
        self.active_modules.push(canonical.clone());
        let bundled = self.bundle_rust_source(
            &canonical,
            &child_source,
            &SourceSite {
                logical_file_path: &resolved.path,
                context: &child_context,
                module_path: child_module_path,
                external_admitted: external_admitted || directive == BundleDirective::Bundle,
                macro_scope: &child_macro_scope,
                retention: RetentionSafety {
                    module_paths: child_module_paths_are_stable,
                    include_paths: child_include_paths_are_stable,
                    source_positions: false,
                },
            },
        );
        self.active_modules.pop();
        bundled
    }

    fn expand_include(
        &mut self,
        kind: IncludeKind,
        path: &Path,
        logical_path: &Path,
        site: &IncludeSite<'_>,
    ) -> ExpansionResult<String> {
        self.track_source(path, site.module_path, kind.source_kind())?;
        match kind {
            IncludeKind::Str => {
                let text = fs::read_to_string(path).map_err(|error| {
                    ExpansionError::recoverable(format!(
                        "read include_str file {}: {error}",
                        path.display()
                    ))
                })?;
                Ok(format!("{text:?}"))
            }
            IncludeKind::Bytes => {
                let bytes = fs::read(path).map_err(|error| {
                    ExpansionError::recoverable(format!(
                        "read include_bytes file {}: {error}",
                        path.display()
                    ))
                })?;
                Ok(byte_string_expression(&bytes))
            }
            IncludeKind::Source => {
                if let Some(index) = self
                    .active_includes
                    .iter()
                    .position(|active| active == path)
                {
                    let mut chain = self.active_includes[index..]
                        .iter()
                        .map(|item| display_path(item))
                        .collect::<Vec<_>>();
                    chain.push(display_path(path));
                    return Err(ExpansionError::Atomic(format!(
                        "include cycle detected: {}",
                        chain.join(" -> ")
                    )));
                }
                let source = read_rust_source(path, false).map_err(ExpansionError::recoverable)?;
                self.active_includes.push(path.to_path_buf());
                let result = self.bundle_include_fragment(path, logical_path, &source, site);
                self.active_includes.pop();
                result
            }
        }
    }

    fn bundle_include_fragment(
        &mut self,
        path: &Path,
        logical_path: &Path,
        source: &str,
        site: &IncludeSite<'_>,
    ) -> ExpansionResult<String> {
        let edition = self.options.edition.into();
        let items = SourceFile::parse(source, edition);
        let include_dir = logical_path
            .parent()
            .unwrap_or(&site.surrounding_context.physical_dir)
            .to_path_buf();
        let include_context = SourceContext {
            physical_dir: include_dir.clone(),
            default_module_dir: include_dir,
        };
        if items.errors().is_empty() {
            return self.bundle_syntax(
                path,
                source,
                items.tree().syntax(),
                &SourceSite {
                    logical_file_path: logical_path,
                    context: &include_context,
                    module_path: site.module_path,
                    external_admitted: site.external_admitted,
                    macro_scope: site.macro_scope,
                    retention: RetentionSafety {
                        module_paths: false,
                        include_paths: include_context.physical_dir == self.bundle_dir,
                        source_positions: false,
                    },
                },
            );
        }

        let expression = ast::Expr::parse(source, edition);
        if expression.errors().is_empty() {
            let bundled = self.bundle_syntax(
                path,
                source,
                expression.tree().syntax(),
                &SourceSite {
                    logical_file_path: logical_path,
                    context: &include_context,
                    module_path: site.module_path,
                    external_admitted: site.external_admitted,
                    macro_scope: site.macro_scope,
                    retention: RetentionSafety {
                        module_paths: false,
                        include_paths: include_context.physical_dir == self.bundle_dir,
                        source_positions: false,
                    },
                },
            )?;
            return Ok(format!("({bundled})"));
        }

        Err(ExpansionError::recoverable(format!(
            "parse include file {}: include! input must be valid Rust items or one expression\n{}",
            path.display(),
            format_parse_errors(path, source, &items.errors())
        )))
    }

    fn track_source(
        &mut self,
        path: &Path,
        module_path: &str,
        kind: BundledSourceKind,
    ) -> ExpansionResult<()> {
        if self.source_file_count >= self.options.max_source_files {
            return Err(ExpansionError::fatal(format!(
                "source file limit of {} exceeded while expanding {}",
                self.options.max_source_files,
                path.display()
            )));
        }
        self.source_file_count += 1;
        self.sources.push(BundledSource {
            file_path: display_path(path),
            module_path: module_path.to_owned(),
            kind,
        });
        Ok(())
    }

    fn check_module_cycle(&self, path: &Path, module_path: &str) -> ExpansionResult<()> {
        let Some(index) = self.active_modules.iter().position(|active| active == path) else {
            return Ok(());
        };
        let mut chain = self.active_modules[index..]
            .iter()
            .map(|item| display_path(item))
            .collect::<Vec<_>>();
        chain.push(display_path(path));
        Err(ExpansionError::Atomic(format!(
            "module cycle detected while resolving {module_path}: {}",
            chain.join(" -> ")
        )))
    }

    fn macro_scope_at(
        &self,
        node: &SyntaxNode,
        root: &SyntaxNode,
        inherited: &MacroScope,
    ) -> MacroScope {
        MacroScope::at(node, root, inherited).with_defined_macros(&self.exported_macros)
    }

    fn checkpoint(&self) -> ExpansionCheckpoint {
        ExpansionCheckpoint {
            source_count: self.source_file_count,
            source_list_len: self.sources.len(),
        }
    }

    fn rollback(&mut self, checkpoint: ExpansionCheckpoint) {
        self.source_file_count = checkpoint.source_count;
        self.sources.truncate(checkpoint.source_list_len);
    }
}

struct IncludeSite<'a> {
    surrounding_context: &'a SourceContext,
    module_path: &'a str,
    external_admitted: bool,
    macro_scope: &'a MacroScope,
}

enum ModuleBranch {
    Inlined { condition: CfgExpr, source: String },
    Retained(CfgExpr),
}

impl ModuleBranch {
    fn condition(&self) -> &CfgExpr {
        match self {
            Self::Inlined { condition, .. } | Self::Retained(condition) => condition,
        }
    }
}

struct PendingModuleEdit {
    module_range: ByteRange,
    semicolon_range: ByteRange,
    branches: Vec<ModuleBranch>,
}

fn module_branches_edit(
    source: &str,
    file_path: &Path,
    pending: PendingModuleEdit,
    internal_edits: Vec<Edit>,
) -> Result<Edit, String> {
    let original = &source[pending.module_range.start..pending.module_range.end];
    let indentation = line_indentation(source, pending.module_range.start);
    let separator = format!("\n\n{indentation}");
    let replacement = pending
        .branches
        .into_iter()
        .map(|branch| {
            let condition = branch.condition().clone();
            let mut branch_edits = internal_edits
                .iter()
                .map(|edit| Edit {
                    start: edit.start - pending.module_range.start,
                    end: edit.end - pending.module_range.start,
                    replacement: edit.replacement.clone(),
                })
                .collect::<Vec<_>>();
            if let ModuleBranch::Inlined {
                source: child_source,
                ..
            } = branch
            {
                branch_edits.push(Edit {
                    start: pending.semicolon_range.start - pending.module_range.start,
                    end: pending.semicolon_range.end - pending.module_range.start,
                    replacement: module_replacement(
                        source,
                        pending.semicolon_range.start,
                        &child_source,
                    ),
                });
            }
            let module_source = apply_edits(original, branch_edits, file_path)?;
            if condition.is_true() {
                Ok(module_source)
            } else {
                Ok(format!(
                    "#[cfg({})]\n{indentation}{module_source}",
                    condition.render()
                ))
            }
        })
        .collect::<Result<Vec<_>, String>>()?
        .join(&separator);
    Ok(Edit {
        start: pending.module_range.start,
        end: pending.module_range.end,
        replacement,
    })
}

fn line_indentation(source: &str, offset: usize) -> &str {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let prefix = &source[line_start..offset];
    if prefix.chars().all(char::is_whitespace) {
        prefix
    } else {
        ""
    }
}

fn module_replacement(source: &str, semicolon_offset: usize, child_source: &str) -> String {
    let separator = if source[..semicolon_offset]
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
    {
        ""
    } else {
        " "
    };
    format!("{separator}{{\n{}\n}}", child_source.trim_end_matches('\n'))
}

fn location_macro_replacement(
    kind: LocationMacroKind,
    logical_file_path: &Path,
    source: &str,
    offset: usize,
) -> String {
    match kind {
        LocationMacroKind::File => format!("{:?}", logical_file_path.to_string_lossy()),
        LocationMacroKind::Line => (source[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1)
        .to_string(),
        LocationMacroKind::Column => {
            let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
            (source[line_start..offset].chars().count() + 1).to_string()
        }
    }
}
