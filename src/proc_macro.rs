use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ra_ap_paths::{Utf8Path, Utf8PathBuf};
use ra_ap_proc_macro_api::{
    legacy_protocol::msg::{FlatTree, SpanDataIndexMap},
    version::CURRENT_API_VERSION,
};
use ra_ap_proc_macro_srv::{
    EnvSnapshot, ProcMacroClientInterface, ProcMacroKind, ProcMacroSrv, TrackedEnv,
};
use ra_ap_span::{
    Edition as SpanEdition, EditionedFileId, FileId, ROOT_ERASED_FILE_AST_ID, Span, SpanAnchor,
    SyntaxContext, TextRange,
};
use ra_ap_syntax::{AstNode, Edition, SourceFile, ast};
use ra_ap_syntax_bridge::parse_to_token_tree;

use super::ProcMacroDylib;
use super::cfg::syntax_is_active;
use super::edit::{Edit, apply_edits, byte_range};
use super::module_resolver::normalize_external_name;
use super::proc_macro_discovery::discover_proc_macros;

const MAX_EXPANSIONS: usize = 1024;

pub(super) struct ProcMacroExpander<'env> {
    server: ProcMacroSrv<'env>,
    definitions: Vec<Definition>,
    environment: Vec<(String, String)>,
    current_dir: PathBuf,
}

struct Definition {
    crate_names: Vec<String>,
    dylib_path: Utf8PathBuf,
    macro_name: String,
    kind: ProcMacroKind,
}

struct SpanContext<'source> {
    file_path: &'source Path,
    body: &'source str,
    attributes: Option<&'source str>,
}

impl ProcMacroClientInterface for SpanContext<'_> {
    fn file(&mut self, _file_id: FileId) -> String {
        self.file_path.to_string_lossy().into_owned()
    }

    fn source_text(&mut self, span: Span) -> Option<String> {
        let source = self.source(span.anchor.file_id.file_id())?;
        source.get(span_range(span)).map(str::to_owned)
    }

    fn local_file(&mut self, _file_id: FileId) -> Option<String> {
        Some(self.file_path.to_string_lossy().into_owned())
    }

    fn line_column(&mut self, span: Span) -> Option<(u32, u32)> {
        let source = self.source(span.anchor.file_id.file_id())?;
        let offset = usize::from(span.range.start()).min(source.len());
        let prefix = source.get(..offset)?;
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix.rsplit_once('\n').map_or(prefix, |(_, tail)| tail);
        Some((
            line.try_into().ok()?,
            (column.chars().count() + 1).try_into().ok()?,
        ))
    }

    fn byte_range(&mut self, span: Span) -> std::ops::Range<usize> {
        span_range(span)
    }

    fn span_source(&mut self, span: Span) -> Span {
        span
    }

    fn span_parent(&mut self, _span: Span) -> Option<Span> {
        None
    }

    fn span_join(&mut self, first: Span, second: Span) -> Option<Span> {
        first.join(second, |_, _| None)
    }
}

impl<'source> SpanContext<'source> {
    fn source(&self, file_id: FileId) -> Option<&'source str> {
        match file_id.index() {
            1 => Some(self.body),
            2 => self.attributes,
            _ => None,
        }
    }
}

enum Candidate {
    Attribute {
        definition: usize,
        attr_range: std::ops::Range<usize>,
        item_range: std::ops::Range<usize>,
        arguments: String,
    },
    Derive {
        definitions: Vec<usize>,
        attr_range: std::ops::Range<usize>,
        item_range: std::ops::Range<usize>,
        retained: Vec<String>,
    },
    Bang {
        definition: usize,
        call_range: std::ops::Range<usize>,
        body: String,
    },
}

impl Candidate {
    fn start(&self) -> usize {
        match self {
            Self::Attribute { attr_range, .. } | Self::Derive { attr_range, .. } => {
                attr_range.start
            }
            Self::Bang { call_range, .. } => call_range.start,
        }
    }
}

