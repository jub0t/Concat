// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The expression language inside a template's `{...}` slots.
//!
//! Small on purpose: numbers, the parameters, arithmetic, comparison, and a
//! fixed set of functions. It is the whole of what a package can compute,
//! which makes it the security boundary as much as a convenience: nothing an
//! expression evaluates to can be anything but a number or a formatted
//! number.
//!
//! Values carry whether they are integers: `round` yields an integer, which
//! prints as `12` rather than `12.0`, and integer arithmetic stays integer
//! except for division. `fixed(x, n)` yields text with exactly `n` decimals,
//! which is how a filtergraph wants most of its numbers.

use std::collections::BTreeMap;
use std::fmt;

/// What an expression evaluates to.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    /// A whole number: the result of `round`, or arithmetic on whole numbers.
    Int(i64),
    /// A real number.
    Float(f64),
    /// Text, from `fixed`. Only ever substituted, never computed with.
    Text(String),
}

impl Value {
    fn as_f64(&self, what: &str) -> Result<f64, EvalError> {
        match self {
            Value::Int(n) => Ok(*n as f64),
            Value::Float(x) => Ok(*x),
            Value::Text(_) => Err(EvalError(format!("{what}: text where a number was needed"))),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Text(s) => f.write_str(s),
        }
    }
}

/// Why an expression could not be parsed or evaluated.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EvalError(pub String);

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EvalError {}

/// A parsed expression.
#[derive(Clone, PartialEq, Debug)]
pub enum Expr {
    /// A literal number.
    Number(f64),
    /// A parameter, a `let` binding, or `index`.
    Name(String),
    /// Negation.
    Neg(Box<Expr>),
    /// A binary operator applied to two operands.
    Binary(Op, Box<Expr>, Box<Expr>),
    /// A function call.
    Call(String, Vec<Expr>),
}

/// The binary operators.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `!=`
    Ne,
}

/// The functions an expression may call, with their arities.
const FUNCTIONS: &[(&str, usize)] = &[
    ("if", 3),
    ("clamp", 3),
    ("min", 2),
    ("max", 2),
    ("abs", 1),
    ("floor", 1),
    ("ceil", 1),
    ("round", 1),
    ("sqrt", 1),
    ("pow", 2),
    ("fixed", 2),
];

// ─── parsing ────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Debug)]
enum Token {
    Number(f64),
    Ident(String),
    Op(Op),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
}

