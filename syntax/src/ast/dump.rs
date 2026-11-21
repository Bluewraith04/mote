use mote_base::{Id, Interner, Symbol};

use super::*;
use crate::token::{FloatSuffix, IntSuffix};

/// Prints a tree as short S-expressions without spans, for parser tests.
pub(crate) struct Dump<'a> {
    pub ast: &'a Ast,
    pub names: &'a Interner,
}

fn join(parts: impl IntoIterator<Item = String>) -> String {
    parts.into_iter().collect::<Vec<_>>().join(" ")
}

impl Dump<'_> {
    pub fn root(&self) -> String {
        self.ast.root.iter().map(|&id| self.item(id)).collect::<Vec<_>>().join("\n")
    }

    fn name(&self, symbol: Symbol) -> &str {
        self.names.resolve(symbol)
    }

    fn path(&self, path: &Path) -> String {
        path.segments.iter().map(|s| self.name(s.name)).collect::<Vec<_>>().join(".")
    }

    fn module_path(&self, path: &ModulePath) -> String {
        let prefix: String = path
            .prefix
            .iter()
            .map(|p| match p {
                PathPrefix::Dot => ".",
                PathPrefix::DotDot => "..",
                PathPrefix::Super => "super.",
            })
            .collect();
        let segments = path.segments.iter().map(|s| self.name(s.name)).collect::<Vec<_>>().join(".");
        format!("{prefix}{segments}")
    }

    pub fn item(&self, id: Id<Item>) -> String {
        let item = &self.ast[id];
        let mut out = String::new();
        for attr in &item.attrs {
            out += &format!("{} ", self.attribute(attr));
        }
        if item.is_pub {
            out += "pub ";
        }
        out += &match &item.kind {
            ItemKind::Function(f) => self.function(f),
            ItemKind::NativeFunction(f) => format!(
                "(native {}{} {}{})",
                self.name(f.name.name),
                self.generics(&f.generics),
                self.params(&f.params),
                self.ret(f.ret)
            ),
            ItemKind::Test(t) => format!("(test {:?} {})", self.name(t.title), self.block(&t.body)),
            ItemKind::Record(r) => self.record(r),
            ItemKind::Enum(e) => self.enum_decl(e),
            ItemKind::Trait(t) => self.trait_decl(t),
            ItemKind::TypeAlias(a) => {
                format!("(type {}{} = {})", self.name(a.name.name), self.generics(&a.generics), self.ty(a.target))
            }
            ItemKind::Import(i) => self.import(i),
            ItemKind::Stmt(s) => self.stmt(*s),
            ItemKind::Error => "(error)".into(),
        };
        out
    }

    fn attribute(&self, attr: &Attribute) -> String {
        let args: Vec<String> = attr
            .args
            .iter()
            .map(|a| {
                let value = match &a.value {
                    AttrValue::Path(p) => self.path(p),
                    AttrValue::String(s) => format!("{:?}", self.name(*s)),
                    AttrValue::Int(i) => self.int(i),
                    AttrValue::Float(f) => self.float(f),
                    AttrValue::Bool(b) => b.to_string(),
                };
                match &a.key {
                    Some(key) => format!("{}={value}", self.path(key)),
                    None => value,
                }
            })
            .collect();
        if args.is_empty() { format!("@{}", self.name(attr.name.name)) } else { format!("@{}({})", self.name(attr.name.name), args.join(", ")) }
    }

    fn generics(&self, generics: &[GenericParam]) -> String {
        if generics.is_empty() {
            return String::new();
        }
        let params: Vec<String> = generics
            .iter()
            .map(|g| {
                let name = self.name(g.name.name);
                if g.bounds.is_empty() {
                    name.to_string()
                } else {
                    format!("{name}: {}", g.bounds.iter().map(|&b| self.ty(b)).collect::<Vec<_>>().join(" + "))
                }
            })
            .collect();
        format!("<{}>", params.join(", "))
    }

    fn params(&self, params: &[Param]) -> String {
        let parts = params.iter().map(|p| match &p.kind {
            ParamKind::SelfValue { is_var } => if *is_var { "var self".to_string() } else { "self".to_string() },
            ParamKind::Variadic { name, ty } => format!("...{}{}", self.name(name.name), self.annotation(*ty)),
            ParamKind::Named { is_var, name, ty, default } => format!(
                "{}{}{}{}",
                if *is_var { "var " } else { "" },
                self.name(name.name),
                self.annotation(*ty),
                default.map_or(String::new(), |d| format!("={}", self.expr(d)))
            ),
        });
        format!("({})", join(parts))
    }

    fn annotation(&self, ty: Option<Id<Type>>) -> String {
        ty.map_or(String::new(), |t| format!(":{}", self.ty(t)))
    }

    fn ret(&self, ret: Option<Id<Type>>) -> String {
        ret.map_or(String::new(), |t| format!(" -> {}", self.ty(t)))
    }

    fn function(&self, f: &Function) -> String {
        format!(
            "(fn {}{} {}{} {})",
            self.name(f.name.name),
            self.generics(&f.generics),
            self.params(&f.params),
            self.ret(f.ret),
            self.block(&f.body)
        )
    }

    fn method(&self, m: &Method) -> String {
        format!("{}{}", if m.is_pub { "pub " } else { "" }, self.function(&m.function))
    }

    fn field(&self, f: &Field) -> String {
        let binding = match f.binding {
            Some(BindingKind::Let) => "let ",
            Some(BindingKind::Var) => "var ",
            None => "",
        };
        format!(
            "(field {}{}{}:{}{})",
            if f.is_pub { "pub " } else { "" },
            binding,
            self.name(f.name.name),
            self.ty(f.ty),
            f.default.map_or(String::new(), |d| format!(" = {}", self.expr(d)))
        )
    }

    fn record(&self, r: &Record) -> String {
        let kind = match r.kind {
            RecordKind::Struct => "struct",
            RecordKind::Class => "class",
        };
        let members = r.members.iter().map(|m| match m {
            Member::Field(f) => self.field(f),
            Member::Method(m) => self.method(m),
        });
        format!("({kind} {}{}{} {})", self.name(r.name.name), self.generics(&r.generics), self.traits(&r.traits), join(members))
            .replace(" )", ")")
    }

    fn traits(&self, traits: &[Id<Type>]) -> String {
        if traits.is_empty() { String::new() } else { format!(": ({})", traits.iter().map(|&t| self.ty(t)).collect::<Vec<_>>().join(", ")) }
    }

    fn enum_decl(&self, e: &Enum) -> String {
        let variants = e.variants.iter().map(|v| {
            let name = self.name(v.name.name);
            match &v.payload {
                VariantPayload::Unit => name.to_string(),
                VariantPayload::Discriminant(d) => format!("{name}={}", self.expr(*d)),
                VariantPayload::Tuple(types) => format!("{name}({})", types.iter().map(|&t| self.ty(t)).collect::<Vec<_>>().join(", ")),
                VariantPayload::Struct(fields) => format!("{name}{{{}}}", join(fields.iter().map(|f| self.field(f)))),
            }
        });
        let methods = e.methods.iter().map(|m| self.method(m));
        format!("(enum {}{}{} {})", self.name(e.name.name), self.generics(&e.generics), self.traits(&e.traits), join(variants.chain(methods)))
            .replace(" )", ")")
    }

    fn trait_decl(&self, t: &Trait) -> String {
        let members = t.members.iter().map(|m| {
            format!(
                "(fn {}{} {}{}{})",
                self.name(m.name.name),
                self.generics(&m.generics),
                self.params(&m.params),
                self.ret(m.ret),
                m.body.as_ref().map_or(String::new(), |b| format!(" {}", self.block(b)))
            )
        });
        format!("(trait {}{} {})", self.name(t.name.name), self.generics(&t.generics), join(members)).replace(" )", ")")
    }

    fn import(&self, import: &Import) -> String {
        match import {
            Import::Module { path, alias } => {
                format!("(import {}{})", self.module_path(path), alias.map_or(String::new(), |a| format!(" as {}", self.name(a.name))))
            }
            Import::Names { names, from } => {
                let names = names.iter().map(|n| match n.alias {
                    Some(alias) => format!("{} as {}", self.name(n.name.name), self.name(alias.name)),
                    None => self.name(n.name.name).to_string(),
                });
                format!("(import {{{}}} from {})", names.collect::<Vec<_>>().join(", "), self.module_path(from))
            }
            Import::Glob { from } => format!("(import {{*}} from {})", self.module_path(from)),
        }
    }

    pub fn block(&self, block: &Block) -> String {
        if block.stmts.is_empty() { "(block)".into() } else { format!("(block {})", join(block.stmts.iter().map(|&s| self.stmt(s)))) }
    }

    fn condition(&self, condition: &Condition) -> String {
        match condition {
            Condition::Expr(e) => self.expr(*e),
            Condition::Let { pattern, value } => format!("(let {} {})", self.pattern(*pattern), self.expr(*value)),
        }
    }

    pub fn stmt(&self, id: Id<Stmt>) -> String {
        match &self.ast[id].kind {
            StmtKind::Binding(b) => format!(
                "({} {}{} {})",
                if b.kind == BindingKind::Let { "let" } else { "var" },
                self.pattern(b.pattern),
                self.annotation(b.ty),
                self.expr(b.init)
            ),
            StmtKind::Assign { target, op, value } => format!("({} {} {})", assign_op(*op), self.expr(*target), self.expr(*value)),
            StmtKind::If(i) => self.if_stmt(i),
            StmtKind::While { condition, body } => format!("(while {} {})", self.condition(condition), self.block(body)),
            StmtKind::For { pattern, iterable, body } => {
                format!("(for {} {} {})", self.pattern(*pattern), self.expr(*iterable), self.block(body))
            }
            StmtKind::Match(m) => {
                let arms = m.arms.iter().map(|arm| {
                    let guard = arm.guard.map_or(String::new(), |g| format!(" if {}", self.expr(g)));
                    let body = match &arm.body {
                        ArmBody::Block(b) => self.block(b),
                        ArmBody::Stmt(s) => self.stmt(*s),
                    };
                    format!("({}{guard} => {body})", self.pattern(arm.pattern))
                });
                format!("(match {} {})", self.expr(m.subject), join(arms)).replace(" )", ")")
            }
            StmtKind::Scope(b) => format!("(scope {})", self.block(b)),
            StmtKind::With { name, init, body } => format!("(with {} {} {})", self.name(name.name), self.expr(*init), self.block(body)),
            StmtKind::Spawn(body) => self.spawn(body),
            StmtKind::Return(e) => self.keyword_stmt("return", *e),
            StmtKind::Break(e) => self.keyword_stmt("break", *e),
            StmtKind::Continue => "(continue)".into(),
            StmtKind::Yield(e) => format!("(yield {})", self.expr(*e)),
            StmtKind::Function(f) => self.function(f),
            StmtKind::Expr(e) => self.expr(*e),
            StmtKind::Error => "(error)".into(),
        }
    }

    fn keyword_stmt(&self, word: &str, value: Option<Id<Expr>>) -> String {
        match value {
            Some(e) => format!("({word} {})", self.expr(e)),
            None => format!("({word})"),
        }
    }

    fn if_stmt(&self, i: &If) -> String {
        let otherwise = match &i.otherwise {
            None => String::new(),
            Some(Else::If(s)) => format!(" {}", self.stmt(*s)),
            Some(Else::Block(b)) => format!(" {}", self.block(b)),
        };
        format!("(if {} {}{otherwise})", self.condition(&i.condition), self.block(&i.then))
    }

    fn spawn(&self, body: &SpawnBody) -> String {
        match body {
            SpawnBody::Block(b) => format!("(spawn {})", self.block(b)),
            SpawnBody::Expr(e) => format!("(spawn {})", self.expr(*e)),
        }
    }

    pub fn pattern(&self, id: Id<Pattern>) -> String {
        let list = |ids: &[Id<Pattern>]| ids.iter().map(|&p| self.pattern(p)).collect::<Vec<_>>().join(", ");
        match &self.ast[id].kind {
            PatternKind::Wildcard => "_".into(),
            PatternKind::Literal(l) => match l {
                LiteralPattern::Int { negative, literal } => format!("{}{}", if *negative { "-" } else { "" }, self.int(literal)),
                LiteralPattern::String(s) => format!("{:?}", self.name(*s)),
                LiteralPattern::Bool(b) => b.to_string(),
                LiteralPattern::Null => "null".into(),
            },
            PatternKind::Range { start, end, inclusive } => {
                format!("{}{}{}", self.pattern(*start), if *inclusive { "..=" } else { ".." }, self.pattern(*end))
            }
            PatternKind::Binding(name) => self.name(name.name).to_string(),
            PatternKind::At { name, pattern } => format!("{} @ {}", self.name(name.name), self.pattern(*pattern)),
            PatternKind::Or(alts) => alts.iter().map(|&p| self.pattern(p)).collect::<Vec<_>>().join(" | "),
            PatternKind::Tuple(elements) => format!("({})", list(elements)),
            PatternKind::Path(path) => self.path(path),
            PatternKind::TupleStruct { path, elements } => format!("{}({})", self.path(path), list(elements)),
            PatternKind::Struct { path, fields, rest } => {
                let mut parts: Vec<String> = fields
                    .iter()
                    .map(|f| match f.pattern {
                        Some(p) => format!("{}: {}", self.name(f.name.name), self.pattern(p)),
                        None => self.name(f.name.name).to_string(),
                    })
                    .collect();
                if *rest {
                    parts.push("..".into());
                }
                format!("{} {{ {} }}", self.path(path), parts.join(", "))
            }
            PatternKind::Type { name, ty } => {
                format!("{}: {}", name.map_or("_", |n| self.name(n.name)), self.ty(*ty))
            }
            PatternKind::Error => "(error)".into(),
        }
    }

    pub fn ty(&self, id: Id<Type>) -> String {
        let list = |ids: &[Id<Type>]| ids.iter().map(|&t| self.ty(t)).collect::<Vec<_>>().join(", ");
        match &self.ast[id].kind {
            TypeKind::Named(path) => self.path(path),
            TypeKind::Generic { path, args } => format!("{}<{}>", self.path(path), list(args)),
            TypeKind::Nullable(inner) => format!("{}?", self.ty(*inner)),
            TypeKind::Tuple(elements) if elements.len() == 1 => format!("({},)", list(elements)),
            TypeKind::Tuple(elements) => format!("({})", list(elements)),
            TypeKind::Array { element, len } => match len {
                Some(len) => format!("[{}; {}]", self.ty(*element), self.expr(*len)),
                None => format!("[{}]", self.ty(*element)),
            },
            TypeKind::Function { send, params, ret } => {
                let params: Vec<String> =
                    params.iter().map(|p| format!("{}{}", if p.is_var { "var " } else { "" }, self.ty(p.ty))).collect();
                format!("{}({}) -> {}", if *send { "Send " } else { "" }, params.join(", "), self.ty(*ret))
            }
            TypeKind::Union(members) => members.iter().map(|&t| self.ty(t)).collect::<Vec<_>>().join(" | "),
            TypeKind::SelfType => "Self".into(),
            TypeKind::Paren(inner) => format!("({})", self.ty(*inner)),
            TypeKind::Error => "(error)".into(),
        }
    }

    fn int(&self, literal: &crate::token::IntLiteral) -> String {
        let suffix = match literal.suffix {
            None => "",
            Some(IntSuffix::I8) => "i8",
            Some(IntSuffix::I16) => "i16",
            Some(IntSuffix::I32) => "i32",
            Some(IntSuffix::I64) => "i64",
            Some(IntSuffix::U8) => "u8",
            Some(IntSuffix::U16) => "u16",
            Some(IntSuffix::U32) => "u32",
            Some(IntSuffix::U64) => "u64",
        };
        format!("{}{suffix}", literal.value)
    }

    fn float(&self, literal: &crate::token::FloatLiteral) -> String {
        let suffix = match literal.suffix {
            None => "",
            Some(FloatSuffix::F32) => "f32",
            Some(FloatSuffix::F64) => "f64",
        };
        format!("{:?}{suffix}", literal.value)
    }

    fn list(&self, head: &str, ids: &[Id<Expr>]) -> String {
        if ids.is_empty() { format!("({head})") } else { format!("({head} {})", join(ids.iter().map(|&e| self.expr(e)))) }
    }

    fn opt(&self, id: Option<Id<Expr>>) -> String {
        id.map_or("_".into(), |e| self.expr(e))
    }

    pub fn expr(&self, id: Id<Expr>) -> String {
        use ExprKind as E;
        match &self.ast[id].kind {
            E::Int(i) => self.int(i),
            E::Float(f) => self.float(f),
            E::String(s) => format!("{:?}", self.name(*s)),
            E::Interpolated { head, parts } => {
                let parts = parts.iter().map(|p| format!("{} {:?}", self.expr(p.value), self.name(p.text)));
                format!("(interp {:?} {})", self.name(*head), join(parts))
            }
            E::Bool(b) => b.to_string(),
            E::Null => "null".into(),
            E::Ident(name) => self.name(*name).to_string(),
            E::SelfValue => "self".into(),
            E::Paren(inner) => format!("(paren {})", self.expr(*inner)),
            E::Tuple(es) => self.list("tuple", es),
            E::List(es) => self.list("list", es),
            E::ArrayRepeat { value, len } => format!("(repeat {} {})", self.expr(*value), self.expr(*len)),
            E::Map(entries) => {
                let entries = entries.iter().map(|e| format!("({} {})", self.expr(e.key), self.expr(e.value)));
                format!("(map {})", join(entries)).replace(" )", ")")
            }
            E::StructInit { path, args, fields } => {
                let fields = fields.iter().map(|f| match f {
                    FieldInit::Named { name, value } => format!("({} {})", self.name(name.name), self.expr(*value)),
                    FieldInit::Shorthand(name) => self.name(name.name).to_string(),
                    FieldInit::Spread(e) => format!("(.. {})", self.expr(*e)),
                });
                let target = if args.is_empty() {
                    self.path(path)
                } else {
                    format!("(apply {} {})", self.path(path), args.iter().map(|&t| self.ty(t)).collect::<Vec<_>>().join(" "))
                };
                format!("(init {target} {})", join(fields)).replace(" )", ")")
            }
            E::Lambda { params, ret, body } => {
                let params = params.iter().map(|p| {
                    format!("{}{}{}", if p.is_var { "var " } else { "" }, self.name(p.name.name), self.annotation(p.ty))
                });
                let body = match body {
                    LambdaBody::Expr(e) => self.expr(*e),
                    LambdaBody::Block(b) => self.block(b),
                };
                format!("(lambda ({}){} {body})", join(params), self.ret(*ret))
            }
            E::Call { callee, args } => {
                let args = args.iter().map(|a| match a.name {
                    Some(name) => format!("{}={}", self.name(name.name), self.expr(a.value)),
                    None => self.expr(a.value),
                });
                format!("(call {} {})", self.expr(*callee), join(args)).replace(" )", ")")
            }
            E::Member { object, name } => format!("(. {} {})", self.expr(*object), self.name(name.name)),
            E::TupleIndex { object, index } => format!("(. {} {})", self.expr(*object), self.int(index)),
            E::OptionalMember { object, name } => format!("(?. {} {})", self.expr(*object), self.name(name.name)),
            E::Index { object, index } => format!("(index {} {})", self.expr(*object), self.expr(*index)),
            E::Slice { object, start, end, inclusive, step } => format!(
                "({} {} {} {} {})",
                if *inclusive { "slice=" } else { "slice" },
                self.expr(*object),
                self.opt(*start),
                self.opt(*end),
                self.opt(*step)
            ),
            E::TypeApply { path, args } => {
                format!("(apply {} {})", self.path(path), args.iter().map(|&t| self.ty(t)).collect::<Vec<_>>().join(" "))
            }
            E::Unwrap(e) => format!("(unwrap {})", self.expr(*e)),
            E::Propagate(e) => format!("(propagate {})", self.expr(*e)),
            E::Unary { op, operand } => {
                let op = match op {
                    UnaryOp::Neg => "neg",
                    UnaryOp::Not => "not",
                    UnaryOp::BitNot => "bitnot",
                };
                format!("({op} {})", self.expr(*operand))
            }
            E::Binary { op, left, right } => format!("({} {} {})", binary_op(*op), self.expr(*left), self.expr(*right)),
            E::TypeTest { expr, ty } => format!("(is {} {})", self.expr(*expr), self.ty(*ty)),
            E::Cast { expr, ty } => format!("(as {} {})", self.expr(*expr), self.ty(*ty)),
            E::Range { start, end, inclusive } => {
                format!("({} {} {})", if *inclusive { "..=" } else { ".." }, self.expr(*start), self.expr(*end))
            }
            E::Spawn(body) => self.spawn(body),
            E::If { condition, then, otherwise } => {
                let otherwise = match otherwise {
                    ElseExpr::If(e) | ElseExpr::Value(e) => self.expr(*e),
                };
                format!("(if {} {} {otherwise})", self.condition(condition), self.expr(*then))
            }
            E::Match { subject, arms } => {
                let arms = arms.iter().map(|arm| {
                    let guard = arm.guard.map_or(String::new(), |g| format!(" if {}", self.expr(g)));
                    format!("({}{guard} => {})", self.pattern(arm.pattern), self.expr(arm.value))
                });
                format!("(match {} {})", self.expr(*subject), join(arms)).replace(" )", ")")
            }
            E::Error => "(error)".into(),
        }
    }
}

