//! Cypher tokenizer with line/column tracking.

use mygraph_core::{MyGraphError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Unquoted identifier or keyword (keywords are matched case-insensitively by the parser).
    Ident(String),
    /// Backtick-quoted identifier; never a keyword.
    QuotedIdent(String),
    Str(String),
    Int(i64),
    Float(f64),
    Param(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Dot,
    Pipe,
    Plus,
    PlusEq,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    /// `=~`
    RegexMatch,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub column: u32,
    /// Byte offsets into the source.
    pub start: usize,
    pub end: usize,
}

struct Lexer<'a> {
    src: &'a str,
    chars: Vec<(usize, char)>,
    i: usize,
    line: u32,
    column: u32,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    let mut lx = Lexer { src, chars: src.char_indices().collect(), i: 0, line: 1, column: 1 };
    let mut out = Vec::new();
    loop {
        lx.skip_trivia()?;
        let (line, column, start) = (lx.line, lx.column, lx.offset());
        let Some(c) = lx.peek(0) else {
            out.push(Token { tok: Tok::Eof, line, column, start, end: start });
            return Ok(out);
        };
        let tok = lx.next_token(c, line, column)?;
        out.push(Token { tok, line, column, start, end: lx.offset() });
    }
}

fn syntax(message: impl Into<String>, line: u32, column: u32) -> MyGraphError {
    MyGraphError::Syntax { message: message.into(), line, column }
}

