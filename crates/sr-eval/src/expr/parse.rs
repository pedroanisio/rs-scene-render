//! Lexer and parser for the scene-render expression language: a pure,
//! side-effect-free subset of ECMAScript.
//!
//! Supported: number, string, boolean, `null`/`undefined` and array
//! literals; `let`/`const`/`var` declarations; assignment and compound
//! assignment to declared variables; `if`/`else`; blocks; `return`; unary
//! `! - +`; binary `** * / % + - < <= > >= == != === !== && || ??`; the
//! conditional operator; calls, member access and indexing. Loops, function
//! definitions, objects literals and `new` are rejected, so an expression
//! runs each of its operations at most once; nesting is limited here
//! ([`MAX_DEPTH`], [`MAX_HEIGHT`]) and the size of the values it builds in
//! the VM.

use std::fmt;
use std::sync::Arc;

/// A syntax error with its byte offset in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// Message.
    pub message: String,
    /// Byte offset.
    pub offset: usize,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at offset {})", self.message, self.offset)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(Arc<str>),
    Ident(String),
    Punct(&'static str),
    Eof,
}

const PUNCTS: [&str; 39] = [
    "===", "!==", "**=", "...", "**", "==", "!=", "<=", ">=", "&&", "||", "??", "+=", "-=", "*=", "/=", "%=", "=>",
    "?.", "+", "-", "*", "/", "%", "<", ">", "!", "=", "?", ":", "(", ")", "[", "]", "{", "}", ",", ";", ".",
];

struct Lexer<'s> {
    src: &'s str,
    pos: usize,
}

impl<'s> Lexer<'s> {
    fn err(&self, msg: impl Into<String>, at: usize) -> SyntaxError {
        SyntaxError { message: msg.into(), offset: at }
    }

    fn skip_trivia(&mut self) -> Result<(), SyntaxError> {
        let b = self.src.as_bytes();
        loop {
            while self.pos < b.len() && (b[self.pos] as char).is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.src[self.pos..].starts_with("//") {
                while self.pos < b.len() && b[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if self.src[self.pos..].starts_with("/*") {
                let start = self.pos;
                match self.src[self.pos + 2..].find("*/") {
                    Some(e) => self.pos += e + 4,
                    None => return Err(self.err("unterminated comment", start)),
                }
            } else {
                return Ok(());
            }
        }
    }

    fn next(&mut self) -> Result<(Tok, usize), SyntaxError> {
        self.skip_trivia()?;
        let start = self.pos;
        let rest = &self.src[self.pos..];
        let Some(c) = rest.chars().next() else { return Ok((Tok::Eof, start)) };
        if c.is_ascii_digit() || (c == '.' && rest[1..].starts_with(|d: char| d.is_ascii_digit())) {
            let b = rest.as_bytes();
            let mut i = 0;
            if rest.starts_with("0x") || rest.starts_with("0X") {
                i = 2;
                while i < b.len() && (b[i] as char).is_ascii_hexdigit() {
                    i += 1;
                }
                let v =
                    i64::from_str_radix(&rest[2..i], 16).map_err(|_| self.err("invalid hexadecimal literal", start))?;
                self.pos += i;
                return Ok((Tok::Num(v as f64), start));
            }
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let v: f64 = rest[..i].parse().map_err(|_| self.err(format!("invalid number {:?}", &rest[..i]), start))?;
            self.pos += i;
            return Ok((Tok::Num(v), start));
        }
        if c == '"' || c == '\'' {
            let mut out = String::new();
            let mut it = rest.char_indices().skip(1);
            while let Some((i, ch)) = it.next() {
                if ch == c {
                    self.pos += i + 1;
                    return Ok((Tok::Str(out.into()), start));
                }
                if ch == '\n' {
                    break;
                }
                if ch == '\\' {
                    let Some((_, e)) = it.next() else { break };
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        other => other,
                    });
                } else {
                    out.push(ch);
                }
            }
            return Err(self.err("unterminated string", start));
        }
        if c == '`' {
            return Err(self.err("template literals are not supported; use + to concatenate", start));
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let len = rest.find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$')).unwrap_or(rest.len());
            self.pos += len;
            return Ok((Tok::Ident(rest[..len].to_string()), start));
        }
        for p in PUNCTS {
            if rest.starts_with(p) {
                self.pos += p.len();
                return Ok((Tok::Punct(p), start));
            }
        }
        Err(self.err(format!("unexpected character {c:?}"), start))
    }
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
}