fn assign_op(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign => "=",
        AssignOp::Add => "+=",
        AssignOp::Sub => "-=",
        AssignOp::Mul => "*=",
        AssignOp::Div => "/=",
        AssignOp::Rem => "%=",
        AssignOp::BitAnd => "&=",
        AssignOp::BitOr => "|=",
        AssignOp::BitXor => "^=",
        AssignOp::Shl => "<<=",
        AssignOp::Shr => ">>=",
        AssignOp::Coalesce => "??=",
    }
}

fn binary_op(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Rem => "%",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::LtEq => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::GtEq => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        BinaryOp::Coalesce => "??",
    }
}

#[cfg(test)]
mod tests {
    use mote_base::Span;

    use super::*;
    use crate::token::{IntLiteral, Radix};

    const AT: Span = Span { start: 0, end: 0 };

    struct Tree {
        ast: Ast,
        names: Interner,
    }

    impl Tree {
        fn new() -> Self {
            Tree { ast: Ast::default(), names: Interner::new() }
        }

        fn expr(&mut self, kind: ExprKind) -> Id<Expr> {
            self.ast.exprs.alloc(Expr { kind, span: AT })
        }

        fn int(&mut self, value: u128) -> Id<Expr> {
            self.expr(ExprKind::Int(IntLiteral { value, radix: Radix::Decimal, suffix: None }))
        }

