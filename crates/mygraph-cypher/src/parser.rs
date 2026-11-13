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
        if self.is_kw("CREATE")
            && (self.is_kw_at(1, "INDEX") || (self.is_kw_at(1, "RANGE") && self.is_kw_at(2, "INDEX")))
        {
            return self.create_index();
        }
        if self.is_kw("CREATE") && self.is_kw_at(1, "CONSTRAINT") {
            return self.create_constraint();
        }
        if self.is_kw("DROP") && self.is_kw_at(1, "CONSTRAINT") {
            self.advance();
            self.advance();
            let name = self.ident("a constraint name")?;
            let if_exists = if self.eat_kw("IF") {
                self.expect_kw("EXISTS")?;
                true
            } else {
                false
            };
            return Ok(Statement::Admin(AdminCommand::DropConstraint { name, if_exists }));
        }
        if self.is_kw("DROP") && self.is_kw_at(1, "INDEX") {
            self.advance();
            self.advance();
            let target = if self.eat_kw("ON") {
                let (label, key) = self.memgraph_index_target()?;
                IndexTarget::On { label, key }
            } else {
                IndexTarget::Name(self.ident("an index name or `ON`")?)
            };
            let if_exists = if self.eat_kw("IF") {
                self.expect_kw("EXISTS")?;
                true
            } else {
                false
            };
            return Ok(Statement::Admin(AdminCommand::DropIndex { target, if_exists }));
        }
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
                return Err(self.error("expected `DATABASE`, `SCHEMA` or `INDEX`"));
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
            if self.eat_kw("INDEXES") || self.eat_kw("INDEX") {
                return Ok(Statement::Admin(AdminCommand::ShowIndexes));
            }
            if self.eat_kw("CONSTRAINTS") || self.eat_kw("CONSTRAINT") {
                return Ok(Statement::Admin(AdminCommand::ShowConstraints));
            }
            return Err(self.error("expected `DATABASES`, `SCHEMAS`, `INDEXES` or `CONSTRAINTS`"));
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

    /// `CREATE [RANGE] INDEX [name] [IF NOT EXISTS] FOR (n:Label) ON (n.key)` (Neo4j)
    /// or `CREATE INDEX ON :Label(key)` (Memgraph).
    fn create_index(&mut self) -> Result<Statement> {
        self.expect_kw("CREATE")?;
        self.eat_kw("RANGE");
        self.expect_kw("INDEX")?;
        if self.eat_kw("ON") {
            let (label, key) = self.memgraph_index_target()?;
            // Memgraph treats an existing index as success
            return Ok(Statement::Admin(AdminCommand::CreateIndex { name: None, label, key, if_not_exists: true }));
        }
        let name = if self.is_kw("IF") || self.is_kw("FOR") { None } else { Some(self.ident("an index name")?) };
        let if_not_exists = if self.eat_kw("IF") {
            self.expect_kw("NOT")?;
            self.expect_kw("EXISTS")?;
            true
        } else {
            false
        };
        self.expect_kw("FOR")?;
        self.expect(&Tok::LParen, "`(`")?;
        let var = self.ident("a variable")?;
        self.expect(&Tok::Colon, "`:`")?;
        let label = self.ident("a label")?;
        self.expect(&Tok::RParen, "`)`")?;
        self.expect_kw("ON")?;
        self.expect(&Tok::LParen, "`(`")?;
        let v2 = self.ident("a variable")?;
        if v2 != var {
            return Err(self.error(format!("expected variable `{var}`")));
        }
        self.expect(&Tok::Dot, "`.`")?;
        let key = self.ident("a property name")?;
        if self.peek() == &Tok::Comma {
            return Err(self.error("composite indexes are not supported yet"));
        }
        self.expect(&Tok::RParen, "`)`")?;
        Ok(Statement::Admin(AdminCommand::CreateIndex { name, label, key, if_not_exists }))
    }

    /// `CREATE CONSTRAINT [name] [IF NOT EXISTS] FOR (n:L) REQUIRE n.k IS UNIQUE | IS NOT NULL` (Neo4j)
    /// or `CREATE CONSTRAINT ON (n:L) ASSERT n.k IS UNIQUE | ASSERT EXISTS (n.k)` (Memgraph).
    fn create_constraint(&mut self) -> Result<Statement> {
        self.expect_kw("CREATE")?;
        self.expect_kw("CONSTRAINT")?;
        let memgraph = self.is_kw("ON") && self.peek_at(1) == &Tok::LParen;
        let (name, if_not_exists) = if memgraph {
            // Memgraph treats an existing constraint as success
            (None, true)
        } else {
            let name =
                if self.is_kw("IF") || self.is_kw("FOR") { None } else { Some(self.ident("a constraint name")?) };
            let if_not_exists = if self.eat_kw("IF") {
                self.expect_kw("NOT")?;
                self.expect_kw("EXISTS")?;
                true
            } else {
                false
            };
            (name, if_not_exists)
        };
        self.expect_kw(if memgraph { "ON" } else { "FOR" })?;
        self.expect(&Tok::LParen, "`(`")?;
        let var = self.ident("a variable")?;
        self.expect(&Tok::Colon, "`:`")?;
        let label = self.ident("a label")?;
        self.expect(&Tok::RParen, "`)`")?;
        self.expect_kw(if memgraph { "ASSERT" } else { "REQUIRE" })?;
        if memgraph && self.eat_kw("EXISTS") {
            self.expect(&Tok::LParen, "`(`")?;
            let key = self.constraint_property(&var)?;
            self.expect(&Tok::RParen, "`)`")?;
            return Ok(Statement::Admin(AdminCommand::CreateConstraint {
                name,
                label,
                key,
                kind: ConstraintKind::NotNull,
                if_not_exists,
            }));
        }
        let parens = self.eat(&Tok::LParen);
        let key = self.constraint_property(&var)?;
        if self.peek() == &Tok::Comma {
            return Err(self.error("composite constraints are not supported yet"));
        }
        if parens {
            self.expect(&Tok::RParen, "`)`")?;
        }
        self.expect_kw("IS")?;
        let kind = if self.eat_kw("UNIQUE") {
            ConstraintKind::Unique
        } else if self.eat_kw("NOT") {
            self.expect_kw("NULL")?;
            ConstraintKind::NotNull
        } else {
            return Err(self.error("expected `UNIQUE` or `NOT NULL`"));
        };
        Ok(Statement::Admin(AdminCommand::CreateConstraint { name, label, key, kind, if_not_exists }))
    }

    /// `var.key`, where `var` must be the constraint's variable.
    fn constraint_property(&mut self, var: &str) -> Result<String> {
        let v = self.ident("a variable")?;
        if v != var {
            return Err(self.error(format!("expected variable `{var}`")));
        }
        self.expect(&Tok::Dot, "`.`")?;
        self.ident("a property name")
    }

    /// `:Label(key)`
    fn memgraph_index_target(&mut self) -> Result<(String, String)> {
        self.expect(&Tok::Colon, "`:`")?;
        let label = self.ident("a label")?;
        self.expect(&Tok::LParen, "`(`")?;
        let key = self.ident("a property name")?;
        self.expect(&Tok::RParen, "`)`")?;
        Ok((label, key))
    }

    fn query(&mut self) -> Result<Query> {
        let mut parts = vec![self.single_query()?];
        let mut union_all: Option<bool> = None;
        while self.is_kw("UNION") {
            if !matches!(parts.last().and_then(|p| p.clauses.last()), Some(Clause::Return(_))) {
                return Err(self.error("each part of a UNION must end with RETURN"));
            }
            self.advance();
            let all = self.eat_kw("ALL");
            if union_all.is_some_and(|a| a != all) {
                return Err(self.error("UNION and UNION ALL cannot be mixed in one query"));
            }
            union_all = Some(all);
            let part = self.single_query()?;
            if !matches!(part.clauses.last(), Some(Clause::Return(_))) {
                return Err(self.error("each part of a UNION must end with RETURN"));
            }
            parts.push(part);
        }
        Ok(Query { parts, union_all: union_all.unwrap_or(false) })
    }

    fn single_query(&mut self) -> Result<SingleQuery> {
        let mut clauses = Vec::new();
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) && !self.is_kw("UNION") {
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
            Some(Clause::Call { .. }) if clauses.len() > 1 => {
                Err(self.error("a CALL inside a query must be followed by RETURN or an updating clause"))
            }
            _ => Ok(SingleQuery { clauses }),
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
            let pattern = self.path()?;
            if self.peek() == &Tok::Comma {
                return Err(self.error("MERGE takes a single pattern; use one MERGE per pattern"));
            }
            let (mut on_create, mut on_match) = (Vec::new(), Vec::new());
            while self.eat_kw("ON") {
                let target = if self.eat_kw("CREATE") {
                    &mut on_create
                } else if self.eat_kw("MATCH") {
                    &mut on_match
                } else {
                    return Err(self.error("expected `CREATE` or `MATCH` after `ON`"));
                };
                self.expect_kw("SET")?;
                let items = self.set_items()?;
                target.extend(items);
            }
            return Ok(Clause::Merge { pattern, on_create, on_match });
        }
        if self.eat_kw("SET") {
            return Ok(Clause::Set { items: self.set_items()? });
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
        if self.eat_kw("CALL") {
            return self.call();
        }
        Err(self.error("expected a clause (MATCH, CREATE, MERGE, SET, REMOVE, DELETE, UNWIND, WITH, CALL or RETURN)"))
    }

    /// After `CALL`: `ns.name(args) [YIELD a [AS b], … | YIELD *] [WHERE expr]`.
    fn call(&mut self) -> Result<Clause> {
        if self.peek() == &Tok::LBrace {
            return Err(self.error("CALL subqueries are not supported yet"));
        }
        let mut procedure = self.ident("a procedure name")?;
        while self.eat(&Tok::Dot) {
            procedure.push('.');
            procedure.push_str(&self.ident("a procedure name")?);
        }
        self.expect(&Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        if !self.eat(&Tok::RParen) {
            args.push(self.expr()?);
            while self.eat(&Tok::Comma) {
                args.push(self.expr()?);
            }
            self.expect(&Tok::RParen, "`)`")?;
        }
        let mut yields = None;
        let mut filter = None;
        if self.eat_kw("YIELD") {
            if !self.eat(&Tok::Star) {
                let mut items = Vec::new();
                loop {
                    let name = self.ident("an output column")?;
                    let alias = if self.eat_kw("AS") { Some(self.ident("a variable")?) } else { None };
                    items.push((name, alias));
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                yields = Some(items);
            }
            if self.eat_kw("WHERE") {
                filter = Some(self.expr()?);
            }
        }
        Ok(Clause::Call { procedure, args, yields, filter })
    }

    fn set_items(&mut self) -> Result<Vec<SetItem>> {
        let mut items = vec![self.set_item()?];
        while self.eat(&Tok::Comma) {
            items.push(self.set_item()?);
        }
        Ok(items)
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
        let mut name = None;
        if matches!(self.peek(), Tok::Ident(_) | Tok::QuotedIdent(_)) && matches!(self.peek_at(1), Tok::Eq) {
            name = Some(self.ident("a path name")?);
            self.advance();
        }
        let shortest = if self.is_kw("shortestPath") && matches!(self.peek_at(1), Tok::LParen) {
            Some(Shortest::One)
        } else if self.is_kw("allShortestPaths") && matches!(self.peek_at(1), Tok::LParen) {
            Some(Shortest::All)
        } else {
            None
        };
        if shortest.is_some() {
            self.advance();
            self.advance();
        }
        let start = self.node_pattern()?;
        let mut steps = Vec::new();
        while matches!(self.peek(), Tok::Minus | Tok::Lt) {
            let rel = self.rel_pattern()?;
            let node = self.node_pattern()?;
            steps.push((rel, node));
        }
        let mut weight = None;
        if shortest.is_some() {
            if self.eat(&Tok::Comma) {
                weight = Some(self.shortest_options()?);
            }
            self.expect(&Tok::RParen, "`)` to close shortestPath")?;
            if steps.len() != 1 {
                return Err(self.error("shortestPath needs a pattern with exactly one relationship"));
            }
            if steps[0].0.length.is_some_and(|l| l.min > 1) {
                return Err(self.error("shortestPath supports a minimum length of 0 or 1 only"));
            }
            if weight.is_some() {
                if shortest == Some(Shortest::All) {
                    return Err(self.error("allShortestPaths does not support a weight yet"));
                }
                if steps[0].0.length.is_none_or(|l| l.max.is_some()) {
                    return Err(self.error("a weighted shortestPath needs a relationship of any length, like `-[*]->`"));
                }
            }
        }
        Ok(PathPattern { name, shortest, weight, start, steps })
    }

    /// `{weight: 'property'}` after the pattern of a shortest path. Returns the
    /// property name.
    fn shortest_options(&mut self) -> Result<String> {
        let Expr::Map(entries) = self.map_literal()? else { unreachable!("map_literal returns a map") };
        let mut weight = None;
        for (key, value) in entries {
            match (key.as_str(), value) {
                ("weight", Expr::Literal(Value::String(p))) => weight = Some(p),
                ("weight", _) => return Err(self.error("the shortestPath `weight` must be a property name string")),
                (other, _) => return Err(self.error(format!("unknown shortestPath option `{other}`"))),
            }
        }
        weight.ok_or_else(|| self.error("shortestPath options need a `weight`"))
    }

    /// After `*`: `n`, `n..`, `..m`, `n..m` or nothing.
    fn var_length(&mut self) -> Result<VarLength> {
        let int = |p: &mut Self| -> Result<Option<u32>> {
            match p.peek().clone() {
                Tok::Int(i) => {
                    p.advance();
                    u32::try_from(i).map(Some).map_err(|_| p.error("relationship length out of range"))
                }
                _ => Ok(None),
            }
        };
        let first = int(self)?;
        if self.peek() == &Tok::Dot && self.peek_at(1) == &Tok::Dot {
            self.advance();
            self.advance();
            let max = int(self)?;
            let min = first.unwrap_or(1);
            if max.is_some_and(|m| m < min) {
                return Err(self.error("the maximum length is smaller than the minimum"));
            }
            return Ok(VarLength { min, max });
        }
        Ok(match first {
            Some(n) => VarLength { min: n, max: Some(n) },
            None => VarLength { min: 1, max: None },
        })
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
        let (mut variable, mut types, mut properties, mut length) = (None, Vec::new(), None, None);
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
            if self.eat(&Tok::Star) {
                length = Some(self.var_length()?);
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
        Ok(RelPattern { variable, types, properties, direction, length })
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
            } else if self.eat(&Tok::RegexMatch) {
                BinaryOp::RegexMatch
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
        let Clause::Return(p) = &q.parts[0].clauses[1] else { panic!() };
        assert_eq!(p.items[0].name, "u.name");
        let Clause::Match { patterns, .. } = &q.parts[0].clauses[0] else { panic!() };
        assert_eq!(patterns[0].steps[0].0.direction, Direction::Outgoing);
    }

    #[test]
    fn precedence() {
        let Statement::Query(q) = one("RETURN 1 + 2 * 3 = 7 AND NOT false AS x") else { panic!() };
        let Clause::Return(p) = &q.parts[0].clauses[0] else { panic!() };
        assert!(matches!(&p.items[0].expr, Expr::Binary(BinaryOp::And, _, _)));
        assert_eq!(p.items[0].name, "x");
    }

    #[test]
    fn regex_binds_tighter_than_and() {
        let Statement::Query(q) = one("RETURN 'ab' =~ 'a.' AND true AS x") else { panic!() };
        let Clause::Return(p) = &q.parts[0].clauses[0] else { panic!() };
        let Expr::Binary(BinaryOp::And, l, _) = &p.items[0].expr else { panic!() };
        assert!(matches!(**l, Expr::Binary(BinaryOp::RegexMatch, _, _)));
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
    fn constraint_commands() {
        let unique = |name: Option<&str>, if_not_exists| {
            Statement::Admin(AdminCommand::CreateConstraint {
                name: name.map(str::to_string),
                label: "User".into(),
                key: "email".into(),
                kind: ConstraintKind::Unique,
                if_not_exists,
            })
        };
        assert_eq!(
            one("CREATE CONSTRAINT u IF NOT EXISTS FOR (n:User) REQUIRE n.email IS UNIQUE"),
            unique(Some("u"), true)
        );
        assert_eq!(one("CREATE CONSTRAINT FOR (n:User) REQUIRE (n.email) IS UNIQUE"), unique(None, false));
        assert_eq!(one("CREATE CONSTRAINT ON (n:User) ASSERT n.email IS UNIQUE"), unique(None, true));
        let Statement::Admin(AdminCommand::CreateConstraint { kind, .. }) =
            one("CREATE CONSTRAINT ON (n:User) ASSERT EXISTS (n.email)")
        else {
            panic!()
        };
        assert_eq!(kind, ConstraintKind::NotNull);
        let Statement::Admin(AdminCommand::CreateConstraint { kind, .. }) =
            one("CREATE CONSTRAINT FOR (n:User) REQUIRE n.email IS NOT NULL")
        else {
            panic!()
        };
        assert_eq!(kind, ConstraintKind::NotNull);
        assert_eq!(
            one("DROP CONSTRAINT u IF EXISTS"),
            Statement::Admin(AdminCommand::DropConstraint { name: "u".into(), if_exists: true })
        );
        assert_eq!(one("SHOW CONSTRAINTS"), Statement::Admin(AdminCommand::ShowConstraints));
        assert!(parse("CREATE CONSTRAINT FOR (n:User) REQUIRE m.email IS UNIQUE").is_err());
        assert!(parse("CREATE CONSTRAINT FOR (n:User) REQUIRE (n.a, n.b) IS UNIQUE").is_err());
    }

    #[test]
    fn errors_carry_positions() {
        let err = parse("MATCH (n)\nRETURN n.").unwrap_err();
        assert!(matches!(err, MyGraphError::Syntax { line: 2, column: 10, .. }), "{err}");
        assert!(parse("MATCH (n)").is_err());
        assert!(parse("WITH 1 + 1 RETURN 1").is_err());
    }
}
