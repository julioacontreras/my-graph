//! Built-in procedures, run with `CALL name(args) [YIELD …]`.
//!
//! Every built-in procedure only reads, so a query with `CALL` can still run
//! as a read-only query.

use mygraph_core::Result;

use super::eval::{Env, Val};

/// Runs a procedure: arguments in, output records out.
type Run = fn(&Env<'_>, &[Val]) -> Result<Vec<Vec<Val>>>;

pub(crate) struct Procedure {
    pub name: &'static str,
    pub signature: &'static str,
    pub description: &'static str,
    /// Number of arguments.
    pub arity: usize,
    /// Output column names, in order.
    pub outputs: &'static [&'static str],
    run: Run,
}

impl Procedure {
    pub fn run(&self, env: &Env<'_>, args: &[Val]) -> Result<Vec<Vec<Val>>> {
        (self.run)(env, args)
    }
}

fn strings(names: Vec<String>) -> Vec<Vec<Val>> {
    names.into_iter().map(|n| vec![Val::Str(n)]).collect()
}

pub(crate) const PROCEDURES: &[Procedure] = &[
    Procedure {
        name: "db.constraints",
        signature: "db.constraints() :: (name, type, label, property)",
        description: "Lists the constraints of the current schema.",
        arity: 0,
        outputs: &["name", "type", "label", "property"],
        run: |env, _| {
            Ok(env
                .graph
                .constraints()
                .into_iter()
                .map(|c| vec![Val::Str(c.name), Val::Str(c.kind.as_str().into()), Val::Str(c.label), Val::Str(c.key)])
                .collect())
        },
    },
    Procedure {
        name: "db.indexes",
        signature: "db.indexes() :: (name, label, property, entries)",
        description: "Lists the property indexes of the current schema.",
        arity: 0,
        outputs: &["name", "label", "property", "entries"],
        run: |env, _| {
            Ok(env
                .graph
                .indexes()
                .into_iter()
                .map(|i| vec![Val::Str(i.name), Val::Str(i.label), Val::Str(i.key), Val::Int(i.entries as i64)])
                .collect())
        },
    },
    Procedure {
        name: "db.labels",
        signature: "db.labels() :: (label)",
        description: "Lists the labels that at least one node carries.",
        arity: 0,
        outputs: &["label"],
        run: |env, _| Ok(strings(env.graph.label_names())),
    },
    Procedure {
        name: "db.propertyKeys",
        signature: "db.propertyKeys() :: (propertyKey)",
        description: "Lists the property keys used in the current schema.",
        arity: 0,
        outputs: &["propertyKey"],
        run: |env, _| Ok(strings(env.graph.property_keys())),
    },
    Procedure {
        name: "db.relationshipTypes",
        signature: "db.relationshipTypes() :: (relationshipType)",
        description: "Lists the relationship types that at least one relationship has.",
        arity: 0,
        outputs: &["relationshipType"],
        run: |env, _| Ok(strings(env.graph.rel_type_names())),
    },
    Procedure {
        name: "dbms.procedures",
        signature: "dbms.procedures() :: (name, signature, description)",
        description: "Lists the available procedures.",
        arity: 0,
        outputs: &["name", "signature", "description"],
        run: |_, _| {
            Ok(PROCEDURES
                .iter()
                .map(|p| vec![Val::Str(p.name.into()), Val::Str(p.signature.into()), Val::Str(p.description.into())])
                .collect())
        },
    },
];

/// The procedure called `name` (case-insensitive).
pub(crate) fn lookup(name: &str) -> Option<&'static Procedure> {
    PROCEDURES.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}