fn tokenize(source: &str) -> Result<Vec<Token>, EvalError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let number = text
                    .parse::<f64>()
                    .map_err(|_| EvalError(format!("bad number `{text}`")))?;
                tokens.push(Token::Number(number));
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                tokens.push(Token::Ident(chars[start..i].iter().collect()));
            }
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            '<' | '>' | '=' | '!' => {
                let next = chars.get(i + 1).copied();
                let (op, width) = match (c, next) {
                    ('<', Some('=')) => (Op::Le, 2),
                    ('>', Some('=')) => (Op::Ge, 2),
                    ('=', Some('=')) => (Op::Eq, 2),
                    ('!', Some('=')) => (Op::Ne, 2),
                    ('<', _) => (Op::Lt, 1),
                    ('>', _) => (Op::Gt, 1),
                    _ => return Err(EvalError(format!("unexpected `{c}`"))),
                };
                tokens.push(Token::Op(op));
                i += width;
            }
            _ => return Err(EvalError(format!("unexpected `{c}`"))),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn expect(&mut self, token: Token) -> Result<(), EvalError> {
        match self.next() {
            Some(found) if found == token => Ok(()),
            Some(found) => Err(EvalError(format!("expected {token:?}, found {found:?}"))),
            None => Err(EvalError(format!("expected {token:?}, found the end"))),
        }
    }

    fn comparison(&mut self) -> Result<Expr, EvalError> {
        let left = self.sum()?;
        if let Some(Token::Op(op)) = self.peek().cloned() {
            self.next();
            let right = self.sum()?;
            return Ok(Expr::Binary(op, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn sum(&mut self) -> Result<Expr, EvalError> {
        let mut left = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => Op::Add,
                Some(Token::Minus) => Op::Sub,
                _ => return Ok(left),
            };
            self.next();
            let right = self.term()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn term(&mut self) -> Result<Expr, EvalError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => Op::Mul,
                Some(Token::Slash) => Op::Div,
                _ => return Ok(left),
            };
            self.next();
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Expr, EvalError> {
        if let Some(Token::Minus) = self.peek() {
            self.next();
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, EvalError> {
        match self.next() {
            Some(Token::Number(n)) => Ok(Expr::Number(n)),
            Some(Token::Ident(name)) => {
                if let Some(Token::LParen) = self.peek() {
                    self.next();
                    let mut args = Vec::new();
                    if let Some(Token::RParen) = self.peek() {
                        self.next();
                    } else {
                        loop {
                            args.push(self.comparison()?);
                            match self.next() {
                                Some(Token::Comma) => continue,
                                Some(Token::RParen) => break,
                                other => {
                                    return Err(EvalError(format!(
                                        "expected `,` or `)` in call to {name}, found {other:?}"
                                    )));
                                }
                            }
                        }
                    }
                    let Some((_, arity)) = FUNCTIONS.iter().find(|(f, _)| *f == name) else {
                        return Err(EvalError(format!("unknown function `{name}`")));
                    };
                    if args.len() != *arity {
                        return Err(EvalError(format!(
                            "{name} takes {arity} argument(s), given {}",
                            args.len()
                        )));
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Name(name))
                }
            }
            Some(Token::LParen) => {
                let inner = self.comparison()?;
                self.expect(Token::RParen)?;
                Ok(inner)
            }
            Some(other) => Err(EvalError(format!("unexpected {other:?}"))),
            None => Err(EvalError("unexpected end of expression".to_owned())),
        }
    }
}

impl Expr {
    /// Parses one expression.
    pub fn parse(source: &str) -> Result<Expr, EvalError> {
        let mut parser = Parser {
            tokens: tokenize(source)?,
            at: 0,
        };
        let expr = parser.comparison()?;
        if parser.at != parser.tokens.len() {
            return Err(EvalError(format!("trailing input in `{source}`")));
        }
        Ok(expr)
    }

    /// Every name the expression reads, so a manifest can be checked for
    /// references to parameters it does not declare.
    pub fn names(&self, into: &mut Vec<String>) {
        match self {
            Expr::Number(_) => {}
            Expr::Name(name) => into.push(name.clone()),
            Expr::Neg(inner) => inner.names(into),
            Expr::Binary(_, left, right) => {
                left.names(into);
                right.names(into);
            }
            Expr::Call(_, args) => {
                for arg in args {
                    arg.names(into);
                }
            }
        }
    }

    /// Evaluates against `env`, the values every name resolves to.
    pub fn eval(&self, env: &BTreeMap<String, Value>) -> Result<Value, EvalError> {
        match self {
            Expr::Number(n) => Ok(Value::Float(*n)),
            Expr::Name(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| EvalError(format!("unknown name `{name}`"))),
            Expr::Neg(inner) => match inner.eval(env)? {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(x) => Ok(Value::Float(-x)),
                Value::Text(_) => Err(EvalError("cannot negate text".to_owned())),
            },
            Expr::Binary(op, left, right) => {
                let left = left.eval(env)?;
                let right = right.eval(env)?;
                binary(*op, &left, &right)
            }
            Expr::Call(name, args) => {
                let values = args
                    .iter()
                    .map(|arg| arg.eval(env))
                    .collect::<Result<Vec<_>, _>>()?;
                call(name, &values)
            }
        }
    }
}

fn binary(op: Op, left: &Value, right: &Value) -> Result<Value, EvalError> {
    if let (Value::Int(a), Value::Int(b)) = (left, right) {
        let (a, b) = (*a, *b);
        return Ok(match op {
            Op::Add => Value::Int(a + b),
            Op::Sub => Value::Int(a - b),
            Op::Mul => Value::Int(a * b),
            Op::Div => Value::Float(a as f64 / b as f64),
            Op::Lt => Value::Int(i64::from(a < b)),
            Op::Gt => Value::Int(i64::from(a > b)),
            Op::Le => Value::Int(i64::from(a <= b)),
            Op::Ge => Value::Int(i64::from(a >= b)),
            Op::Eq => Value::Int(i64::from(a == b)),
            Op::Ne => Value::Int(i64::from(a != b)),
        });
    }
    let a = left.as_f64("operator")?;
    let b = right.as_f64("operator")?;
    Ok(match op {
        Op::Add => Value::Float(a + b),
        Op::Sub => Value::Float(a - b),
        Op::Mul => Value::Float(a * b),
        Op::Div => Value::Float(a / b),
        Op::Lt => Value::Int(i64::from(a < b)),
        Op::Gt => Value::Int(i64::from(a > b)),
        Op::Le => Value::Int(i64::from(a <= b)),
        Op::Ge => Value::Int(i64::from(a >= b)),
        Op::Eq => Value::Int(i64::from(a == b)),
        Op::Ne => Value::Int(i64::from(a != b)),
    })
}

fn truthy(value: &Value) -> Result<bool, EvalError> {
    Ok(value.as_f64("if")? != 0.0)
}

fn call(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    let num = |i: usize| args[i].as_f64(name);
    Ok(match name {
        "if" => {
            if truthy(&args[0])? {
                args[1].clone()
            } else {
                args[2].clone()
            }
        }
        "clamp" => match (&args[0], &args[1], &args[2]) {
            (Value::Int(x), Value::Int(lo), Value::Int(hi)) => Value::Int((*x).clamp(*lo, *hi)),
            _ => Value::Float(num(0)?.clamp(num(1)?, num(2)?)),
        },
        "min" => match (&args[0], &args[1]) {
            (Value::Int(a), Value::Int(b)) => Value::Int((*a).min(*b)),
            _ => Value::Float(num(0)?.min(num(1)?)),
        },
        "max" => match (&args[0], &args[1]) {
            (Value::Int(a), Value::Int(b)) => Value::Int((*a).max(*b)),
            _ => Value::Float(num(0)?.max(num(1)?)),
        },
        "abs" => match &args[0] {
            Value::Int(n) => Value::Int(n.abs()),
            _ => Value::Float(num(0)?.abs()),
        },
        "floor" => Value::Int(num(0)?.floor() as i64),
        "ceil" => Value::Int(num(0)?.ceil() as i64),
        // Nearest integer, halves away from zero.
        "round" => Value::Int(num(0)?.round() as i64),
        "sqrt" => Value::Float(num(0)?.sqrt()),
        "pow" => Value::Float(num(0)?.powf(num(1)?)),
        "fixed" => {
            let digits = num(1)?;
            if digits < 0.0 || digits.fract() != 0.0 || digits > 20.0 {
                return Err(EvalError(format!("fixed: {digits} is not a digit count")));
            }
            Value::Text(fixed(num(0)?, digits as usize))
        }
        _ => return Err(EvalError(format!("unknown function `{name}`"))),
    })
}

/// `value` with exactly `digits` decimals. Negative zero prints unsigned, so
/// a slider at 0 never emits `-0.00`.
pub fn fixed(value: f64, digits: usize) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value:.digits$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(source: &str, vars: &[(&str, Value)]) -> Value {
        let env: BTreeMap<String, Value> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        Expr::parse(source)
            .expect("parses")
            .eval(&env)
            .expect("evaluates")
    }

    #[test]
    fn arithmetic_keeps_precedence() {
        assert_eq!(eval("1 + 2 * 3", &[]), Value::Float(7.0));
        assert_eq!(eval("(1 + 2) * 3", &[]), Value::Float(9.0));
        assert_eq!(eval("-2 * -3", &[]), Value::Float(6.0));
        assert_eq!(eval("10 / 4", &[]), Value::Float(2.5));
    }

    #[test]
    fn round_yields_integers_and_integer_arithmetic_stays_integer() {
        assert_eq!(eval("round(12.4)", &[]), Value::Int(12));
        assert_eq!(eval("round(2.5)", &[]), Value::Int(3));
        // A literal is a real number, so mixing one in yields a real.
        assert_eq!(eval("round(12) * 2", &[]), Value::Float(24.0));
        assert_eq!(eval("round(12) * round(2)", &[]), Value::Int(24));
        assert_eq!(eval("round(12) / 2", &[]), Value::Float(6.0));
        assert_eq!(eval("round(13 * 1.3)", &[]), Value::Int(17));
        assert_eq!(eval("round(12.4)", &[]).to_string(), "12");
    }

    #[test]
    fn fixed_formats_like_the_chains_always_have() {
        assert_eq!(eval("fixed(0.5, 2)", &[]).to_string(), "0.50");
        assert_eq!(eval("fixed(-0.0, 3)", &[]).to_string(), "0.000");
        assert_eq!(
            eval("fixed(48000 * pow(2, 12 / 12), 6)", &[]).to_string(),
            "96000.000000"
        );
        assert_eq!(eval("fixed(-0.55 * 0.05, 3)", &[]).to_string(), "-0.028");
    }

    #[test]
    fn conditionals_and_comparisons() {
        let vars = [("pitch", Value::Float(0.0)), ("s", Value::Float(0.65))];
        assert_eq!(
            eval("if(pitch > 0, pitch, 1.2 + 3.8 * s)", &vars),
            Value::Float(1.2 + 3.8 * 0.65)
        );
        assert_eq!(eval("if(3 >= 3, 1, 2)", &[]), Value::Float(1.0));
        assert_eq!(eval("clamp(150, 0, 100) / 100", &[]), Value::Float(1.0));
        assert_eq!(eval("max(round(1.6), 2)", &[]), Value::Float(2.0));
        assert_eq!(eval("max(round(1.6), round(2))", &[]), Value::Int(2));
    }

    #[test]
    fn names_are_reported_and_unknown_ones_fail() {
        let expr = Expr::parse("a + max(b, 2) * index").expect("parses");
        let mut names = Vec::new();
        expr.names(&mut names);
        assert_eq!(names, ["a", "b", "index"]);
        assert!(expr.eval(&BTreeMap::new()).is_err());
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(Expr::parse("1 +").is_err());
        assert!(Expr::parse("foo(1)").is_err());
        assert!(Expr::parse("round(1, 2)").is_err());
        assert!(Expr::parse("1 & 2").is_err());
        assert!(Expr::parse("(1").is_err());
        assert!(Expr::parse("1 2").is_err());
    }
}
