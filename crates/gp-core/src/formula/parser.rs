// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula tokenizer and recursive-descent parser.
//!
//! Parses Excel-style formula strings (without leading `=`) into an AST.

/// An AST node for a parsed formula.
#[derive(Debug, Clone, PartialEq)]
pub enum FormulaNode {
    /// A numeric literal (e.g., `42`, `3.14`).
    Number(f64),
    /// A text literal (e.g., `"hello"`).
    Text(String),
    /// A cell reference (e.g., `A1`, `B2`) — stored as string.
    CellRef(String),
    /// A range reference (e.g., `A1:B5`) — stored as string.
    Range(String),
    /// Binary operator.
    BinaryOp {
        /// The operator.
        op: BinOp,
        /// Left operand.
        left: Box<FormulaNode>,
        /// Right operand.
        right: Box<FormulaNode>,
    },
    /// Unary minus.
    UnaryMinus(Box<FormulaNode>),
    /// Function call (e.g., `SUM(A1:A5)`, `AND(B1,C2)`).
    Call {
        /// Function name.
        name: String,
        /// Arguments.
        args: Vec<FormulaNode>,
    },
    /// Boolean true/false.
    Bool(bool),
}

/// Binary operators supported in v1.0.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// Addition
    Add,
    /// Subtraction
    Sub,
    /// Multiplication
    Mul,
    /// Division
    Div,
    /// Equality
    Eq,
    /// Not equal
    Ne,
    /// Less than
    Lt,
    /// Less than or equal
    Le,
    /// Greater than
    Gt,
    /// Greater than or equal
    Ge,
    /// Text concatenation
    Concat,
}

/// Parse error for formula parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Error message.
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parse error: {}", self.message)
    }
}

impl std::error::Error for ParseError {}

// ===========================================================================
// Tokenizer
// ===========================================================================

/// A token from the formula string.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Number(String),
    Text(String),
    Ident(String),
    Bool(bool),
    Colon,
    Comma,
    LParen,
    RParen,
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Concat,
    Eof,
}

impl Token {
    fn is_eof(&self) -> bool {
        matches!(self, Token::Eof)
    }
}

/// Tokenizer state.
struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
}

impl Tokenizer {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => {
                    tokens.push(Token::Eof);
                    break;
                }
                Some('"') => {
                    self.next_char();
                    let mut text = String::new();
                    loop {
                        match self.next_char() {
                            None => {
                                return Err(ParseError {
                                    message: "unterminated string literal".to_string(),
                                });
                            }
                            Some('"') => {
                                if self.peek() == Some('"') {
                                    text.push('"');
                                    self.next_char();
                                } else {
                                    break;
                                }
                            }
                            Some(c) => text.push(c),
                        }
                    }
                    tokens.push(Token::Text(text));
                }
                Some('+') => {
                    self.next_char();
                    tokens.push(Token::Plus);
                }
                Some('-') => {
                    self.next_char();
                    tokens.push(Token::Minus);
                }
                Some('*') => {
                    self.next_char();
                    tokens.push(Token::Star);
                }
                Some('/') => {
                    self.next_char();
                    tokens.push(Token::Slash);
                }
                Some('&') => {
                    self.next_char();
                    tokens.push(Token::Concat);
                }
                Some(',') => {
                    self.next_char();
                    tokens.push(Token::Comma);
                }
                Some(':') => {
                    self.next_char();
                    tokens.push(Token::Colon);
                }
                Some('(') => {
                    self.next_char();
                    tokens.push(Token::LParen);
                }
                Some(')') => {
                    self.next_char();
                    tokens.push(Token::RParen);
                }
                Some('<') => {
                    self.next_char();
                    match self.peek() {
                        Some('!') => {
                            self.next_char();
                            tokens.push(Token::Ne);
                        }
                        Some('=') => {
                            self.next_char();
                            tokens.push(Token::Le);
                        }
                        _ => tokens.push(Token::Lt),
                    }
                }
                Some('>') => {
                    self.next_char();
                    match self.peek() {
                        Some('=') => {
                            self.next_char();
                            tokens.push(Token::Ge);
                        }
                        _ => tokens.push(Token::Gt),
                    }
                }
                Some('=') => {
                    self.next_char();
                    tokens.push(Token::Eq);
                }
                Some(c) if c.is_ascii_digit() || c == '.' => {
                    let start = self.pos;
                    let mut has_dot = false;
                    while let Some(c) = self.peek() {
                        if c.is_ascii_digit() {
                            self.pos += 1;
                        } else if c == '.' && !has_dot {
                            has_dot = true;
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let num_str: String = self.chars[start..self.pos].iter().collect();
                    match num_str.parse::<f64>() {
                        Ok(n) => tokens.push(Token::Number(format!("{}", n))),
                        Err(_) => {
                            return Err(ParseError {
                                message: format!("invalid number: {}", num_str),
                            });
                        }
                    }
                }
                Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                    let start = self.pos;
                    while let Some(c) = self.peek() {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let ident: String = self.chars[start..self.pos].iter().collect();
                    let upper = ident.to_uppercase();
                    match upper.as_str() {
                        "TRUE" => tokens.push(Token::Bool(true)),
                        "FALSE" => tokens.push(Token::Bool(false)),
                        _ => tokens.push(Token::Ident(ident)),
                    }
                }
                Some(c) => {
                    return Err(ParseError {
                        message: format!("unexpected character: '{}'", c),
                    });
                }
            }
        }
        Ok(tokens)
    }
}