/// Short-circuit operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Logic {
    And,
    Or,
    Nullish,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
}

/// Expression nodes. `usize` fields are source offsets for diagnostics.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Expr {
    Num(f64),
    Str(Arc<str>),
    Bool(bool),
    Undefined,
    Array(Vec<Expr>),
    Ident(String, usize),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Logical(Logic, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>, usize),
    Member(Box<Expr>, String, usize),
    Index(Box<Expr>, Box<Expr>),
}

/// Statements.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Stmt {
    Let(String, Expr, usize),
    Assign(String, Option<BinOp>, Expr, usize),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    Block(Vec<Stmt>),
    Return(Option<Expr>),
    Expr(Expr),
}

/// Deepest nesting of statements, parentheses, brackets, unary and conditional operators: what
/// the parser and the compiler recurse over (a debug build spends about 12 KiB of stack a level).
pub const MAX_DEPTH: usize = 64;
/// Tallest expression tree: a chain of `n` binary operators, which nests nothing, is `n + 1` tall.
pub const MAX_HEIGHT: usize = 1000;

struct Parser<'s> {
    lex: Lexer<'s>,
    tok: Tok,
    at: usize,
    depth: usize,
    /// Height of the expression tree parsed last.
    height: usize,
}

impl<'s> Parser<'s> {
    fn bump(&mut self) -> Result<Tok, SyntaxError> {
        let (t, at) = self.lex.next()?;
        self.at = at;
        Ok(std::mem::replace(&mut self.tok, t))
    }

    fn err(&self, msg: impl Into<String>) -> SyntaxError {
        SyntaxError { message: msg.into(), offset: self.at }
    }

    fn is(&self, p: &str) -> bool {
        matches!(&self.tok, Tok::Punct(q) if *q == p)
    }

    fn is_kw(&self, k: &str) -> bool {
        matches!(&self.tok, Tok::Ident(i) if i == k)
    }

    fn expect(&mut self, p: &str) -> Result<(), SyntaxError> {
        if self.is(p) {
            self.bump()?;
            Ok(())
        } else {
            Err(self.err(format!("expected '{p}', found {}", self.describe())))
        }
    }

    fn describe(&self) -> String {
        match &self.tok {
            Tok::Num(n) => format!("number {n}"),
            Tok::Str(s) => format!("string {s:?}"),
            Tok::Ident(i) => format!("'{i}'"),
            Tok::Punct(p) => format!("'{p}'"),
            Tok::Eof => "end of expression".into(),
        }
    }

    fn skip_semis(&mut self) -> Result<(), SyntaxError> {
        while self.is(";") {
            self.bump()?;
        }
        Ok(())
    }

    fn too_deep(&self) -> SyntaxError {
        self.err("expression is nested too deeply")
    }

