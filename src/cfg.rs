use std::path::Path;

use ra_ap_syntax::{AstNode, SyntaxNode, ast, ast::HasAttrs};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) enum CfgExpr {
    True,
    False,
    Atom(String),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
}

impl CfgExpr {
    pub(super) fn all(expressions: impl IntoIterator<Item = Self>) -> Self {
        let mut flattened = Vec::new();
        for expression in expressions {
            match expression {
                Self::True => {}
                Self::False => return Self::False,
                Self::All(nested) => flattened.extend(nested),
                expression => flattened.push(expression),
            }
        }
        deduplicate(&mut flattened);
        if contains_complement(&flattened) {
            return Self::False;
        }
        match flattened.len() {
            0 => Self::True,
            1 => flattened.pop().expect("one cfg expression"),
            _ => Self::All(flattened),
        }
    }

    pub(super) fn any(expressions: impl IntoIterator<Item = Self>) -> Self {
        let mut flattened = Vec::new();
        for expression in expressions {
            match expression {
                Self::False => {}
                Self::True => return Self::True,
                Self::Any(nested) => flattened.extend(nested),
                expression => flattened.push(expression),
            }
        }
        deduplicate(&mut flattened);
        if contains_complement(&flattened) {
            return Self::True;
        }
        match flattened.len() {
            0 => Self::False,
            1 => flattened.pop().expect("one cfg expression"),
            _ => Self::Any(flattened),
        }
    }

    pub(super) fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Not(expression) => *expression,
            expression => Self::Not(Box::new(expression)),
        }
    }

    pub(super) fn is_true(&self) -> bool {
        matches!(self, Self::True)
    }

    pub(super) fn is_false(&self) -> bool {
        matches!(self, Self::False)
    }

    pub(super) fn render(&self) -> String {
        match self {
            Self::True => "all()".to_owned(),
            Self::False => "any()".to_owned(),
            Self::Atom(atom) => atom.clone(),
            Self::All(expressions) => render_composite("all", expressions),
            Self::Any(expressions) => render_composite("any", expressions),
            Self::Not(expression) => format!("not({})", expression.render()),
        }
    }
}

fn deduplicate(expressions: &mut Vec<CfgExpr>) {
    let mut index = 0;
    while index < expressions.len() {
        if expressions[..index].contains(&expressions[index]) {
            expressions.remove(index);
        } else {
            index += 1;
        }
    }
}

fn contains_complement(expressions: &[CfgExpr]) -> bool {
    expressions.iter().any(|expression| match expression {
        CfgExpr::Not(inner) => expressions.contains(inner),
        expression => expressions.contains(&CfgExpr::Not(Box::new(expression.clone()))),
    })
}

fn render_composite(operator: &str, expressions: &[CfgExpr]) -> String {
    format!(
        "{operator}({})",
        expressions
            .iter()
            .map(CfgExpr::render)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn cfg_predicate_expression(
    predicate: ast::CfgPredicate,
    file_path: &Path,
) -> Result<CfgExpr, String> {
    match predicate {
        ast::CfgPredicate::CfgAtom(atom) => match atom.key() {
            Some(ast::CfgAtomKey::True) => Ok(CfgExpr::True),
            Some(ast::CfgAtomKey::False) => Ok(CfgExpr::False),
            Some(ast::CfgAtomKey::Ident(_)) => Ok(CfgExpr::Atom(atom.syntax().text().to_string())),
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
            let expressions = composite
                .cfg_predicates()
                .map(|predicate| cfg_predicate_expression(predicate, file_path))
                .collect::<Result<Vec<_>, _>>()?;
            match keyword.text() {
                "all" => Ok(CfgExpr::all(expressions)),
                "any" => Ok(CfgExpr::any(expressions)),
                "not" if expressions.len() == 1 => Ok(expressions
                    .into_iter()
                    .next()
                    .expect("one cfg expression")
                    .not()),
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

pub(super) fn syntax_is_active(node: &SyntaxNode, file_path: &Path) -> Result<bool, String> {
    for ancestor in node.ancestors() {
        let Some(owner) = ast::AnyHasAttrs::cast(ancestor) else {
            continue;
        };
        for attr in owner.attrs() {
            if let Some(meta) = attr.meta()
                && owner_condition(meta, file_path)?.is_false()
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn owner_condition(meta: ast::Meta, file_path: &Path) -> Result<CfgExpr, String> {
    match meta {
        ast::Meta::CfgMeta(meta) => {
            let predicate = meta.cfg_predicate().ok_or_else(|| {
                format!("{}: cfg attribute has no predicate", file_path.display())
            })?;
            cfg_predicate_expression(predicate, file_path)
        }
        ast::Meta::CfgAttrMeta(meta) => {
            let predicate = meta.cfg_predicate().ok_or_else(|| {
                format!(
                    "{}: cfg_attr attribute has no predicate",
                    file_path.display()
                )
            })?;
            let condition = cfg_predicate_expression(predicate, file_path)?;
            let nested = CfgExpr::all(
                meta.metas()
                    .map(|meta| owner_condition(meta, file_path))
                    .collect::<Result<Vec<_>, _>>()?,
            );
            Ok(CfgExpr::any([condition.not(), nested]))
        }
        _ => Ok(CfgExpr::True),
    }
}

#[cfg(test)]
mod tests {
    use super::CfgExpr;

    #[test]
    fn simplifies_boolean_expressions() {
        let atom = CfgExpr::Atom("unix".to_owned());
        assert_eq!(CfgExpr::all([atom.clone(), atom.clone()]), atom.clone());
        assert_eq!(
            CfgExpr::all([atom.clone(), atom.clone().not()]),
            CfgExpr::False
        );
        assert_eq!(
            CfgExpr::any([atom.clone(), atom.clone().not()]),
            CfgExpr::True
        );
        assert_eq!(atom.clone().not().not(), atom);
    }
}
