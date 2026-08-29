use std::path::Path;

use ra_ap_syntax::{AstNode, AstToken, NodeOrToken, SyntaxNode, ast};

use super::edit::byte_range;
use super::source::source_position;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum BundleDirective {
    #[default]
    Auto,
    Bundle,
    NoBundle,
}

pub(super) fn bundle_directive(
    node: &SyntaxNode,
    root: &SyntaxNode,
    source: &str,
    file_path: &Path,
) -> Result<BundleDirective, String> {
    let range = byte_range(node.text_range());
    let meaningful_start = node
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .find(|token| !token.kind().is_trivia())
        .map(|token| byte_range(token.text_range()).start)
        .unwrap_or(range.start);
    let node_start_line = line_start(source, meaningful_start);
    let node_end_line = line_start(source, range.end.saturating_sub(1));
    let previous_line = (node_start_line > 0).then(|| {
        let end = node_start_line - 1;
        (line_start(source, end), end)
    });
    let mut has_bundle = false;
    let mut has_no_bundle = false;

    for token in root
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
    {
        let Some(comment) = ast::Comment::cast(token) else {
            continue;
        };
        if comment.kind().doc.is_some() {
            continue;
        }
        let comment_range = byte_range(comment.syntax().text_range());
        let same_line = comment_range.start >= range.end
            && line_start(source, comment_range.start) == node_end_line
            && is_nearest_dependency_before_comment(node, root, source, comment_range.start);
        let standalone_previous = previous_line.is_some_and(|(start, end)| {
            comment_range.start >= start
                && comment_range.end <= end
                && source[start..end].trim() == comment.text()
                && is_first_dependency_on_line(node, root, source, meaningful_start)
        });
        if !same_line && !standalone_previous {
            continue;
        }
        let (bundle, no_bundle) = comment_directives(comment.text());
        has_bundle |= bundle;
        has_no_bundle |= no_bundle;
    }

    match (has_bundle, has_no_bundle) {
        (true, true) => Err(format!(
            "conflicting bundle and no-bundle directives at {}",
            source_position(file_path, source, meaningful_start)
        )),
        (true, false) => Ok(BundleDirective::Bundle),
        (false, true) => Ok(BundleDirective::NoBundle),
        (false, false) => Ok(BundleDirective::Auto),
    }
}

fn is_nearest_dependency_before_comment(
    node: &SyntaxNode,
    root: &SyntaxNode,
    source: &str,
    comment_start: usize,
) -> bool {
    dependency_nodes(root)
        .into_iter()
        .filter(|candidate| {
            let end = dependency_end(candidate);
            end <= comment_start
                && line_start(source, end.saturating_sub(1)) == line_start(source, comment_start)
        })
        .max_by_key(dependency_end)
        .is_some_and(|candidate| candidate == *node)
}

fn is_first_dependency_on_line(
    node: &SyntaxNode,
    root: &SyntaxNode,
    source: &str,
    meaningful_start: usize,
) -> bool {
    let line = line_start(source, meaningful_start);
    dependency_nodes(root)
        .into_iter()
        .filter(|candidate| line_start(source, dependency_start(candidate)) == line)
        .min_by_key(dependency_start)
        .is_some_and(|candidate| candidate == *node)
}

fn dependency_start(node: &SyntaxNode) -> usize {
    node.descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .find(|token| !token.kind().is_trivia())
        .map_or_else(
            || byte_range(node.text_range()).start,
            |token| byte_range(token.text_range()).start,
        )
}

fn dependency_end(node: &SyntaxNode) -> usize {
    node.descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .filter(|token| !token.kind().is_trivia())
        .last()
        .map_or_else(
            || byte_range(node.text_range()).end,
            |token| byte_range(token.text_range()).end,
        )
}

fn dependency_nodes(root: &SyntaxNode) -> Vec<SyntaxNode> {
    root.descendants()
        .filter(|candidate| {
            ast::Module::cast(candidate.clone())
                .is_some_and(|module| module.semicolon_token().is_some())
                || ast::MacroCall::cast(candidate.clone()).is_some()
        })
        .collect()
}

fn comment_directives(comment: &str) -> (bool, bool) {
    let body = comment
        .strip_prefix("//")
        .or_else(|| comment.strip_prefix("/*"))
        .unwrap_or(comment);
    let body = body.strip_suffix("*/").unwrap_or(body);
    let mut bundle = false;
    let mut no_bundle = false;
    for token in body.split(|character: char| {
        !(character.is_alphanumeric() || character == '-' || character == '_')
    }) {
        bundle |= token == "bundle";
        no_bundle |= token == "no-bundle";
    }
    (bundle, no_bundle)
}

fn line_start(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .rfind('\n')
        .map_or(0, |index| index + 1)
}
