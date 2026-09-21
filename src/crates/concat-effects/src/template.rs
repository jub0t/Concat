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

    /// Every filter the chain names, in order, or why one cannot be told.
    ///
    /// A filtergraph is filters parted by `,` and `;`. Each is a run of
    /// `[labels]`, a name, `@instance` if it has one, and `=` with its
    /// options, then `[labels]` again for its outputs; a `'quoted'` option
    /// holds any character, and `\` escapes the one after it. A name has
    /// to be literal: a slot where a name goes would let a package pick at
    /// render time a filter it never showed at load, which is the one
    /// thing the check in [`crate::filters`] exists to stop.
    pub fn filters(&self) -> Result<Vec<String>, String> {
        #[derive(PartialEq, Clone, Copy)]
        enum At {
            Name,
            Options,
        }
        let mut names = Vec::new();
        let mut name = String::new();
        let mut at = At::Name;
        let mut quoted = false;
        let mut escaped = false;
        let mut labelled = false;
        let finish = |name: &mut String, names: &mut Vec<String>| -> Result<(), String> {
            if name.is_empty() {
                return Err("a filter with no name, from a stray `,` or `;`".to_owned());
            }
            names.push(std::mem::take(name));
            Ok(())
        };
        for part in &self.parts {
            let text = match part {
                Part::Slot(_) if at == At::Name && !labelled => {
                    return Err(if name.is_empty() {
                        "a `{slot}` where a filter's name goes: a name is spelt out".to_owned()
                    } else {
                        format!("a `{{slot}}` inside the filter name `{name}`: a name is spelt out")
                    });
                }
                Part::Slot(_) => continue,
                Part::Literal(text) => text,
            };
            for c in text.chars() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if quoted {
                    quoted = c != '\'';
                    continue;
                }
                if labelled {
                    labelled = c != ']';
                    continue;
                }
                match at {
                    At::Name => match c {
                        c if c.is_whitespace() => {}
                        '[' if name.is_empty() => labelled = true,
                        '[' => {
                            finish(&mut name, &mut names)?;
                            at = At::Options;
                            labelled = true;
                        }
                        '=' | '@' => {
                            finish(&mut name, &mut names)?;
                            at = At::Options;
                        }
                        ',' | ';' => finish(&mut name, &mut names)?,
                        c if c.is_ascii_alphanumeric() || c == '_' => name.push(c),
                        c => return Err(format!("`{c}` is not part of a filter name")),
                    },
                    At::Options => match c {
                        '\'' => quoted = true,
                        '\\' => escaped = true,
                        '[' => labelled = true,
                        ',' | ';' => at = At::Name,
                        _ => {}
                    },
                }
            }
        }
        if quoted {
            return Err("an unclosed `'`".to_owned());
        }
        if labelled {
            return Err("an unclosed `[`".to_owned());
        }
        if at == At::Name {
            finish(&mut name, &mut names)?;
        }
        Ok(names)
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

    fn filters_of(source: &str) -> Result<Vec<String>, String> {
        Template::parse(source).expect("parses").filters()
    }

    #[test]
    fn every_filter_in_a_chain_is_named() {
        assert_eq!(filters_of("negate").unwrap(), ["negate"]);
        assert_eq!(
            filters_of("hue=h={round(hue)},eq=brightness={b}:contrast=1").unwrap(),
            ["hue", "eq"]
        );
        // Labels before and after, an instance name, and a second graph.
        assert_eq!(
            filters_of("split[a][b];[a]gblur=sigma=2[c];[b][c]blend@mix=all_mode=screen").unwrap(),
            ["split", "gblur", "blend"]
        );
        // A comma inside a quoted option is not a part.
        assert_eq!(
            filters_of("curves=all='0/0,0.5/0.6,1/1',lutrgb=r='val*2'").unwrap(),
            ["curves", "lutrgb"]
        );
        // An escaped comma is not a part either, and nor is one in a slot.
        assert_eq!(
            filters_of("geq=lum='p(X\\,Y)':cb={fixed(a, 2)},negate").unwrap(),
            ["geq", "negate"]
        );
        // Whitespace and newlines between filters are nothing.
        assert_eq!(
            filters_of("hue=h=1,\n    negate ,vflip").unwrap(),
            ["hue", "negate", "vflip"]
        );
        // The catalogue's own `{lut}` path is an option, not a name.
        assert_eq!(filters_of("lut3d=file={lut}").unwrap(), ["lut3d"]);
    }

    #[test]
    fn a_filter_name_is_spelt_out_or_the_chain_is_refused() {
        for (chain, needle) in [
            ("{name}=1", "spelt out"),
            ("hue=h=1,{name}", "spelt out"),
            ("hu{e}=h=1", "spelt out"),
            ("hue=h=1,", "no name"),
            (",hue", "no name"),
            ("hue=h=1;;negate", "no name"),
            ("hue=h='1", "unclosed `'`"),
            ("[in", "unclosed `[`"),
            ("hue-rotate=1", "not part"),
            ("$(rm -rf /)", "not part"),
        ] {
            let error = filters_of(chain).expect_err(chain);
            assert!(error.contains(needle), "{chain}: {error}");
        }
    }

    #[test]
    fn malformed_templates_fail_at_parse() {
        assert!(Template::parse("gblur={radius").is_err());
        assert!(Template::parse("gblur=radius}").is_err());
        assert!(Template::parse("gblur={1 +}").is_err());
    }
}
