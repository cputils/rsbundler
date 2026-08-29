use ra_ap_syntax::{AstNode, NodeOrToken, SyntaxNode, ast};

use super::edit::byte_range;
#[derive(Debug)]
pub(super) struct MacroTranscriber {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) source: String,
    pub(super) definition: SyntaxNode,
    pub(super) has_dependency: bool,
}

#[derive(Clone, Debug)]
pub(super) struct HiddenLocationMacro {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) path: String,
}

pub(super) fn relevant_transcribers(root: &SyntaxNode) -> Vec<MacroTranscriber> {
    let mut transcribers = Vec::new();
    for definition in root.descendants().filter_map(ast::MacroRules::cast) {
        let Some(rules) = definition.token_tree() else {
            continue;
        };
        let mut after_fat_arrow = false;
        let mut saw_equals = false;
        for element in rules.token_trees_and_tokens() {
            match element {
                NodeOrToken::Token(token) => {
                    if token.kind().is_trivia() {
                        continue;
                    }
                    if token.text() == "=" {
                        saw_equals = true;
                    } else if saw_equals && token.text() == ">" {
                        after_fat_arrow = true;
                        saw_equals = false;
                    } else {
                        saw_equals = false;
                        after_fat_arrow = false;
                    }
                }
                NodeOrToken::Node(node) if after_fat_arrow => {
                    after_fat_arrow = false;
                    saw_equals = false;
                    let source = node.syntax().text().to_string();
                    let has_dependency = contains_dependency_macro(node.syntax())
                        || contains_module_declaration(node.syntax());
                    if !has_dependency && hidden_location_macros(node.syntax()).is_empty() {
                        continue;
                    }
                    let range = byte_range(node.syntax().text_range());
                    transcribers.push(MacroTranscriber {
                        start: range.start,
                        end: range.end,
                        source,
                        definition: definition.syntax().clone(),
                        has_dependency,
                    });
                }
                NodeOrToken::Node(_) => {
                    saw_equals = false;
                    after_fat_arrow = false;
                }
            }
        }
    }
    transcribers
}

pub(super) fn hidden_location_macros(root: &SyntaxNode) -> Vec<HiddenLocationMacro> {
    let mut calls = Vec::new();
    collect_hidden_location_macros(root, &mut calls);
    calls
}

fn collect_hidden_location_macros(node: &SyntaxNode, calls: &mut Vec<HiddenLocationMacro>) {
    let elements = node.children_with_tokens().collect::<Vec<_>>();
    for (index, element) in elements.iter().enumerate() {
        let NodeOrToken::Token(bang) = element else {
            continue;
        };
        if bang.text() != "!" {
            continue;
        }
        let Some(name_index) = elements[..index]
            .iter()
            .rposition(|element| !element.kind().is_trivia())
        else {
            continue;
        };
        let NodeOrToken::Token(name) = &elements[name_index] else {
            continue;
        };
        let Some(NodeOrToken::Node(arguments)) = elements[index + 1..]
            .iter()
            .find(|element| !element.kind().is_trivia())
        else {
            continue;
        };
        let argument_text = arguments.text().to_string();
        if argument_text.len() < 2 || !argument_text[1..argument_text.len() - 1].trim().is_empty() {
            continue;
        }
        let Some((path, start)) = location_macro_path(&elements, name_index, name) else {
            continue;
        };
        calls.push(HiddenLocationMacro {
            start,
            end: byte_range(arguments.text_range()).end,
            path,
        });
    }
    for child in node.children() {
        collect_hidden_location_macros(&child, calls);
    }
}

fn location_macro_path(
    elements: &[NodeOrToken<ra_ap_syntax::SyntaxNode, ra_ap_syntax::SyntaxToken>],
    name_index: usize,
    name: &ra_ap_syntax::SyntaxToken,
) -> Option<(String, usize)> {
    let significant = elements[..name_index]
        .iter()
        .filter(|element| !element.kind().is_trivia())
        .collect::<Vec<_>>();
    let name_start = byte_range(name.text_range()).start;
    let Some((_, separator_start)) = path_separator_before(&significant, significant.len()) else {
        if significant
            .last()
            .is_some_and(|element| element.as_token().is_some_and(|token| token.text() == "$"))
        {
            return None;
        }
        return Some((name.text().to_string(), name_start));
    };
    let prefix_index = separator_start.checked_sub(1)?;
    let prefix = significant.get(prefix_index)?.as_token()?;
    if !matches!(prefix.text(), "std" | "core") {
        return None;
    }
    let mut start = byte_range(prefix.text_range()).start;
    let mut path = format!("{}::{}", prefix.text(), name.text());
    if let Some((_, absolute_separator_start)) = path_separator_before(&significant, prefix_index) {
        if significant[..absolute_separator_start]
            .last()
            .and_then(|element| element.as_token())
            .is_some_and(|token| {
                let text = token.text();
                text.starts_with("r#")
                    || text
                        .chars()
                        .next()
                        .is_some_and(|character| character == '_' || character.is_alphabetic())
            })
        {
            return None;
        }
        start = byte_range(significant[absolute_separator_start].text_range()).start;
        path.insert_str(0, "::");
    }
    Some((path, start))
}

fn path_separator_before(
    elements: &[&NodeOrToken<ra_ap_syntax::SyntaxNode, ra_ap_syntax::SyntaxToken>],
    end: usize,
) -> Option<(usize, usize)> {
    let last = end.checked_sub(1)?;
    let last_text = elements[last].as_token()?.text();
    if last_text == "::" {
        return Some((1, last));
    }
    let previous = last.checked_sub(1)?;
    (last_text == ":" && elements[previous].as_token()?.text() == ":").then_some((2, previous))
}

fn contains_dependency_macro(node: &SyntaxNode) -> bool {
    let mut previous = None;
    for token in node
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .filter(|token| !token.kind().is_trivia())
    {
        if token.text() == "!"
            && previous.as_ref().is_some_and(|name: &String| {
                matches!(name.as_str(), "include" | "include_str" | "include_bytes")
            })
        {
            return true;
        }
        previous = Some(token.text().to_string());
    }
    false
}

fn contains_module_declaration(node: &SyntaxNode) -> bool {
    node.descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .any(|token| !token.kind().is_trivia() && token.text() == "mod")
}