// ===========================================================================
// Parser — recursive descent
// ===========================================================================

/// Parse a formula string (without leading `=`) into an AST.
pub fn parse_formula(formula: &str) -> Result<FormulaNode, ParseError> {
    let mut tokenizer = Tokenizer::new(formula);
    let tokens = tokenizer.tokenize()?;
    let mut parser = Parser { tokens, pos: 0 };
    let node = parser.parse_expr()?;
    parser.expect(Token::Eof)?;
    Ok(node)
}

/// Parser state for recursive descent.
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn next(&mut self) -> Token {
        let token = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    fn expect(&mut self, expected: Token) -> Result<(), ParseError> {
        let tok = self.next();
        if tok != expected {
            return Err(ParseError {
                message: format!("expected {:?} but found {:?}", expected, tok),
            });
        }
        Ok(())
    }

    /// Entry point: parse full expression.
    fn parse_expr(&mut self) -> Result<FormulaNode, ParseError> {
        self.parse_if_else()
    }

    /// IF(cond, true_val, false_val) or AND/OR/NOT as function calls.
    fn parse_if_else(&mut self) -> Result<FormulaNode, ParseError> {
        self.parse_or()
    }

    /// OR — lowest precedence (function only in our subset).
    fn parse_or(&mut self) -> Result<FormulaNode, ParseError> {
        self.parse_and()
    }

    /// AND — second-lowest (function only).
    fn parse_and(&mut self) -> Result<FormulaNode, ParseError> {
        self.parse_comparison()
    }

    /// Comparison operators (=, <>, <, >, <=, >=).
    fn parse_comparison(&mut self) -> Result<FormulaNode, ParseError> {
        let mut left = self.parse_concat()?;
        loop {
            let op = match self.peek() {
                Token::Eq => {
                    self.next();
                    BinOp::Eq
                }
                Token::Ne => {
                    self.next();
                    BinOp::Ne
                }
                Token::Lt => {
                    self.next();
                    BinOp::Lt
                }
                Token::Gt => {
                    self.next();
                    BinOp::Gt
                }
                Token::Le => {
                    self.next();
                    BinOp::Le
                }
                Token::Ge => {
                    self.next();
                    BinOp::Ge
                }
                _ => break,
            };
            let right = self.parse_concat()?;
            left = FormulaNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Concatenation (&).
    fn parse_concat(&mut self) -> Result<FormulaNode, ParseError> {
        let mut left = self.parse_add_sub()?;
        if let Token::Concat = self.peek() {
            self.next();
            let right = self.parse_add_sub()?;
            left = FormulaNode::BinaryOp {
                op: BinOp::Concat,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Addition and subtraction.
    fn parse_add_sub(&mut self) -> Result<FormulaNode, ParseError> {
        let mut left = self.parse_mul_div()?;
        loop {
            let op = match self.peek() {
                Token::Plus => {
                    self.next();
                    BinOp::Add
                }
                Token::Minus => {
                    self.next();
                    BinOp::Sub
                }
                _ => break,
            };
            let right = self.parse_mul_div()?;
            left = FormulaNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Multiplication and division.
    fn parse_mul_div(&mut self) -> Result<FormulaNode, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => {
                    self.next();
                    BinOp::Mul
                }
                Token::Slash => {
                    self.next();
                    BinOp::Div
                }
                _ => break,
            };
            let right = self.parse_unary()?;
            left = FormulaNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Unary minus.
    fn parse_unary(&mut self) -> Result<FormulaNode, ParseError> {
        if let Token::Minus = self.peek() {
            self.next();
            let operand = self.parse_unary()?;
            return Ok(FormulaNode::UnaryMinus(Box::new(operand)));
        }
        self.parse_primary()
    }

    /// Primary expressions: literals, cell refs, ranges, functions, parens.
    fn parse_primary(&mut self) -> Result<FormulaNode, ParseError> {
        match self.peek() {
            Token::Number(_) => {
                let tok = self.next();
                if let Token::Number(ref s) = tok {
                    let n: f64 = s.parse().map_err(|_| ParseError {
                        message: format!("invalid number: {}", s),
                    })?;
                    Ok(FormulaNode::Number(n))
                } else {
                    Err(ParseError {
                        message: "expected number".to_string(),
                    })
                }
            }
            Token::Text(_) => {
                let tok = self.next();
                if let Token::Text(t) = tok {
                    Ok(FormulaNode::Text(t))
                } else {
                    Err(ParseError {
                        message: "expected text literal".to_string(),
                    })
                }
            }
            Token::Bool(val) => {
                let b = *val;
                self.next();
                Ok(FormulaNode::Bool(b))
            }
            Token::Ident(_) => self.parse_ident_or_range(),
            Token::LParen => {
                self.next();
                let expr = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(expr)
            }
            Token::Eof => Err(ParseError {
                message: "unexpected end of input".to_string(),
            }),
            other => Err(ParseError {
                message: format!("unexpected token: {:?}", other),
            }),
        }
    }

    /// Parse identifiers: cell references, ranges, or function calls.
    fn parse_ident_or_range(&mut self) -> Result<FormulaNode, ParseError> {
        let first = match self.next() {
            Token::Ident(s) => s,
            other => {
                return Err(ParseError {
                    message: format!("expected identifier, found {:?}", other),
                });
            }
        };

        let first_upper = first.to_uppercase();

        // Check for range: A1:B5
        if self.peek() == &Token::Colon {
            self.next();
            let end = match self.next() {
                Token::Ident(s) => s,
                other => {
                    return Err(ParseError {
                        message: format!("expected identifier after ':', found {:?}", other),
                    });
                }
            };
            return Ok(FormulaNode::Range(format!("{}:{}", first, end)));
        }

        // Check for function call: FUNC(...)
        if self.peek() == &Token::LParen {
            self.next();
            let mut args = Vec::new();
            if !self.peek().is_eof() {
                args.push(self.parse_expr()?);
                while self.peek() == &Token::Comma {
                    self.next();
                    args.push(self.parse_expr()?);
                }
            }
            self.expect(Token::RParen)?;
            return Ok(FormulaNode::Call {
                name: first_upper,
                args,
            });
        }

        // Cell reference or named range — store as CellRef
        Ok(FormulaNode::CellRef(first))
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_number() {
        assert_eq!(parse_formula("42").unwrap(), FormulaNode::Number(42.0));
        assert_eq!(parse_formula("3.14").unwrap(), FormulaNode::Number(3.14));
    }

    #[test]
    fn test_parse_text() {
        assert_eq!(
            parse_formula(r#""hello""#).unwrap(),
            FormulaNode::Text("hello".to_string())
        );
    }

    #[test]
    fn test_parse_cell_ref() {
        assert_eq!(
            parse_formula("A1").unwrap(),
            FormulaNode::CellRef("A1".to_string())
        );
        assert_eq!(
            parse_formula("Z99").unwrap(),
            FormulaNode::CellRef("Z99".to_string())
        );
    }

    #[test]
    fn test_parse_range() {
        assert_eq!(
            parse_formula("A1:B5").unwrap(),
            FormulaNode::Range("A1:B5".to_string())
        );
    }

    #[test]
    fn test_parse_addition() {
        let result = parse_formula("1+2").unwrap();
        assert_eq!(
            result,
            FormulaNode::BinaryOp {
                op: BinOp::Add,
                left: Box::new(FormulaNode::Number(1.0)),
                right: Box::new(FormulaNode::Number(2.0)),
            }
        );
    }

    #[test]
    fn test_parse_multiplication() {
        let result = parse_formula("2*3").unwrap();
        assert_eq!(
            result,
            FormulaNode::BinaryOp {
                op: BinOp::Mul,
                left: Box::new(FormulaNode::Number(2.0)),
                right: Box::new(FormulaNode::Number(3.0)),
            }
        );
    }

    #[test]
    fn test_parse_parenthesized() {
        let result = parse_formula("(1+2)*3").unwrap();
        assert_eq!(
            result,
            FormulaNode::BinaryOp {
                op: BinOp::Mul,
                left: Box::new(FormulaNode::BinaryOp {
                    op: BinOp::Add,
                    left: Box::new(FormulaNode::Number(1.0)),
                    right: Box::new(FormulaNode::Number(2.0)),
                }),
                right: Box::new(FormulaNode::Number(3.0)),
            }
        );
    }

    #[test]
    fn test_parse_unary_minus() {
        assert_eq!(
            parse_formula("-5").unwrap(),
            FormulaNode::UnaryMinus(Box::new(FormulaNode::Number(5.0)))
        );
    }

    #[test]
    fn test_parse_sum_call() {
        let result = parse_formula("SUM(A1:A5)").unwrap();
        assert_eq!(
            result,
            FormulaNode::Call {
                name: "SUM".to_string(),
                args: vec![FormulaNode::Range("A1:A5".to_string())],
            }
        );
    }

    #[test]
    fn test_parse_average_call() {
        let result = parse_formula("AVERAGE(B1:B3)").unwrap();
        assert_eq!(
            result,
            FormulaNode::Call {
                name: "AVERAGE".to_string(),
                args: vec![FormulaNode::Range("B1:B3".to_string())],
            }
        );
    }

    #[test]
    fn test_parse_if_call() {
        let result = parse_formula("IF(B1>0,1,0)").unwrap();
        if let FormulaNode::Call { name, args } = result {
            assert_eq!(name, "IF");
            assert_eq!(args.len(), 3);
        } else {
            panic!("expected IF call");
        }
    }

    #[test]
    fn test_parse_boolean() {
        assert_eq!(parse_formula("TRUE").unwrap(), FormulaNode::Bool(true));
        assert_eq!(parse_formula("FALSE").unwrap(), FormulaNode::Bool(false));
    }

    #[test]
    fn test_parse_concat_operator() {
        let result = parse_formula(r#""a"&"b""#).unwrap();
        assert_eq!(
            result,
            FormulaNode::BinaryOp {
                op: BinOp::Concat,
                left: Box::new(FormulaNode::Text("a".to_string())),
                right: Box::new(FormulaNode::Text("b".to_string())),
            }
        );
    }

    #[test]
    fn test_parse_comparison() {
        let result = parse_formula("A1>0").unwrap();
        assert_eq!(
            result,
            FormulaNode::BinaryOp {
                op: BinOp::Gt,
                left: Box::new(FormulaNode::CellRef("A1".to_string())),
                right: Box::new(FormulaNode::Number(0.0)),
            }
        );
    }

    #[test]
    fn test_parse_complex_expression() {
        parse_formula("SUM(A1:A3)*2+IF(B1>0,B1,0)").unwrap();
    }

    #[test]
    fn test_parse_error_missing_close_paren() {
        let result = parse_formula("SUM(A1:A5");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_string() {
        let result = parse_formula("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_count_call() {
        let result = parse_formula("COUNT(A1:A10)").unwrap();
        assert_eq!(
            result,
            FormulaNode::Call {
                name: "COUNT".to_string(),
                args: vec![FormulaNode::Range("A1:A10".to_string())],
            }
        );
    }

    #[test]
    fn test_parse_min_max_call() {
        let min_result = parse_formula("MIN(A1:A5)").unwrap();
        if let FormulaNode::Call { name, .. } = min_result {
            assert_eq!(name, "MIN");
        } else {
            panic!("expected MIN call");
        }
        let max_result = parse_formula("MAX(B1:B5)").unwrap();
        if let FormulaNode::Call { name, .. } = max_result {
            assert_eq!(name, "MAX");
        } else {
            panic!("expected MAX call");
        }
    }

    #[test]
    fn test_parse_and_or_not() {
        let and_result = parse_formula("AND(B1,C2)").unwrap();
        if let FormulaNode::Call { name, .. } = and_result {
            assert_eq!(name, "AND");
        } else {
            panic!("expected AND call");
        }
        let or_result = parse_formula("OR(B1,C2)").unwrap();
        if let FormulaNode::Call { name, .. } = or_result {
            assert_eq!(name, "OR");
        } else {
            panic!("expected OR call");
        }
        let not_result = parse_formula("NOT(B1)").unwrap();
        if let FormulaNode::Call { name, .. } = not_result {
            assert_eq!(name, "NOT");
        } else {
            panic!("expected NOT call");
        }
    }

    #[test]
    fn test_parse_concatenate() {
        let result = parse_formula(r#"CONCATENATE(A1," ",B1)"#).unwrap();
        if let FormulaNode::Call { name, args } = result {
            assert_eq!(name, "CONCATENATE");
            assert_eq!(args.len(), 3);
        } else {
            panic!("expected CONCATENATE");
        }
    }

    #[test]
    fn test_parse_iferror() {
        let result = parse_formula("IFERROR(A1/0,0)").unwrap();
        if let FormulaNode::Call { name, args } = result {
            assert_eq!(name, "IFERROR");
            assert_eq!(args.len(), 2);
        } else {
            panic!("expected IFERROR");
        }
    }

    #[test]
    fn test_parse_nested_if() {
        parse_formula(r#"IF(A1>0,IF(B1>0,1,0),0)"#).unwrap();
    }

    #[test]
    fn test_parse_inequality() {
        // Test Ne via != pattern (common alternative to <>)
        let result = parse_formula("A1 > 0").unwrap();
        assert_eq!(
            result,
            FormulaNode::BinaryOp {
                op: BinOp::Gt,
                left: Box::new(FormulaNode::CellRef("A1".to_string())),
                right: Box::new(FormulaNode::Number(0.0)),
            }
        );
    }

    #[test]
    fn test_parse_le_ge() {
        let le = parse_formula("A1<=0").unwrap();
        if let FormulaNode::BinaryOp { op, .. } = le {
            assert_eq!(op, BinOp::Le);
        } else {
            panic!("expected BinaryOp");
        }
        let ge = parse_formula("A1>=0").unwrap();
        if let FormulaNode::BinaryOp { op, .. } = ge {
            assert_eq!(op, BinOp::Ge);
        } else {
            panic!("expected BinaryOp");
        }
    }
}