impl<'env> ProcMacroExpander<'env> {
    pub(super) fn new(
        libraries: &[ProcMacroDylib],
        environment: Vec<(String, String)>,
        current_dir: PathBuf,
        snapshot: &'env EnvSnapshot,
        edition: Edition,
    ) -> Result<Self, String> {
        let server = ProcMacroSrv::new(snapshot);
        let mut definitions = Vec::new();
        let explicitly_configured = libraries
            .iter()
            .map(|library| normalize_external_name(&library.crate_name, edition))
            .collect::<Result<HashSet<_>, _>>()?;
        for library in libraries {
            let crate_name = normalize_external_name(&library.crate_name, edition)?;
            let dylib_path = canonical_utf8_dylib(&library.dylib_path)?;
            let macros = server.list_macros(&dylib_path).map_err(|error| {
                format!(
                    "load procedural-macro library {} for crate {:?}: {error}",
                    dylib_path, library.crate_name
                )
            })?;
            if macros.is_empty() {
                return Err(format!(
                    "procedural-macro library {} exports no macros",
                    dylib_path
                ));
            }
            add_definitions(&mut definitions, vec![crate_name], dylib_path, macros)?;
        }
        for discovered in discover_proc_macros(&current_dir, &environment, &explicitly_configured) {
            let crate_names = discovered
                .crate_names
                .iter()
                .map(|name| normalize_external_name(name, edition))
                .collect::<Result<Vec<_>, _>>()?;
            for candidate in discovered.candidates {
                let Ok(dylib_path) = canonical_utf8_dylib(&candidate) else {
                    continue;
                };
                let Ok(macros) = server.list_macros(&dylib_path) else {
                    continue;
                };
                if macros.is_empty() {
                    continue;
                }
                add_definitions(&mut definitions, crate_names.clone(), dylib_path, macros)?;
                break;
            }
        }
        Ok(Self {
            server,
            definitions,
            environment,
            current_dir,
        })
    }

    pub(super) fn expand_source(
        &self,
        file_path: &Path,
        source: &str,
        edition: Edition,
    ) -> Result<String, String> {
        if self.definitions.is_empty() {
            return Ok(source.to_owned());
        }

        let mut expanded = source.to_owned();
        for _ in 0..MAX_EXPANSIONS {
            let parsed = SourceFile::parse(&expanded, edition);
            if !parsed.errors().is_empty() {
                return Ok(expanded);
            }
            let Some(candidate) =
                self.find_candidate(parsed.tree().syntax(), &expanded, file_path)?
            else {
                return Ok(expanded);
            };
            expanded = self.expand_candidate(candidate, &expanded, file_path, edition)?;
        }
        Err(format!(
            "procedural-macro expansion limit ({MAX_EXPANSIONS}) exceeded in {}",
            file_path.display()
        ))
    }