    fn enter(&mut self) -> Result<(), SyntaxError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        Ok(())
    }

    /// Records the height of a node built over subtrees of height `below`.
    fn grow(&mut self, below: usize) -> Result<usize, SyntaxError> {
        self.height = below + 1;
        if self.height > MAX_HEIGHT {
            return Err(self.too_deep());
        }
        Ok(self.height)
    }

    fn statement(&mut self) -> Result<Stmt, SyntaxError> {
        self.enter()?;
        let s = self.statement_body()?;
        self.depth -= 1;
        Ok(s)
    }

    fn statement_body(&mut self) -> Result<Stmt, SyntaxError> {
        if let Tok::Ident(kw) = &self.tok {
            match kw.as_str() {
                "let" | "const" | "var" => {
                    self.bump()?;
                    let mut decls = Vec::new();
                    loop {
                        let at = self.at;
                        let Tok::Ident(name) = self.bump()? else { return Err(self.err("expected a variable name")) };
                        check_name(&name, at)?;
                        self.expect("=")?;
                        let e = self.expr(0)?;
                        decls.push(Stmt::Let(name, e, at));
                        if self.is(",") {
                            self.bump()?;
                        } else {
                            break;
                        }
                    }
                    self.skip_semis()?;
                    return Ok(if decls.len() == 1 { decls.pop().unwrap() } else { Stmt::Block(decls) });
                }
                "if" => {
                    self.bump()?;
                    self.expect("(")?;
                    let c = self.expr(0)?;
                    self.expect(")")?;
                    let then = Box::new(self.statement()?);
                    let els = if self.is_kw("else") {
                        self.bump()?;
                        Some(Box::new(self.statement()?))
                    } else {
                        None
                    };
                    return Ok(Stmt::If(c, then, els));
                }
                "return" => {
                    self.bump()?;
                    let e =
                        if self.is(";") || self.is("}") || self.tok == Tok::Eof { None } else { Some(self.expr(0)?) };
                    self.skip_semis()?;
                    return Ok(Stmt::Return(e));
                }
                "for" | "while" | "do" => {
                    return Err(self.err("loops are not supported; expressions must have bounded cost"))
                }
                "function" | "class" | "new" => return Err(self.err(format!("'{kw}' is not supported in expressions"))),
                "else" => return Err(self.err("'else' without 'if'")),
                _ => {}
            }
        }
        if self.is("{") {
            self.bump()?;
            let mut body = Vec::new();
            while !self.is("}") {
                if self.tok == Tok::Eof {
                    return Err(self.err("missing '}'"));
                }
                body.push(self.statement()?);
            }
            self.bump()?;
            self.skip_semis()?;
            return Ok(Stmt::Block(body));
        }
        let at = self.at;
        let e = self.expr(0)?;
        let assign = match &self.tok {
            Tok::Punct("=") => Some(None),
            Tok::Punct("+=") => Some(Some(BinOp::Add)),
            Tok::Punct("-=") => Some(Some(BinOp::Sub)),
            Tok::Punct("*=") => Some(Some(BinOp::Mul)),
            Tok::Punct("/=") => Some(Some(BinOp::Div)),
            Tok::Punct("%=") => Some(Some(BinOp::Rem)),
            Tok::Punct("**=") => Some(Some(BinOp::Pow)),
            _ => None,
        };
        if let Some(op) = assign {
            let Expr::Ident(name, _) = e else {
                return Err(SyntaxError { message: "only variables can be assigned".into(), offset: at });
            };
            self.bump()?;
            let v = self.expr(0)?;
            self.skip_semis()?;
            return Ok(Stmt::Assign(name, op, v, at));
        }
        self.skip_semis()?;
        Ok(Stmt::Expr(e))
    }

    fn binding(&self) -> Option<(u8, u8, &'static str)> {
        // (left binding power, right binding power, operator)
        let Tok::Punct(p) = &self.tok else { return None };
        let bp = match *p {
            "??" => (3, 4),
            "||" => (5, 6),
            "&&" => (7, 8),
            "==" | "!=" | "===" | "!==" => (9, 10),
            "<" | "<=" | ">" | ">=" => (11, 12),
            "+" | "-" => (13, 14),
            "*" | "/" | "%" => (15, 16),
            "**" => (18, 17),
            _ => return None,
        };
        Some((bp.0, bp.1, p))
    }

    fn expr(&mut self, min_bp: u8) -> Result<Expr, SyntaxError> {
        self.enter()?;
        let mut lhs = self.unary()?;
        let mut h = self.height;
        loop {
            if self.is("?") && min_bp <= 2 {
                self.bump()?;
                let a = self.expr(0)?;
                h = h.max(self.height);
                self.expect(":")?;
                let b = self.expr(2)?;
                h = self.grow(h.max(self.height))?;
                lhs = Expr::Cond(Box::new(lhs), Box::new(a), Box::new(b));
                continue;
            }
            let Some((l, r, op)) = self.binding() else { break };
            if l < min_bp {
                break;
            }
            self.bump()?;
            let rhs = self.expr(r)?;
            h = self.grow(h.max(self.height))?;
            lhs = match op {
                "&&" => Expr::Logical(Logic::And, Box::new(lhs), Box::new(rhs)),
                "||" => Expr::Logical(Logic::Or, Box::new(lhs), Box::new(rhs)),
                "??" => Expr::Logical(Logic::Nullish, Box::new(lhs), Box::new(rhs)),
                _ => {
                    let b = match op {
                        "+" => BinOp::Add,
                        "-" => BinOp::Sub,
                        "*" => BinOp::Mul,
                        "/" => BinOp::Div,
                        "%" => BinOp::Rem,
                        "**" => BinOp::Pow,
                        "<" => BinOp::Lt,
                        "<=" => BinOp::Le,
                        ">" => BinOp::Gt,
                        ">=" => BinOp::Ge,
                        "==" => BinOp::Eq,
                        "!=" => BinOp::Ne,
                        "===" => BinOp::StrictEq,
                        _ => BinOp::StrictNe,
                    };
                    Expr::Binary(b, Box::new(lhs), Box::new(rhs))
                }
            };
        }
        self.height = h;
        self.depth -= 1;
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, SyntaxError> {
        let op = match &self.tok {
            Tok::Punct("-") => Some(UnOp::Neg),
            Tok::Punct("+") => Some(UnOp::Plus),
            Tok::Punct("!") => Some(UnOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            self.bump()?;
            self.enter()?;
            let e = self.unary()?;
            self.depth -= 1;
            self.grow(self.height)?;
            if self.is("**") {
                return Err(self.err("wrap the unary expression in parentheses before '**'"));
            }
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, SyntaxError> {
        let mut e = self.primary()?;
        let mut h = self.height;
        loop {
            if self.is("(") {
                let at = self.at;
                self.bump()?;
                let mut args = Vec::new();
                while !self.is(")") {
                    args.push(self.expr(0)?);
                    h = h.max(self.height);
                    if !self.is(")") {
                        self.expect(",")?;
                    }
                }
                self.bump()?;
                e = Expr::Call(Box::new(e), args, at);
            } else if self.is(".") || self.is("?.") {
                self.bump()?;
                let at = self.at;
                let Tok::Ident(name) = self.bump()? else { return Err(self.err("expected a property name after '.'")) };
                e = Expr::Member(Box::new(e), name, at);
            } else if self.is("[") {
                self.bump()?;
                let i = self.expr(0)?;
                h = h.max(self.height);
                self.expect("]")?;
                e = Expr::Index(Box::new(e), Box::new(i));
            } else {
                break;
            }
            h = self.grow(h)?;
        }
        self.height = h;
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, SyntaxError> {
        let at = self.at;
        self.height = 1;
        match self.bump()? {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Ident(i) => match i.as_str() {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "null" | "undefined" => Ok(Expr::Undefined),
                "NaN" => Ok(Expr::Num(f64::NAN)),
                "Infinity" => Ok(Expr::Num(f64::INFINITY)),
                "function" | "new" | "class" | "this" | "for" | "while" | "let" | "const" | "var" | "if" | "return" => {
                    Err(SyntaxError { message: format!("'{i}' cannot appear here"), offset: at })
                }
                _ => Ok(Expr::Ident(i, at)),
            },
            Tok::Punct("(") => {
                let e = self.expr(0)?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Punct("[") => {
                let mut items = Vec::new();
                let mut h = 0;
                while !self.is("]") {
                    items.push(self.expr(0)?);
                    h = h.max(self.height);
                    if !self.is("]") {
                        self.expect(",")?;
                    }
                }
                self.bump()?;
                self.grow(h)?;
                Ok(Expr::Array(items))
            }
            Tok::Punct("{") => Err(SyntaxError { message: "object literals are not supported".into(), offset: at }),
            Tok::Punct("=>") => Err(SyntaxError { message: "arrow functions are not supported".into(), offset: at }),
            Tok::Eof => Err(SyntaxError { message: "unexpected end of expression".into(), offset: at }),
            t => {
                self.tok = t;
                Err(SyntaxError { message: format!("unexpected {}", self.describe()), offset: at })
            }
        }
    }
}

fn check_name(name: &str, at: usize) -> Result<(), SyntaxError> {
    const RESERVED: [&str; 16] = [
        "time",
        "frame",
        "value",
        "index",
        "count",
        "seed",
        "textIndex",
        "textTotal",
        "Math",
        "true",
        "false",
        "null",
        "undefined",
        "NaN",
        "Infinity",
        "duration",
    ];
    if RESERVED.contains(&name) {
        return Err(SyntaxError { message: format!("'{name}' is a built-in and cannot be redeclared"), offset: at });
    }
    Ok(())
}

/// Parses a program: statements whose result is the value of `return` or of
/// the last expression statement executed.
pub fn parse(src: &str) -> Result<Vec<Stmt>, SyntaxError> {
    let mut p = Parser { lex: Lexer { src, pos: 0 }, tok: Tok::Eof, at: 0, depth: 0, height: 0 };
    p.bump()?;
    let mut out = Vec::new();
    while p.tok != Tok::Eof {
        out.push(p.statement()?);
    }
    if out.is_empty() {
        return Err(SyntaxError { message: "empty expression".into(), offset: 0 });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_statements() {
        let p = parse("let a = 1 + 2 * 3 ** 2 ** 0.5; if (a > 3 && !false) { a * 2 } else a").unwrap();
        assert_eq!(p.len(), 2);
        let Stmt::Let(_, Expr::Binary(BinOp::Add, _, rhs), _) = &p[0] else { panic!("{p:?}") };
        let Expr::Binary(BinOp::Mul, _, pow) = &**rhs else { panic!() };
        assert!(matches!(&**pow, Expr::Binary(BinOp::Pow, _, r) if matches!(&**r, Expr::Binary(BinOp::Pow, _, _))));
        assert!(parse("Math.sin(time * 2) * [1, 2][0] + value.length").is_ok());
        assert!(parse("x = 3; x += 1").is_ok());
        assert!(parse("a ? b : c ? d : e").is_ok());
        assert!(parse("// comment\n1 /* block */ + 2").is_ok());
        assert!(parse("0x1F + .5 + 1e-3").is_ok());
    }

    #[test]
    fn nesting_is_bounded() {
        let wrap = |open: &str, n: usize, close: &str| format!("{}1{}", open.repeat(n), close.repeat(n));
        for src in [
            wrap("- ", 30000, ""),
            wrap("!", 60000, ""),
            wrap("[", 16000, ""),
            wrap("(", 30000, ")"),
            wrap("{", 30000, "}"),
            wrap("if(1)", 12000, ""),
            wrap("1?1:", 15000, ""),
            wrap("2**", 20000, ""),
            format!("1{}", "+1".repeat(30000)),
            format!("a{}", "[0]".repeat(20000)),
            wrap("f(", 30000, ")"),
            format!("f{}", "()".repeat(30000)),
            format!("a{}", ".b".repeat(30000)),
        ] {
            let e = parse(&src).unwrap_err();
            assert!(e.message.contains("nested too deeply"), "{}…: {}", &src[..12], e.message);
        }
        assert!(parse(&wrap("(", MAX_DEPTH - 2, ")")).is_ok());
        assert!(parse(&wrap("(", MAX_DEPTH - 1, ")")).is_err());
        assert!(parse(&format!("1{}", "+1".repeat(MAX_HEIGHT - 1))).is_ok());
        assert!(parse(&format!("1{}", "+1".repeat(MAX_HEIGHT))).is_err());
    }

    #[test]
    fn rejections() {
        for (src, msg) in [
            ("for (;;) {}", "loops"),
            ("function f() {}", "not supported"),
            ("{a: 1}", "unexpected"),
            ("({a: 1})", "object literals"),
            ("`x`", "template"),
            ("1 +", "end of expression"),
            ("-2 ** 2", "parentheses"),
            ("let time = 1", "built-in"),
            ("'abc", "unterminated"),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.message.contains(msg), "{src}: {}", e.message);
        }
    }
}
