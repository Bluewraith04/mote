//! Stage boundary types: each stage hands the next one plain data.

use std::collections::HashMap;

use crate::ast::{Expr, Program};
use crate::span::Span;
use crate::types::Type;

/// The resolved type of every expression the checker visited; the only way later stages ask "what is this node's type".
#[derive(Default)]
pub struct TypeTable {
    by_node: HashMap<Span, Type>,
    instance_calls: HashMap<Span, String>,
    instances: HashMap<String, TypeTable>,
    type_args: HashMap<Span, Vec<Type>>,
    variadic_lists: HashMap<Span, Type>,
    any_checks: HashMap<Span, Type>,
    type_tests: HashMap<Span, Type>,
    rewraps: HashMap<Span, i8>,
}

impl TypeTable {
    /// Adds the target of each `is` test.
    pub(crate) fn with_type_tests(mut self, type_tests: HashMap<Span, Type>) -> Self {
        self.type_tests = type_tests;
        self
    }

    /// The type the `is` test at `span` compares against.
    pub(crate) fn type_test(&self, span: Span) -> Option<&Type> {
        self.type_tests.get(&span)
    }

    /// Adds the values wrapped in or unwrapped from `Some` at run time.
    pub(crate) fn with_rewraps(mut self, rewraps: HashMap<Span, i8>) -> Self {
        self.rewraps = rewraps;
        self
    }

    /// `Some` wraps (positive) or unwraps (negative) of the value at `span`.
    pub(crate) fn rewrap(&self, span: Span) -> i8 {
        self.rewraps.get(&span).copied().unwrap_or(0)
    }

    /// Adds the `Any` values checked at run time.
    pub(crate) fn with_any_checks(mut self, any_checks: HashMap<Span, Type>) -> Self {
        self.any_checks = any_checks;
        self
    }

    /// The type the `Any` value of the expression at `span` must fit.
    pub(crate) fn any_check(&self, span: Span) -> Option<&Type> {
        self.any_checks.get(&span)
    }

    /// Adds each call's type arguments and variadic list type.
    pub(crate) fn with_calls(mut self, type_args: HashMap<Span, Vec<Type>>, variadic_lists: HashMap<Span, Type>) -> Self {
        self.type_args = type_args;
        self.variadic_lists = variadic_lists;
        self
    }

    /// The type arguments the call at `span` passes to its generic callee.
    pub(crate) fn type_args(&self, span: Span) -> Option<&[Type]> {
        self.type_args.get(&span).map(Vec::as_slice)
    }

    /// The type of the list the variadic call at `span` packs.
    pub(crate) fn variadic_list(&self, span: Span) -> Option<&Type> {
        self.variadic_lists.get(&span)
    }

    pub fn new(by_node: HashMap<Span, Type>) -> Self {
        Self { by_node, ..Self::default() }
    }

    /// A table that also records the instance each call resolves to.
    pub(crate) fn with_instance_calls(by_node: HashMap<Span, Type>, instance_calls: HashMap<Span, String>) -> Self {
        Self { by_node, instance_calls, ..Self::default() }
    }

    pub(crate) fn add_instance(&mut self, name: String, table: TypeTable) {
        self.instances.insert(name, table);
    }

    /// Takes the table of instance `name`, if it is one.
    pub(crate) fn take_instance(&mut self, name: &str) -> Option<TypeTable> {
        self.instances.remove(name)
    }

    /// The instance name the call at `span` resolves to, if it calls a bounded generic.
    pub(crate) fn instance_call(&self, span: Span) -> Option<&str> {
        self.instance_calls.get(&span).map(String::as_str)
    }

    /// The checked type of `expr`, if the checker visited it.
    pub fn of(&self, expr: &Expr) -> Option<&Type> {
        self.by_node.get(&expr.span())
    }

    /// The checked type of the expression at `span`, if the checker visited it.
    pub fn at(&self, span: Span) -> Option<&Type> {
        self.by_node.get(&span)
    }

    pub fn is_empty(&self) -> bool {
        self.by_node.is_empty()
    }

    /// Sorted `line:col+len  Type` lines, for golden tests.
    pub fn dump(&self) -> String {
        let mut rows: Vec<(usize, usize, usize, String)> =
            self.by_node.iter().map(|(s, t)| (s.line, s.col, s.end - s.start, format!("{t:?}"))).collect();
        rows.sort();
        rows.dedup();
        rows.iter().map(|(line, col, len, ty)| format!("{line}:{col}+{len}  {ty}\n")).collect()
    }
}

/// The checker's output: annotated modules in dependency order (the entry module last) and every expression's type.
pub struct Checked {
    pub modules: Vec<Program>,
    pub types: TypeTable,
}

impl Checked {
    /// The type table as sorted `line:col+len  Type` lines, for golden tests.
    pub fn dump_types(&self) -> String {
        self.types.dump()
    }
}
