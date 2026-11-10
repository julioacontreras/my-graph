//! Recursive-descent parser for the supported Cypher subset.

use mygraph_core::{MyGraphError, Result, Value};

use crate::ast::*;
use crate::lexer::{tokenize, Tok, Token};

/// Parses a script of `;`-separated statements.
pub fn parse(src: &str) -> Result<Vec<Statement>> {
    let mut p = Parser { src, toks: tokenize(src)?, pos: 0 };
    let mut stmts = Vec::new();
    loop {
        while p.eat(&Tok::Semi) {}
        if p.at_eof() {
            return Ok(stmts);
        }
        stmts.push(p.statement()?);
        if !p.eat(&Tok::Semi) && !p.at_eof() {
            return Err(p.error("expected `;` or end of input"));
        }
    }
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("`{s}`"),
        Tok::QuotedIdent(s) => format!("`{s}`"),
        Tok::Str(s) => format!("string {s:?}"),
        Tok::Int(i) => format!("`{i}`"),
        Tok::Float(f) => format!("`{f}`"),
        Tok::Param(p) => format!("`${p}`"),
        Tok::Eof => "end of input".into(),
        other => format!("{other:?}"),
    }
}

impl Parser<'_> {
    // ---- token helpers ----

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }

    fn advance(&mut self) -> &Token {
        let t = &self.toks[self.pos];
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn error(&self, message: impl Into<String>) -> MyGraphError {
        let t = &self.toks[self.pos];
        MyGraphError::Syntax {
            message: format!("{} (found {})", message.into(), describe(&t.tok)),
            line: t.line,
            column: t.column,
        }
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == tok {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Tok, what: &str) -> Result<()> {
        if self.eat(tok) {
            Ok(())
        } else {
            Err(self.error(format!("expected {what}")))
        }
    }

    fn is_kw_at(&self, n: usize, kw: &str) -> bool {
        matches!(self.peek_at(n), Tok::Ident(s) if s.eq_ignore_ascii_case(kw))
    }

    fn is_kw(&self, kw: &str) -> bool {
        self.is_kw_at(0, kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: &str) -> Result<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(self.error(format!("expected `{kw}`")))
        }
    }

    fn ident(&mut self, what: &str) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) | Tok::QuotedIdent(s) => {
                self.advance();
                Ok(s)
            }
            _ => Err(self.error(format!("expected {what}"))),
        }
    }

    fn prev_end(&self) -> usize {
        self.toks[self.pos.saturating_sub(1)].end
    }

    // ---- statements ----

    fn statement(&mut self) -> Result<Statement> {
        if self.is_kw("CREATE") && (self.is_kw_at(1, "DATABASE") || self.is_kw_at(1, "SCHEMA")) {
            self.advance();
            let is_db = self.eat_kw("DATABASE");
            if !is_db {
                self.expect_kw("SCHEMA")?;
            }
            let if_not_exists = if self.eat_kw("IF") {
                self.expect_kw("NOT")?;
                self.expect_kw("EXISTS")?;
                true
            } else {
                false
            };
            let name = self.ident("a name")?;
            return Ok(Statement::Admin(if is_db {
                AdminCommand::CreateDatabase { name, if_not_exists }
            } else {
                AdminCommand::CreateSchema { name, if_not_exists }
            }));
        }
        if self.eat_kw("DROP") {
            let is_db = self.eat_kw("DATABASE");
            if !is_db && !self.eat_kw("SCHEMA") {
                return Err(self.error("expected `DATABASE` or `SCHEMA` (indexes are not supported yet)"));
            }
            let if_exists = if self.eat_kw("IF") {
                self.expect_kw("EXISTS")?;
                true
            } else {
                false
            };
            let name = self.ident("a name")?;
            return Ok(Statement::Admin(if is_db {
                AdminCommand::DropDatabase { name, if_exists }
            } else {
                AdminCommand::DropSchema { name, if_exists }
            }));
        }
        if self.eat_kw("USE") {
            if self.eat_kw("SCHEMA") {
                return Ok(Statement::Admin(AdminCommand::UseSchema(self.ident("a schema name")?)));
            }
            self.eat_kw("DATABASE");
            return Ok(Statement::Admin(AdminCommand::UseDatabase(self.ident("a database name")?)));
        }
        if self.eat_kw("SHOW") {
            if self.eat_kw("DATABASES") {
                return Ok(Statement::Admin(AdminCommand::ShowDatabases));
            }
            if self.eat_kw("SCHEMAS") {
                return Ok(Statement::Admin(AdminCommand::ShowSchemas));
            }
            return Err(self.error("expected `DATABASES` or `SCHEMAS`"));
        }
        if self.eat_kw("BEGIN") {
            return Ok(Statement::Begin);
        }
        if self.eat_kw("COMMIT") {
            return Ok(Statement::Commit);
        }
        if self.eat_kw("ROLLBACK") {
            return Ok(Statement::Rollback);
        }
        Ok(Statement::Query(self.query()?))
    }

    fn query(&mut self) -> Result<Query> {
        let mut clauses = Vec::new();
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
            if matches!(clauses.last(), Some(Clause::Return(_))) {
                return Err(self.error("RETURN must be the last clause of a query"));
            }
            clauses.push(self.clause()?);
        }
        match clauses.last() {
            None => Err(self.error("expected a query")),
            Some(Clause::Match { .. } | Clause::With { .. } | Clause::Unwind { .. }) => {
                Err(self.error("a query cannot end with MATCH, WITH or UNWIND; add a RETURN or an updating clause"))
            }
            _ => Ok(Query { clauses }),
        }
    }

    fn clause(&mut self) -> Result<Clause> {
        if self.is_kw("OPTIONAL") || self.is_kw("MATCH") {
            let optional = self.eat_kw("OPTIONAL");
            self.expect_kw("MATCH")?;
            let patterns = self.patterns()?;
            let filter = if self.eat_kw("WHERE") { Some(self.expr()?) } else { None };
            return Ok(Clause::Match { optional, patterns, filter });
        }
        if self.eat_kw("CREATE") {
            return Ok(Clause::Create { patterns: self.patterns()? });
        }
        if self.eat_kw("MERGE") {
            return Err(self.error("MERGE is not supported yet"));
        }
        if self.eat_kw("SET") {
            let mut items = vec![self.set_item()?];
            while self.eat(&Tok::Comma) {
                items.push(self.set_item()?);
            }
            return Ok(Clause::Set { items });
        }
        if self.eat_kw("REMOVE") {
            let mut items = vec![self.remove_item()?];
            while self.eat(&Tok::Comma) {
                items.push(self.remove_item()?);
            }
            return Ok(Clause::Remove { items });
        }
        if self.is_kw("DETACH") || self.is_kw("DELETE") {
            let detach = self.eat_kw("DETACH");
            self.expect_kw("DELETE")?;
            let mut exprs = vec![self.expr()?];
            while self.eat(&Tok::Comma) {
                exprs.push(self.expr()?);
            }
            return Ok(Clause::Delete { detach, exprs });
        }
        if self.eat_kw("UNWIND") {
            let expr = self.expr()?;
            self.expect_kw("AS")?;
            let variable = self.ident("a variable")?;
            return Ok(Clause::Unwind { expr, variable });
        }
        if self.eat_kw("WITH") {
            let projection = self.projection(true)?;
            let filter = if self.eat_kw("WHERE") { Some(self.expr()?) } else { None };
            return Ok(Clause::With { projection, filter });
        }
        if self.eat_kw("RETURN") {
            return Ok(Clause::Return(self.projection(false)?));
        }
        Err(self.error("expected a clause (MATCH, CREATE, SET, REMOVE, DELETE, UNWIND, WITH or RETURN)"))
    }

    fn set_item(&mut self) -> Result<SetItem> {
        let variable = self.ident("a variable")?;
        if self.eat(&Tok::Dot) {
            let key = self.ident("a property name")?;
            self.expect(&Tok::Eq, "`=`")?;
            return Ok(SetItem::Property { variable, key, value: self.expr()? });
        }
        if self.eat(&Tok::Eq) {
            return Ok(SetItem::Replace { variable, value: self.expr()? });
        }
        if self.eat(&Tok::PlusEq) {
            return Ok(SetItem::Merge { variable, value: self.expr()? });
        }
        if matches!(self.peek(), Tok::Colon) {
            return Ok(SetItem::Labels { variable, labels: self.labels()? });
        }
        Err(self.error("expected `.property =`, `=`, `+=` or `:Label`"))
    }

    fn remove_item(&mut self) -> Result<RemoveItem> {
        let variable = self.ident("a variable")?;
        if self.eat(&Tok::Dot) {
            return Ok(RemoveItem::Property { variable, key: self.ident("a property name")? });
        }
        if matches!(self.peek(), Tok::Colon) {
            return Ok(RemoveItem::Labels { variable, labels: self.labels()? });
        }
        Err(self.error("expected `.property` or `:Label`"))
    }

    fn labels(&mut self) -> Result<Vec<String>> {
        let mut labels = Vec::new();
        while self.eat(&Tok::Colon) {
            labels.push(self.ident("a label")?);
        }
        Ok(labels)
    }

    fn projection(&mut self, is_with: bool) -> Result<Projection> {
        let distinct = self.eat_kw("DISTINCT");
        let mut star = false;
        let mut items = Vec::new();
        if self.eat(&Tok::Star) {
            star = true;
            while self.eat(&Tok::Comma) {
                items.push(self.projection_item(is_with)?);
            }
        } else {
            items.push(self.projection_item(is_with)?);
            while self.eat(&Tok::Comma) {
                items.push(self.projection_item(is_with)?);
            }
        }
        let mut order_by = Vec::new();
        if self.eat_kw("ORDER") {
            self.expect_kw("BY")?;
            loop {
                let expr = self.expr()?;
                let descending = if self.eat_kw("DESC") || self.eat_kw("DESCENDING") {
                    true
                } else {
                    let _ = self.eat_kw("ASC") || self.eat_kw("ASCENDING");
                    false
                };
                order_by.push(SortItem { expr, descending });
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        let skip = if self.eat_kw("SKIP") { Some(self.expr()?) } else { None };
        let limit = if self.eat_kw("LIMIT") { Some(self.expr()?) } else { None };
        Ok(Projection { distinct, star, items, order_by, skip, limit })
    }

    fn projection_item(&mut self, is_with: bool) -> Result<ProjectionItem> {
        let start_tok = self.pos;
        let start = self.toks[self.pos].start;
        let expr = self.expr()?;
        let end = self.prev_end();
        if self.eat_kw("AS") {
            return Ok(ProjectionItem { expr, name: self.ident("an alias")? });
        }
        if let Expr::Variable(v) = &expr {
            return Ok(ProjectionItem { name: v.clone(), expr });
        }
        if is_with {
            let t = &self.toks[start_tok];
            return Err(MyGraphError::Syntax {
                message: "expressions in WITH must be aliased (use AS)".into(),
                line: t.line,
                column: t.column,
            });
        }
        Ok(ProjectionItem { expr, name: self.src[start..end].trim().to_string() })
    }

    // ---- patterns ----

    fn patterns(&mut self) -> Result<Vec<PathPattern>> {
        let mut out = vec![self.path()?];
        while self.eat(&Tok::Comma) {
            out.push(self.path()?);
        }
        Ok(out)
    }

    fn path(&mut self) -> Result<PathPattern> {
        if matches!(self.peek(), Tok::Ident(_) | Tok::QuotedIdent(_)) && matches!(self.peek_at(1), Tok::Eq) {
            return Err(self.error("named paths are not supported yet"));
        }
        let start = self.node_pattern()?;
        let mut steps = Vec::new();
        while matches!(self.peek(), Tok::Minus | Tok::Lt) {
            let rel = self.rel_pattern()?;
            let node = self.node_pattern()?;
            steps.push((rel, node));
        }
        Ok(PathPattern { start, steps })
    }

    fn properties(&mut self) -> Result<Option<Expr>> {
        match self.peek() {
            Tok::LBrace => Ok(Some(self.map_literal()?)),
            Tok::Param(p) => {
                let p = p.clone();
                self.advance();
                Ok(Some(Expr::Parameter(p)))
            }
            _ => Ok(None),
        }
    }

    fn node_pattern(&mut self) -> Result<NodePattern> {
        self.expect(&Tok::LParen, "`(` to start a node pattern")?;
        let variable = match self.peek() {
            Tok::Ident(_) | Tok::QuotedIdent(_) => Some(self.ident("a variable")?),
            _ => None,
        };
        let labels = self.labels()?;
        let properties = self.properties()?;
        self.expect(&Tok::RParen, "`)` to close the node pattern")?;
        Ok(NodePattern { variable, labels, properties })
    }

    fn rel_pattern(&mut self) -> Result<RelPattern> {
        let left = self.eat(&Tok::Lt);
        self.expect(&Tok::Minus, "`-`")?;
        let (mut variable, mut types, mut properties) = (None, Vec::new(), None);
        if self.eat(&Tok::LBracket) {
            if matches!(self.peek(), Tok::Ident(_) | Tok::QuotedIdent(_)) {
                variable = Some(self.ident("a variable")?);
            }
            if self.eat(&Tok::Colon) {
                types.push(self.ident("a relationship type")?);
                while self.eat(&Tok::Pipe) {
                    self.eat(&Tok::Colon);
                    types.push(self.ident("a relationship type")?);
                }
            }
            if matches!(self.peek(), Tok::Star) {
                return Err(self.error("variable-length relationships are not supported yet"));
            }
            properties = self.properties()?;
            self.expect(&Tok::RBracket, "`]` to close the relationship pattern")?;
        }
        self.expect(&Tok::Minus, "`-`")?;
        let right = self.eat(&Tok::Gt);
        let direction = match (left, right) {
            (false, true) => Direction::Outgoing,
            (true, false) => Direction::Incoming,
            (false, false) => Direction::Both,
            (true, true) => return Err(self.error("a relationship cannot point in both directions")),
        };
        Ok(RelPattern { variable, types, properties, direction })
    }

    // ---- expressions (lowest to highest precedence) ----

    pub(crate) fn expr(&mut self) -> Result<Expr> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Expr> {
        let mut e = self.xor_expr()?;
        while self.eat_kw("OR") {
            e = Expr::Binary(BinaryOp::Or, Box::new(e), Box::new(self.xor_expr()?));
        }
        Ok(e)
    }

    fn xor_expr(&mut self) -> Result<Expr> {
        let mut e = self.and_expr()?;
        while self.eat_kw("XOR") {
            e = Expr::Binary(BinaryOp::Xor, Box::new(e), Box::new(self.and_expr()?));
        }
        Ok(e)
    }

    fn and_expr(&mut self) -> Result<Expr> {
        let mut e = self.not_expr()?;
        while self.eat_kw("AND") {
            e = Expr::Binary(BinaryOp::And, Box::new(e), Box::new(self.not_expr()?));
        }
        Ok(e)
    }

    fn not_expr(&mut self) -> Result<Expr> {
        if self.eat_kw("NOT") {
            return Ok(Expr::Unary(UnaryOp::Not, Box::new(self.not_expr()?)));
        }
        self.comparison()
    }

    /// Chained comparisons (`a < b < c`) expand to `a < b AND b < c`.
    fn comparison(&mut self) -> Result<Expr> {
        let first = self.predicate()?;
        let mut parts: Vec<Expr> = Vec::new();
        let mut left = first.clone();
        loop {
            let op = match self.peek() {
                Tok::Eq => BinaryOp::Eq,
                Tok::Neq => BinaryOp::Neq,
                Tok::Lt => BinaryOp::Lt,
                Tok::Le => BinaryOp::Le,
                Tok::Gt => BinaryOp::Gt,
                Tok::Ge => BinaryOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.predicate()?;
            parts.push(Expr::Binary(op, Box::new(left), Box::new(right.clone())));
            left = right;
        }
        Ok(parts.into_iter().reduce(|a, b| Expr::Binary(BinaryOp::And, Box::new(a), Box::new(b))).unwrap_or(first))
    }

    /// String, list and null predicates.
    fn predicate(&mut self) -> Result<Expr> {
        let mut e = self.additive()?;
        loop {
            let op = if self.is_kw("STARTS") && self.is_kw_at(1, "WITH") {
                self.advance();
                self.advance();
                BinaryOp::StartsWith
            } else if self.is_kw("ENDS") && self.is_kw_at(1, "WITH") {
                self.advance();
                self.advance();
                BinaryOp::EndsWith
            } else if self.eat_kw("CONTAINS") {
                BinaryOp::Contains
            } else if self.eat_kw("IN") {
                BinaryOp::In
            } else if self.eat_kw("IS") {
                let negated = self.eat_kw("NOT");
                self.expect_kw("NULL")?;
                e = Expr::IsNull { expr: Box::new(e), negated };
                continue;
            } else {
                return Ok(e);
            };
            e = Expr::Binary(op, Box::new(e), Box::new(self.additive()?));
        }
    }

    fn additive(&mut self) -> Result<Expr> {
        let mut e = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinaryOp::Add,
                Tok::Minus => BinaryOp::Sub,
                _ => return Ok(e),
            };
            self.advance();
            e = Expr::Binary(op, Box::new(e), Box::new(self.multiplicative()?));
        }
    }

    fn multiplicative(&mut self) -> Result<Expr> {
        let mut e = self.power()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinaryOp::Mul,
                Tok::Slash => BinaryOp::Div,
                Tok::Percent => BinaryOp::Mod,
                _ => return Ok(e),
            };
            self.advance();
            e = Expr::Binary(op, Box::new(e), Box::new(self.power()?));
        }
    }

    fn power(&mut self) -> Result<Expr> {
        let mut e = self.unary()?;
        while self.eat(&Tok::Caret) {
            e = Expr::Binary(BinaryOp::Pow, Box::new(e), Box::new(self.unary()?));
        }
        Ok(e)
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.eat(&Tok::Minus) {
            // Fold negative numeric literals so i64::MIN is representable.
            if let Tok::Int(i) = self.peek() {
                let i = *i;
                self.advance();
                return self.postfix(Expr::Literal(Value::Integer(-i)));
            }
            return Ok(Expr::Unary(UnaryOp::Neg, Box::new(self.unary()?)));
        }
        if self.eat(&Tok::Plus) {
            return Ok(Expr::Unary(UnaryOp::Pos, Box::new(self.unary()?)));
        }
        let atom = self.atom()?;
        self.postfix(atom)
    }

    fn postfix(&mut self, mut e: Expr) -> Result<Expr> {
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.advance();
                    e = Expr::Property(Box::new(e), self.ident("a property name")?);
                }
                Tok::LBracket => {
                    self.advance();
                    if matches!(self.peek(), Tok::Dot) {
                        return Err(self.error("list slicing is not supported yet"));
                    }
                    let idx = self.expr()?;
                    if matches!(self.peek(), Tok::Dot) {
                        return Err(self.error("list slicing is not supported yet"));
                    }
                    self.expect(&Tok::RBracket, "`]`")?;
                    e = Expr::Index(Box::new(e), Box::new(idx));
                }
                Tok::Colon => {
                    e = Expr::HasLabels(Box::new(e), self.labels()?);
                }
                _ => return Ok(e),
            }
        }
    }

    fn map_literal(&mut self) -> Result<Expr> {
        self.expect(&Tok::LBrace, "`{`")?;
        let mut entries = Vec::new();
        if !self.eat(&Tok::RBrace) {
            loop {
                let key = match self.peek().clone() {
                    Tok::Str(s) => {
                        self.advance();
                        s
                    }
                    _ => self.ident("a map key")?,
                };
                self.expect(&Tok::Colon, "`:` after map key")?;
                entries.push((key, self.expr()?));
                if self.eat(&Tok::RBrace) {
                    break;
                }
                self.expect(&Tok::Comma, "`,` or `}`")?;
            }
        }
        Ok(Expr::Map(entries))
    }

    fn atom(&mut self) -> Result<Expr> {
        match self.peek().clone() {
            Tok::Int(i) => {
                self.advance();
                Ok(Expr::Literal(Value::Integer(i)))
            }
            Tok::Float(f) => {
                self.advance();
                Ok(Expr::Literal(Value::Float(f)))
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Expr::Literal(Value::String(s)))
            }
            Tok::Param(p) => {
                self.advance();
                Ok(Expr::Parameter(p))
            }
            Tok::LParen => {
                self.advance();
                let e = self.expr()?;
                self.expect(&Tok::RParen, "`)`")?;
                Ok(e)
            }
            Tok::LBracket => {
                self.advance();
                let mut items = Vec::new();
                if !self.eat(&Tok::RBracket) {
                    loop {
                        items.push(self.expr()?);
                        if self.eat(&Tok::RBracket) {
                            break;
                        }
                        self.expect(&Tok::Comma, "`,` or `]`")?;
                    }
                }
                Ok(Expr::List(items))
            }
            Tok::LBrace => self.map_literal(),
            Tok::QuotedIdent(s) => {
                self.advance();
                Ok(Expr::Variable(s))
            }
            Tok::Ident(s) => {
                let followed_by_paren = matches!(self.peek_at(1), Tok::LParen);
                if !followed_by_paren {
                    if s.eq_ignore_ascii_case("true") {
                        self.advance();
                        return Ok(Expr::Literal(Value::Boolean(true)));
                    }
                    if s.eq_ignore_ascii_case("false") {
                        self.advance();
                        return Ok(Expr::Literal(Value::Boolean(false)));
                    }
                    if s.eq_ignore_ascii_case("null") {
                        self.advance();
                        return Ok(Expr::Literal(Value::Null));
                    }
                    if s.eq_ignore_ascii_case("case") {
                        self.advance();
                        return self.case_expr();
                    }
                    self.advance();
                    return Ok(Expr::Variable(s));
                }
                self.advance();
                self.advance(); // (
                if s.eq_ignore_ascii_case("count") && matches!(self.peek(), Tok::Star) {
                    self.advance();
                    self.expect(&Tok::RParen, "`)`")?;
                    return Ok(Expr::CountStar);
                }
                let distinct = self.eat_kw("DISTINCT");
                let mut args = Vec::new();
                if !self.eat(&Tok::RParen) {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(&Tok::RParen) {
                            break;
                        }
                        self.expect(&Tok::Comma, "`,` or `)`")?;
                    }
                }
                Ok(Expr::FunctionCall { name: s, args, distinct })
            }
            _ => Err(self.error("expected an expression")),
        }
    }

    fn case_expr(&mut self) -> Result<Expr> {
        let operand = if self.is_kw("WHEN") { None } else { Some(Box::new(self.expr()?)) };
        let mut branches = Vec::new();
        while self.eat_kw("WHEN") {
            let when = self.expr()?;
            self.expect_kw("THEN")?;
            branches.push((when, self.expr()?));
        }
        if branches.is_empty() {
            return Err(self.error("expected `WHEN`"));
        }
        let default = if self.eat_kw("ELSE") { Some(Box::new(self.expr()?)) } else { None };
        self.expect_kw("END")?;
        Ok(Expr::Case { operand, branches, default })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(src: &str) -> Statement {
        let mut s = parse(src).unwrap();
        assert_eq!(s.len(), 1);
        s.remove(0)
    }

    #[test]
    fn parses_create_and_match() {
        let stmts = parse(
            "CREATE (a:User {name: \"Alice\"}) CREATE (b:User {name: 'Bob'}) CREATE (a)-[:KNOWS]->(b);\n\
             MATCH (u:User)-[:KNOWS]->(f) RETURN u.name, f.name;",
        )
        .unwrap();
        assert_eq!(stmts.len(), 2);
        let Statement::Query(q) = &stmts[1] else { panic!() };
        let Clause::Return(p) = &q.clauses[1] else { panic!() };
        assert_eq!(p.items[0].name, "u.name");
        let Clause::Match { patterns, .. } = &q.clauses[0] else { panic!() };
        assert_eq!(patterns[0].steps[0].0.direction, Direction::Outgoing);
    }

    #[test]
    fn precedence() {
        let Statement::Query(q) = one("RETURN 1 + 2 * 3 = 7 AND NOT false AS x") else { panic!() };
        let Clause::Return(p) = &q.clauses[0] else { panic!() };
        assert!(matches!(&p.items[0].expr, Expr::Binary(BinaryOp::And, _, _)));
        assert_eq!(p.items[0].name, "x");
    }

    #[test]
    fn admin_commands() {
        assert_eq!(
            one("CREATE DATABASE IF NOT EXISTS shop"),
            Statement::Admin(AdminCommand::CreateDatabase { name: "shop".into(), if_not_exists: true })
        );
        assert_eq!(one("use schema products"), Statement::Admin(AdminCommand::UseSchema("products".into())));
    }

    #[test]
    fn errors_carry_positions() {
        let err = parse("MATCH (n)\nRETURN n.").unwrap_err();
        assert!(matches!(err, MyGraphError::Syntax { line: 2, column: 10, .. }), "{err}");
        assert!(parse("MATCH (n)").is_err());
        assert!(parse("WITH 1 + 1 RETURN 1").is_err());
    }
}