        fn ident(&mut self, name: &str) -> Id<Expr> {
            let name = self.names.intern(name);
            self.expr(ExprKind::Ident(name))
        }

        fn binding(&mut self, name: &str) -> Id<Pattern> {
            let name = Ident { name: self.names.intern(name), span: AT };
            self.ast.patterns.alloc(Pattern { kind: PatternKind::Binding(name), span: AT })
        }

        fn stmt(&mut self, kind: StmtKind) -> Id<Stmt> {
            self.ast.stmts.alloc(Stmt { kind, span: AT })
        }

        fn dump(&self) -> Dump<'_> {
            Dump { ast: &self.ast, names: &self.names }
        }
    }

    #[test]
    fn a_binary_tree_prints_in_prefix_form() {
        let mut t = Tree::new();
        let one = t.int(1);
        let two = t.int(2);
        let x = t.ident("x");
        let product = t.expr(ExprKind::Binary { op: BinaryOp::Mul, left: two, right: x });
        let sum = t.expr(ExprKind::Binary { op: BinaryOp::Add, left: one, right: product });
        assert_eq!(t.dump().expr(sum), "(+ 1 (* 2 x))");
    }

    #[test]
    fn a_call_prints_named_arguments_with_equals() {
        let mut t = Tree::new();
        let f = t.ident("f");
        let one = t.int(1);
        let three = t.int(3);
        let width = Ident { name: t.names.intern("width"), span: AT };
        let args = vec![Arg { name: None, value: one }, Arg { name: Some(width), value: three }];
        let call = t.expr(ExprKind::Call { callee: f, args });
        assert_eq!(t.dump().expr(call), "(call f 1 width=3)");
    }

    #[test]
    fn an_array_repeat_prints_value_then_length() {
        let mut t = Tree::new();
        let zero = t.int(0);
        let len = t.int(1024);
        let repeat = t.expr(ExprKind::ArrayRepeat { value: zero, len });
        assert_eq!(t.dump().expr(repeat), "(repeat 0 1024)");
    }

    #[test]
    fn a_slice_marks_missing_parts() {
        let mut t = Tree::new();
        let xs = t.ident("xs");
        let two = t.int(2);
        let slice = t.expr(ExprKind::Slice { object: xs, start: None, end: Some(two), inclusive: true, step: None });
        assert_eq!(t.dump().expr(slice), "(slice= xs _ 2 _)");
    }

    #[test]
    fn a_let_with_a_tuple_pattern_prints_the_pattern() {
        let mut t = Tree::new();
        let a = t.binding("a");
        let b = t.binding("b");
        let pair = t.ast.patterns.alloc(Pattern { kind: PatternKind::Tuple(vec![a, b]), span: AT });
        let init = t.ident("t");
        let stmt = t.stmt(StmtKind::Binding(Binding { kind: BindingKind::Let, pattern: pair, ty: None, init }));
        assert_eq!(t.dump().stmt(stmt), "(let (a, b) t)");
    }

    #[test]
    fn an_if_let_with_an_else_if_nests() {
        let mut t = Tree::new();
        let some = t.binding("x");
        let value = t.ident("opt");
        let flag = t.ident("flag");
        let inner = t.stmt(StmtKind::If(If { condition: Condition::Expr(flag), then: Block { stmts: vec![], span: AT }, otherwise: None }));
        let outer = t.stmt(StmtKind::If(If {
            condition: Condition::Let { pattern: some, value },
            then: Block { stmts: vec![], span: AT },
            otherwise: Some(Else::If(inner)),
        }));
        assert_eq!(t.dump().stmt(outer), "(if (let x opt) (block) (if flag (block)))");
    }

    #[test]
    fn a_value_if_prints_both_branches() {
        let mut t = Tree::new();
        let c = t.ident("c");
        let one = t.int(1);
        let two = t.int(2);
        let e = t.expr(ExprKind::If { condition: Condition::Expr(c), then: one, otherwise: ElseExpr::Value(two) });
        assert_eq!(t.dump().expr(e), "(if c 1 2)");
    }

    #[test]
    fn a_function_item_prints_its_signature_and_body() {
        let mut t = Tree::new();
        let int_name = Ident { name: t.names.intern("Int"), span: AT };
        let int_ty = t.ast.types.alloc(Type { kind: TypeKind::Named(Path { segments: vec![int_name], span: AT }), span: AT });
        let x = Ident { name: t.names.intern("x"), span: AT };
        let param = Param { kind: ParamKind::Named { is_var: false, name: x, ty: Some(int_ty), default: None }, span: AT };
        let x_use = t.ident("x");
        let ret = t.stmt(StmtKind::Return(Some(x_use)));
        let function = Function {
            name: Ident { name: t.names.intern("id"), span: AT },
            generics: vec![],
            params: vec![param],
            ret: Some(int_ty),
            body: Block { stmts: vec![ret], span: AT },
        };
        let item = t.ast.items.alloc(Item { doc: None, attrs: vec![], is_pub: true, kind: ItemKind::Function(function), span: AT });
        t.ast.root.push(item);
        assert_eq!(t.dump().root(), "pub (fn id (x:Int) -> Int (block (return x)))");
    }
}