    fn find_candidate(
        &self,
        root: &ra_ap_syntax::SyntaxNode,
        source: &str,
        file_path: &Path,
    ) -> Result<Option<Candidate>, String> {
        let mut candidates = Vec::new();
        for attr in root.descendants().filter_map(ast::Attr::cast) {
            if !attr.kind().is_outer() {
                continue;
            }
            let Some(item) = attr.syntax().parent() else {
                continue;
            };
            if !syntax_is_active(&item, file_path)? {
                continue;
            }
            let Some(meta) = attr.meta() else {
                continue;
            };
            let path = meta
                .path()
                .map(|path| compact_path(&path.syntax().text().to_string()))
                .unwrap_or_default();
            if path == "derive" {
                let ast::Meta::TokenTreeMeta(meta) = meta else {
                    continue;
                };
                let Some(token_tree) = meta.token_tree() else {
                    continue;
                };
                let mut definitions = Vec::new();
                let mut retained = Vec::new();
                for derive in token_tree_inner(token_tree.syntax(), source)
                    .split(',')
                    .map(str::trim)
                    .filter(|derive| !derive.is_empty())
                {
                    match self.resolve(derive, ProcMacroKind::CustomDerive)? {
                        Some(definition) => definitions.push(definition),
                        None => retained.push(derive.to_owned()),
                    }
                }
                if !definitions.is_empty() {
                    candidates.push(Candidate::Derive {
                        definitions,
                        attr_range: syntax_range(attr.syntax()),
                        item_range: syntax_range(&item),
                        retained,
                    });
                }
                continue;
            }
            let Some(definition) = self.resolve(&path, ProcMacroKind::Attr)? else {
                continue;
            };
            let arguments = match meta {
                ast::Meta::TokenTreeMeta(meta) => meta
                    .token_tree()
                    .map(|tree| token_tree_inner(tree.syntax(), source).to_owned())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            candidates.push(Candidate::Attribute {
                definition,
                attr_range: syntax_range(attr.syntax()),
                item_range: syntax_range(&item),
                arguments,
            });
        }

        for call in root.descendants().filter_map(ast::MacroCall::cast) {
            if !syntax_is_active(call.syntax(), file_path)? {
                continue;
            }
            let Some(path) = call.path() else {
                continue;
            };
            let path = compact_path(&path.syntax().text().to_string());
            let Some(definition) = self.resolve(&path, ProcMacroKind::Bang)? else {
                continue;
            };
            let Some(token_tree) = call.token_tree() else {
                continue;
            };
            candidates.push(Candidate::Bang {
                definition,
                call_range: syntax_range(call.syntax()),
                body: token_tree_inner(token_tree.syntax(), source).to_owned(),
            });
        }

        Ok(candidates.into_iter().min_by_key(Candidate::start))
    }

    fn expand_candidate(
        &self,
        candidate: Candidate,
        source: &str,
        file_path: &Path,
        edition: Edition,
    ) -> Result<String, String> {
        match candidate {
            Candidate::Attribute {
                definition,
                attr_range,
                item_range,
                arguments,
            } => {
                let body = remove_range(source, &item_range, &attr_range);
                let replacement =
                    self.expand_macro(definition, &body, Some(&arguments), file_path, edition)?;
                apply_edits(
                    source,
                    vec![Edit {
                        start: item_range.start,
                        end: item_range.end,
                        replacement,
                    }],
                    file_path,
                )
            }
            Candidate::Derive {
                definitions,
                attr_range,
                item_range,
                retained,
            } => {
                let body = remove_range(source, &item_range, &attr_range);
                let mut replacement = if retained.is_empty() {
                    body.clone()
                } else {
                    replace_range(
                        source,
                        &item_range,
                        &attr_range,
                        &format!("#[derive({})]", retained.join(", ")),
                    )
                };
                for definition in definitions {
                    let generated =
                        self.expand_macro(definition, &body, None, file_path, edition)?;
                    if !generated.is_empty() {
                        replacement.push('\n');
                        replacement.push_str(&generated);
                    }
                }
                apply_edits(
                    source,
                    vec![Edit {
                        start: item_range.start,
                        end: item_range.end,
                        replacement,
                    }],
                    file_path,
                )
            }
            Candidate::Bang {
                definition,
                call_range,
                body,
            } => {
                let replacement = self.expand_macro(definition, &body, None, file_path, edition)?;
                apply_edits(
                    source,
                    vec![Edit {
                        start: call_range.start,
                        end: call_range.end,
                        replacement,
                    }],
                    file_path,
                )
            }
        }
    }

    fn expand_macro(
        &self,
        definition: usize,
        body: &str,
        attributes: Option<&str>,
        file_path: &Path,
        edition: Edition,
    ) -> Result<String, String> {
        let definition = &self.definitions[definition];
        let span_edition = span_edition(edition);
        let body_span = macro_span(1, span_edition);
        let attribute_span = macro_span(2, span_edition);
        let body_tokens = parse_to_token_tree(span_edition, body_span.anchor, body_span.ctx, body)
            .ok_or_else(|| {
                format!(
                    "tokenize input for procedural macro {}::{} in {}",
                    definition.crate_names[0],
                    definition.macro_name,
                    file_path.display()
                )
            })?;
        let attribute_tokens = attributes
            .map(|attributes| {
                parse_to_token_tree(
                    span_edition,
                    attribute_span.anchor,
                    attribute_span.ctx,
                    attributes,
                )
                .ok_or_else(|| {
                    format!(
                        "tokenize attributes for procedural macro {}::{} in {}",
                        definition.crate_names[0],
                        definition.macro_name,
                        file_path.display()
                    )
                })
            })
            .transpose()?;

        let mut spans = SpanDataIndexMap::default();
        let body_tokens =
            FlatTree::from_subtree(body_tokens.view(), CURRENT_API_VERSION, &mut spans)
                .to_tokenstream_resolved(CURRENT_API_VERSION, &spans, |left, right| {
                    left.cover(right)
                });
        let attribute_tokens = attribute_tokens.map(|attributes| {
            FlatTree::from_subtree(attributes.view(), CURRENT_API_VERSION, &mut spans)
                .to_tokenstream_resolved(CURRENT_API_VERSION, &spans, |left, right| {
                    left.cover(right)
                })
        });
        let mut tracked_env = TrackedEnv::default();
        let mut span_context = SpanContext {
            file_path,
            body,
            attributes,
        };
        let expanded = self
            .server
            .expand(
                &definition.dylib_path,
                &self.environment,
                Some(&self.current_dir),
                &definition.macro_name,
                body_tokens,
                attribute_tokens,
                body_span,
                body_span,
                body_span,
                &mut tracked_env,
                Some(&mut span_context),
            )
            .map_err(|error| {
                let detail = error
                    .into_string()
                    .unwrap_or_else(|| "unknown expansion error".to_owned());
                format!(
                    "expand procedural macro {}::{} in {}: {detail}",
                    definition.crate_names[0],
                    definition.macro_name,
                    file_path.display()
                )
            })?;
        let expanded =
            FlatTree::from_tokenstream(expanded, CURRENT_API_VERSION, body_span, &mut spans)
                .to_subtree_resolved(CURRENT_API_VERSION, &spans);
        Ok(expanded.to_string())
    }

    fn resolve(&self, path: &str, kind: ProcMacroKind) -> Result<Option<usize>, String> {
        let path = compact_path(path);
        let mut segments = path.trim_start_matches("::").split("::");
        let Some(first) = segments.next().filter(|segment| !segment.is_empty()) else {
            return Ok(None);
        };
        let last = path
            .trim_start_matches("::")
            .rsplit("::")
            .next()
            .unwrap_or(first)
            .trim_start_matches("r#");
        let qualified = path.trim_start_matches("::").contains("::");
        let first = first.trim_start_matches("r#");
        let matches = self
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, definition)| {
                definition.kind == kind
                    && definition.macro_name == last
                    && (!qualified
                        || definition
                            .crate_names
                            .iter()
                            .any(|crate_name| crate_name == first))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [definition] => Ok(Some(*definition)),
            _ => Err(format!(
                "ambiguous unqualified procedural macro {path:?}; use a qualified crate path"
            )),
        }
    }
}