impl Lexer<'_> {
    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.i + k).map(|(_, c)| *c)
    }

    fn offset(&self) -> usize {
        self.chars.get(self.i).map_or(self.src.len(), |(o, _)| *o)
    }

    fn bump(&mut self) -> char {
        let c = self.chars[self.i].1;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        c
    }

    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            match (self.peek(0), self.peek(1)) {
                (Some(c), _) if c.is_whitespace() => {
                    self.bump();
                }
                (Some('/'), Some('/')) => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                (Some('/'), Some('*')) => {
                    let (line, column) = (self.line, self.column);
                    self.bump();
                    self.bump();
                    loop {
                        match (self.peek(0), self.peek(1)) {
                            (Some('*'), Some('/')) => {
                                self.bump();
                                self.bump();
                                break;
                            }
                            (Some(_), _) => {
                                self.bump();
                            }
                            (None, _) => return Err(syntax("unterminated comment", line, column)),
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next_token(&mut self, c: char, line: u32, column: u32) -> Result<Tok> {
        if c.is_alphabetic() || c == '_' {
            return Ok(Tok::Ident(self.ident_chars()));
        }
        if c.is_ascii_digit() {
            return self.number(line, column);
        }
        self.bump();
        let tok = match c {
            '`' => {
                let mut s = String::new();
                loop {
                    match self.peek(0) {
                        Some('`') if self.peek(1) == Some('`') => {
                            self.bump();
                            self.bump();
                            s.push('`');
                        }
                        Some('`') => {
                            self.bump();
                            break;
                        }
                        Some(_) => s.push(self.bump()),
                        None => return Err(syntax("unterminated quoted identifier", line, column)),
                    }
                }
                Tok::QuotedIdent(s)
            }
            '\'' | '"' => Tok::Str(self.string(c, line, column)?),
            '$' => {
                if self.peek(0).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    Tok::Param(self.ident_chars())
                } else {
                    return Err(syntax("expected parameter name after `$`", line, column));
                }
            }
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            '{' => Tok::LBrace,
            '}' => Tok::RBrace,
            ',' => Tok::Comma,
            ':' => Tok::Colon,
            ';' => Tok::Semi,
            '.' => Tok::Dot,
            '|' => Tok::Pipe,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '^' => Tok::Caret,
            '-' => Tok::Minus,
            '+' => {
                if self.peek(0) == Some('=') {
                    self.bump();
                    Tok::PlusEq
                } else {
                    Tok::Plus
                }
            }
            '=' => {
                if self.peek(0) == Some('~') {
                    self.bump();
                    Tok::RegexMatch
                } else {
                    Tok::Eq
                }
            }
            '<' => match self.peek(0) {
                Some('=') => {
                    self.bump();
                    Tok::Le
                }
                Some('>') => {
                    self.bump();
                    Tok::Neq
                }
                _ => Tok::Lt,
            },
            '>' => {
                if self.peek(0) == Some('=') {
                    self.bump();
                    Tok::Ge
                } else {
                    Tok::Gt
                }
            }
            '!' if self.peek(0) == Some('=') => {
                self.bump();
                Tok::Neq
            }
            other => return Err(syntax(format!("unexpected character `{other}`"), line, column)),
        };
        Ok(tok)
    }

    fn ident_chars(&mut self) -> String {
        let mut s = String::new();
        while self.peek(0).is_some_and(|c| c.is_alphanumeric() || c == '_') {
            s.push(self.bump());
        }
        s
    }

    fn number(&mut self, line: u32, column: u32) -> Result<Tok> {
        let mut s = String::new();
        let mut is_float = false;
        while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
            s.push(self.bump());
        }
        if self.peek(0) == Some('.') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            s.push(self.bump());
            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                s.push(self.bump());
            }
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let sign = matches!(self.peek(1), Some('+' | '-'));
            let digit_at = if sign { 2 } else { 1 };
            if self.peek(digit_at).is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                s.push(self.bump());
                if sign {
                    s.push(self.bump());
                }
                while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                    s.push(self.bump());
                }
            }
        }
        if self.peek(0).is_some_and(|c| c.is_alphabetic() || c == '_') {
            return Err(syntax(format!("invalid number literal `{s}{}`", self.peek(0).unwrap()), line, column));
        }
        if is_float {
            s.parse().map(Tok::Float).map_err(|_| syntax(format!("invalid float `{s}`"), line, column))
        } else {
            s.parse().map(Tok::Int).map_err(|_| syntax(format!("integer `{s}` is too large"), line, column))
        }
    }

    fn string(&mut self, quote: char, line: u32, column: u32) -> Result<String> {
        let mut s = String::new();
        loop {
            let Some(c) = self.peek(0) else {
                return Err(syntax("unterminated string literal", line, column));
            };
            self.bump();
            if c == quote {
                return Ok(s);
            }
            if c != '\\' {
                s.push(c);
                continue;
            }
            let (el, ec) = (self.line, self.column);
            let Some(e) = self.peek(0) else {
                return Err(syntax("unterminated string literal", line, column));
            };
            self.bump();
            match e {
                'n' => s.push('\n'),
                't' => s.push('\t'),
                'r' => s.push('\r'),
                'b' => s.push('\u{8}'),
                'f' => s.push('\u{c}'),
                '\\' | '\'' | '"' => s.push(e),
                'u' => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        match self.peek(0) {
                            Some(h) if h.is_ascii_hexdigit() => hex.push(self.bump()),
                            _ => return Err(syntax("invalid \\u escape", el, ec)),
                        }
                    }
                    let code = u32::from_str_radix(&hex, 16).expect("hex digits");
                    s.push(char::from_u32(code).ok_or_else(|| syntax("invalid unicode escape", el, ec))?);
                }
                other => return Err(syntax(format!("invalid escape sequence `\\{other}`"), el, ec)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        tokenize(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn tokenizes_pattern() {
        assert_eq!(
            toks("(a)-[:R]->(b)"),
            vec![
                Tok::LParen,
                Tok::Ident("a".into()),
                Tok::RParen,
                Tok::Minus,
                Tok::LBracket,
                Tok::Colon,
                Tok::Ident("R".into()),
                Tok::RBracket,
                Tok::Minus,
                Tok::Gt,
                Tok::LParen,
                Tok::Ident("b".into()),
                Tok::RParen,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn tokenizes_literals_and_comments() {
        assert_eq!(
            toks("1 2.5 1e3 'it\\'s' \"x\" $p // c\n /* d */ <> <= `a b`"),
            vec![
                Tok::Int(1),
                Tok::Float(2.5),
                Tok::Float(1000.0),
                Tok::Str("it's".into()),
                Tok::Str("x".into()),
                Tok::Param("p".into()),
                Tok::Neq,
                Tok::Le,
                Tok::QuotedIdent("a b".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn tokenizes_regex_match() {
        assert_eq!(
            toks("a =~ 'x' = b"),
            vec![
                Tok::Ident("a".into()),
                Tok::RegexMatch,
                Tok::Str("x".into()),
                Tok::Eq,
                Tok::Ident("b".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn reports_position() {
        let err = tokenize("RETURN\n  'abc").unwrap_err();
        assert!(matches!(err, MyGraphError::Syntax { line: 2, column: 3, .. }), "{err:?}");
    }
}
