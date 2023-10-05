//! Lexer shared by both back ends.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    Comma,
    Dot,
    Minus,
    Plus,
    Semicolon,
    Slash,
    Star,
    Bang,
    BangEqual,
    Equal,
    EqualEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Identifier,
    String,
    Number,
    And,
    Class,
    Else,
    False,
    Fun,
    For,
    If,
    Nil,
    Or,
    Print,
    Return,
    Super,
    This,
    True,
    Var,
    While,
    Error,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    /// Source text of the token. For `Error` tokens, the error message.
    pub lexeme: String,
    pub line: usize,
}

pub struct Scanner<'a> {
    src: &'a [u8],
    start: usize,
    current: usize,
    line: usize,
}

impl<'a> Scanner<'a> {
    pub fn new(src: &'a str) -> Self {
        Scanner { src: src.as_bytes(), start: 0, current: 0, line: 1 }
    }

    /// Scan everything, ending with an `Eof` token.
    pub fn scan_all(mut self) -> Vec<Token> {
        let mut out = Vec::new();
        loop {
            let t = self.next_token();
            let eof = t.kind == TokenKind::Eof;
            out.push(t);
            if eof {
                return out;
            }
        }
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        self.start = self.current;
        let Some(c) = self.advance() else {
            return self.make(TokenKind::Eof);
        };
        use TokenKind::*;
        match c {
            b'(' => self.make(LeftParen),
            b')' => self.make(RightParen),
            b'{' => self.make(LeftBrace),
            b'}' => self.make(RightBrace),
            b',' => self.make(Comma),
            b'.' => self.make(Dot),
            b'-' => self.make(Minus),
            b'+' => self.make(Plus),
            b';' => self.make(Semicolon),
            b'/' => self.make(Slash),
            b'*' => self.make(Star),
            b'!' => self.two(b'=', BangEqual, Bang),
            b'=' => self.two(b'=', EqualEqual, Equal),
            b'<' => self.two(b'=', LessEqual, Less),
            b'>' => self.two(b'=', GreaterEqual, Greater),
            b'"' => self.string(),
            c if c.is_ascii_digit() => self.number(),
            c if is_alpha(c) => self.identifier(),
            _ => self.error("Unexpected character."),
        }
    }

    fn advance(&mut self) -> Option<u8> {
        let c = self.src.get(self.current).copied()?;
        self.current += 1;
        Some(c)
    }

    fn peek(&self) -> u8 {
        self.src.get(self.current).copied().unwrap_or(0)
    }

    fn peek_next(&self) -> u8 {
        self.src.get(self.current + 1).copied().unwrap_or(0)
    }

    fn two(&mut self, next: u8, yes: TokenKind, no: TokenKind) -> Token {
        if self.peek() == next {
            self.current += 1;
            self.make(yes)
        } else {
            self.make(no)
        }
    }

    fn skip_whitespace(&mut self) {
        loop {
            match self.peek() {
                b' ' | b'\r' | b'\t' => self.current += 1,
                b'\n' => {
                    self.line += 1;
                    self.current += 1;
                }
                b'/' if self.peek_next() == b'/' => {
                    while self.peek() != b'\n' && self.current < self.src.len() {
                        self.current += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn string(&mut self) -> Token {
        while self.peek() != b'"' && self.current < self.src.len() {
            if self.peek() == b'\n' {
                self.line += 1;
            }
            self.current += 1;
        }
        if self.current >= self.src.len() {
            return self.error("Unterminated string.");
        }
        self.current += 1;
        self.make(TokenKind::String)
    }

    fn number(&mut self) -> Token {
        while self.peek().is_ascii_digit() {
            self.current += 1;
        }
        if self.peek() == b'.' && self.peek_next().is_ascii_digit() {
            self.current += 1;
            while self.peek().is_ascii_digit() {
                self.current += 1;
            }
        }
        self.make(TokenKind::Number)
    }

    fn identifier(&mut self) -> Token {
        while is_alpha(self.peek()) || self.peek().is_ascii_digit() {
            self.current += 1;
        }
        let text = &self.src[self.start..self.current];
        use TokenKind::*;
        let kind = match text {
            b"and" => And,
            b"class" => Class,
            b"else" => Else,
            b"false" => False,
            b"for" => For,
            b"fun" => Fun,
            b"if" => If,
            b"nil" => Nil,
            b"or" => Or,
            b"print" => Print,
            b"return" => Return,
            b"super" => Super,
            b"this" => This,
            b"true" => True,
            b"var" => Var,
            b"while" => While,
            _ => Identifier,
        };
        self.make(kind)
    }

    fn make(&self, kind: TokenKind) -> Token {
        Token {
            kind,
            lexeme: String::from_utf8_lossy(&self.src[self.start..self.current]).into_owned(),
            line: self.line,
        }
    }

    fn error(&self, msg: &str) -> Token {
        Token { kind: TokenKind::Error, lexeme: msg.to_string(), line: self.line }
    }
}

fn is_alpha(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    fn kinds(s: &str) -> Vec<TokenKind> {
        Scanner::new(s).scan_all().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn punctuation_and_operators() {
        assert_eq!(
            kinds("(){},.-+;*/ ! != = == < <= > >="),
            vec![
                LeftParen, RightParen, LeftBrace, RightBrace, Comma, Dot, Minus, Plus, Semicolon,
                Star, Slash, Bang, BangEqual, Equal, EqualEqual, Less, LessEqual, Greater,
                GreaterEqual, Eof
            ]
        );
    }

    #[test]
    fn keywords_literals_and_comments() {
        let toks = Scanner::new("var x = 1.5; // comment\nprint \"hi\\n\" and classy;").scan_all();
        let k: Vec<_> = toks.iter().map(|t| t.kind).collect();
        assert_eq!(k, vec![Var, Identifier, Equal, Number, Semicolon, Print, String, And, Identifier, Semicolon, Eof]);
        assert_eq!(toks[3].lexeme, "1.5");
        assert_eq!(toks[5].line, 2);
    }

    #[test]
    fn unterminated_string_is_error() {
        assert_eq!(kinds("\"abc")[0], Error);
    }
}