fn add_definitions(
    definitions: &mut Vec<Definition>,
    crate_names: Vec<String>,
    dylib_path: Utf8PathBuf,
    macros: Vec<(String, ProcMacroKind)>,
) -> Result<(), String> {
    for (macro_name, kind) in macros {
        if let Some(definition) = definitions.iter_mut().find(|definition| {
            definition.dylib_path == dylib_path
                && definition.macro_name == macro_name
                && definition.kind == kind
        }) {
            for crate_name in &crate_names {
                if !definition.crate_names.contains(crate_name) {
                    definition.crate_names.push(crate_name.clone());
                }
            }
            continue;
        }
        if let Some(crate_name) = crate_names.iter().find(|crate_name| {
            definitions.iter().any(|definition| {
                definition.crate_names.contains(crate_name)
                    && definition.macro_name == macro_name
                    && definition.kind == kind
            })
        }) {
            return Err(format!(
                "duplicate procedural macro {crate_name}::{macro_name} ({kind:?})"
            ));
        }
        definitions.push(Definition {
            crate_names: crate_names.clone(),
            dylib_path: dylib_path.clone(),
            macro_name,
            kind,
        });
    }
    Ok(())
}

fn canonical_utf8_dylib(path: &Path) -> Result<Utf8PathBuf, String> {
    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "resolve procedural-macro library {}: {error}",
            path.display()
        )
    })?;
    if !canonical.is_file() {
        return Err(format!(
            "procedural-macro library is not a file: {}",
            canonical.display()
        ));
    }
    Utf8Path::from_path(&canonical)
        .map(Utf8Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "procedural-macro library path is not UTF-8: {}",
                canonical.display()
            )
        })
}

fn syntax_range(node: &ra_ap_syntax::SyntaxNode) -> std::ops::Range<usize> {
    let range = byte_range(node.text_range());
    range.start..range.end
}

fn token_tree_inner<'a>(node: &ra_ap_syntax::SyntaxNode, source: &'a str) -> &'a str {
    let range = syntax_range(node);
    if range.end.saturating_sub(range.start) < 2 {
        ""
    } else {
        &source[range.start + 1..range.end - 1]
    }
}

fn remove_range(
    source: &str,
    outer: &std::ops::Range<usize>,
    inner: &std::ops::Range<usize>,
) -> String {
    replace_range(source, outer, inner, "")
}

fn replace_range(
    source: &str,
    outer: &std::ops::Range<usize>,
    inner: &std::ops::Range<usize>,
    replacement: &str,
) -> String {
    format!(
        "{}{}{}",
        &source[outer.start..inner.start],
        replacement,
        &source[inner.end..outer.end]
    )
}

fn compact_path(path: &str) -> String {
    path.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn span_edition(edition: Edition) -> SpanEdition {
    match edition {
        Edition::Edition2015 => SpanEdition::Edition2015,
        Edition::Edition2018 => SpanEdition::Edition2018,
        Edition::Edition2021 => SpanEdition::Edition2021,
        Edition::Edition2024 => SpanEdition::Edition2024,
    }
}

fn macro_span(file_id: u32, edition: SpanEdition) -> Span {
    Span {
        range: TextRange::default(),
        anchor: SpanAnchor {
            file_id: EditionedFileId::new(FileId::from_raw(file_id), edition),
            ast_id: ROOT_ERASED_FILE_AST_ID,
        },
        ctx: SyntaxContext::root(edition),
    }
}

fn span_range(span: Span) -> std::ops::Range<usize> {
    usize::from(span.range.start())..usize::from(span.range.end())
}
