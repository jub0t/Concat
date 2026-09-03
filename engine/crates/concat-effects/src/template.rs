// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Chain templates: literal text with `{expression}` slots.
//!
//! `{{` and `}}` are literal braces. Everything between a single `{` and
//! its `}` is an expression (see [`crate::expr`]) whose value is substituted
//! as text. The output is an FFmpeg filter fragment, so the literal parts
//! are FFmpeg syntax and the slots are the numbers a parameter decides.

use std::collections::BTreeMap;

use crate::expr::{EvalError, Expr, Value};

/// A parsed template.
#[derive(Clone, PartialEq, Debug)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Clone, PartialEq, Debug)]
enum Part {
    Literal(String),
    Slot(Expr),
}

impl Template {
    /// Parses `source`. Every slot is parsed now, so a malformed expression
    /// fails at load rather than at render.
    pub fn parse(source: &str) -> Result<Template, EvalError> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut rest = source;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix("{{") {
                literal.push('{');
                rest = after;
            } else if let Some(after) = rest.strip_prefix("}}") {
                literal.push('}');
                rest = after;
            } else if let Some(after) = rest.strip_prefix('{') {
                let Some(end) = after.find('}') else {
                    return Err(EvalError(format!("unclosed `{{` in `{source}`")));
                };
                if !literal.is_empty() {
                    parts.push(Part::Literal(std::mem::take(&mut literal)));
                }
                parts.push(Part::Slot(Expr::parse(&after[..end])?));
                rest = &after[end + 1..];
            } else if rest.starts_with('}') {
                return Err(EvalError(format!("stray `}}` in `{source}`")));
            } else {
                let next = rest.find(['{', '}']).unwrap_or(rest.len());
                literal.push_str(&rest[..next]);
                rest = &rest[next..];
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }
        Ok(Template { parts })
    }

    /// Every name any slot reads.
    pub fn names(&self, into: &mut Vec<String>) {
        for part in &self.parts {
            if let Part::Slot(expr) = part {
                expr.names(into);
            }
        }
    }

    /// The template with every slot evaluated against `env`.
    pub fn render(&self, env: &BTreeMap<String, Value>) -> Result<String, EvalError> {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(text) => out.push_str(text),
                Part::Slot(expr) => out.push_str(&expr.eval(env)?.to_string()),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, Value)]) -> BTreeMap<String, Value> {
        vars.iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn slots_substitute_and_braces_escape() {
        let template =
            Template::parse("gblur=sigma={fixed(radius, 1)}:x={{literal}}").expect("parses");
        let out = template
            .render(&env(&[("radius", Value::Float(10.0))]))
            .expect("renders");
        assert_eq!(out, "gblur=sigma=10.0:x={literal}");
    }

    #[test]
    fn a_slot_may_be_used_twice() {
        let template = Template::parse("trunc(val/{size})*{size}").expect("parses");
        let out = template
            .render(&env(&[("size", Value::Int(64))]))
            .expect("renders");
        assert_eq!(out, "trunc(val/64)*64");
    }

    #[test]
    fn malformed_templates_fail_at_parse() {
        assert!(Template::parse("gblur={radius").is_err());
        assert!(Template::parse("gblur=radius}").is_err());
        assert!(Template::parse("gblur={1 +}").is_err());
    }
}
