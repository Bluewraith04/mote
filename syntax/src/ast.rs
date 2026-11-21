mod expr;
mod item;
mod pattern;
mod stmt;
mod ty;

#[cfg(test)]
pub(crate) mod dump;

use std::ops::{Index, IndexMut};

use mote_base::{Arena, Id, Span, Symbol};

pub use expr::*;
pub use item::*;
pub use pattern::*;
pub use stmt::*;
pub use ty::*;

/// The syntax tree of one source file.
#[derive(Default)]
pub struct Ast {
    pub root: Vec<Id<Item>>,
    pub items: Arena<Item>,
    pub exprs: Arena<Expr>,
    pub stmts: Arena<Stmt>,
    pub patterns: Arena<Pattern>,
    pub types: Arena<Type>,
}

macro_rules! indexed {
    ($($field:ident: $node:ty),* $(,)?) => {$(
        impl Index<Id<$node>> for Ast {
            type Output = $node;

            fn index(&self, id: Id<$node>) -> &$node {
                &self.$field[id]
            }
        }

        impl IndexMut<Id<$node>> for Ast {
            fn index_mut(&mut self, id: Id<$node>) -> &mut $node {
                &mut self.$field[id]
            }
        }
    )*};
}

indexed!(items: Item, exprs: Expr, stmts: Stmt, patterns: Pattern, types: Type);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: Symbol,
    pub span: Span,
}

/// A dotted name such as `std.json.Value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    pub segments: Vec<Ident>,
    pub span: Span,
}

/// A braced list of statements.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Id<Stmt>>,
    pub span: Span,
}
