use mote_base::{Id, Interner, Severity};

use super::check;
use crate::defs::{DefId, Resolved};
use crate::program::Program;
use crate::resolve::resolve;
use crate::testing::parse_sources;
use crate::tree::{Body, Callee, Condition, Conversion, Else, Expr, ExprKind, Pattern, PatternKind, Stmt, StmtKind};

struct Run {
    program: Program,
    interner: Interner,
    errors: Vec<String>,
}

fn run(sources: &[(&str, &str)]) -> Run {
    let mut parsed = parse_sources(sources);
    let mut program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    check(&mut program, &mut parsed.interner, &mut parsed.sink);
    let errors = parsed.sink.iter().filter(|diagnostic| diagnostic.severity == Severity::Error).map(|diagnostic| diagnostic.message.clone()).collect();
    Run { program, interner: parsed.interner, errors }
}

fn main(text: &str) -> Run {
    run(&[("main", text)])
}

/// A function body of `main` as one line, statements separated by `;`.
fn dump(run: &Run, name: &str) -> String {
    dump_def(run, run.def(name), name)
}

/// The body of `Owner.name`.
fn dump_method(run: &Run, owner: &str, name: &str) -> String {
    dump_def(run, run.method(owner, name), name)
}

fn dump_def(run: &Run, def: DefId, name: &str) -> String {
    let body = run.program.checked.get(&def).unwrap_or_else(|| panic!("{name} has no checked body"));
    Dump { run, body }.block(&body.block)
}

impl Run {
    fn method(&self, owner: &str, name: &str) -> DefId {
        let symbol = self.interner.get(name).expect("a known name");
        let methods = match &self.program.defs[self.def(owner)].kind {
            crate::defs::DefKind::Record(record) => &record.methods,
            crate::defs::DefKind::Enum(en) => &en.methods,
            _ => panic!("{owner} is not a record or enum"),
        };
        *methods.iter().find(|&&method| self.program.defs[method].name == symbol).expect("a method")
    }

    fn def(&self, name: &str) -> DefId {
        let module = self.program.modules.iter().find(|(_, module)| module.path.len() == 1 && self.interner.resolve(module.path[0]) == "main").expect("a main module").1;
        let symbol = self.interner.get(name).expect("a known name");
        match module.scope.get(&symbol) {
            Some(Resolved::Def(def)) => *def,
            other => panic!("{name} is {other:?}"),
        }
    }
}

struct Dump<'a> {
    run: &'a Run,
    body: &'a Body,
}

impl Dump<'_> {
    fn name(&self, symbol: mote_base::Symbol) -> &str {
        self.run.interner.resolve(symbol)
    }

    fn ty(&self, expr: &Expr) -> String {
        self.run.program.render(expr.ty, &self.run.interner)
    }

    fn field_name(&self, ty: crate::types::TyId, index: u32) -> &str {
        let crate::types::TyKind::Adt { def, .. } = self.run.program.types.kind(ty) else { panic!("a record type") };
        self.name(self.run.program.defs[*def].record().expect("a record").fields[index as usize].name)
    }

    fn block(&self, block: &crate::tree::Block) -> String {
        block.stmts.iter().map(|&stmt| self.stmt(stmt)).collect::<Vec<_>>().join("; ")
    }

    fn stmt(&self, id: Id<Stmt>) -> String {
        match &self.body.stmts[id].kind {
            StmtKind::Let { pattern, init } => format!("(let {} {})", self.pattern(*pattern), self.expr(*init)),
            StmtKind::Assign { target, op, value } => {
                let op = op.map_or("=".to_string(), |op| format!("{}=", self.op(op)));
                format!("({op} {} {})", self.expr(*target), self.expr(*value))
            }
            StmtKind::If(branch) => {
                let mut text = format!("(if {} {{{}}}", self.condition(&branch.condition), self.block(&branch.then));
                match &branch.otherwise {
                    Some(Else::Block(block)) => text += &format!(" else {{{}}}", self.block(block)),
                    Some(Else::If(other)) => text += &format!(" else {}", self.stmt(*other)),
                    None => {}
                }
                text + ")"
            }
            StmtKind::While { condition, body } => format!("(while {} {{{}}})", self.condition(condition), self.block(body)),
            StmtKind::Match(matched) => format!("(match {} [{}])", self.expr(matched.subject), self.arms(&matched.arms)),
            StmtKind::Return(Some(value)) => format!("(return {})", self.expr(*value)),
            StmtKind::Return(None) => "(return)".to_string(),
            StmtKind::Break => "(break)".to_string(),
            StmtKind::Continue => "(continue)".to_string(),
            StmtKind::Expr(expr) => self.expr(*expr),
            StmtKind::Scope(block) => format!("(scope {{{}}})", self.block(block)),
            StmtKind::With { local, init, body, .. } => format!("(with {} {} {{{}}})", self.name(self.body.locals[*local].name), self.expr(*init), self.block(body)),
            other => format!("{other:?}"),
        }
    }

    fn condition(&self, condition: &Condition) -> String {
        match condition {
            Condition::Expr(expr) => self.expr(*expr),
            Condition::Let { pattern, value } => format!("let {} = {}", self.pattern(*pattern), self.expr(*value)),
        }
    }

    fn variant_name(&self, ty: crate::types::TyId, index: u32) -> &str {
        let crate::types::TyKind::Adt { def, .. } = self.run.program.types.kind(ty) else { panic!("an enum type") };
        self.name(self.run.program.defs[*def].enum_def().expect("an enum").variants[index as usize].name)
    }

    fn pattern(&self, id: Id<Pattern>) -> String {
        let pattern = &self.body.patterns[id];
        let join = |items: Vec<String>| items.join(", ");
        match &pattern.kind {
            PatternKind::Wildcard => "_".to_string(),
            PatternKind::Bind(local) => self.name(self.body.locals[*local].name).to_string(),
            PatternKind::At { local, pattern } => format!("{}@{}", self.name(self.body.locals[*local].name), self.pattern(*pattern)),
            PatternKind::Literal(literal) => match literal {
                crate::tree::Literal::Int(value) => value.to_string(),
                crate::tree::Literal::Bool(value) => value.to_string(),
                crate::tree::Literal::String(text) => format!("{:?}", self.name(*text)),
                crate::tree::Literal::Null => "null".to_string(),
            },
            PatternKind::Range { start, end, inclusive } => format!("{start}{}{end}", if *inclusive { "..=" } else { ".." }),
            PatternKind::Or(items) => items.iter().map(|&item| self.pattern(item)).collect::<Vec<_>>().join(" | "),
            PatternKind::Tuple(items) => format!("({})", join(items.iter().map(|&item| self.pattern(item)).collect())),
            PatternKind::OptionalSome(inner) => format!("Some({})", self.pattern(*inner)),
            PatternKind::OptionalNone => "None".to_string(),
            PatternKind::Variant { variant, fields } => {
                let name = self.variant_name(pattern.ty, *variant);
                if fields.is_empty() {
                    name.to_string()
                } else {
                    format!("{name}({})", join(fields.iter().map(|field| format!("{}:{}", field.index, self.pattern(field.pattern))).collect()))
                }
            }
            PatternKind::Record { fields } => format!("{{{}}}", join(fields.iter().map(|field| format!("{}:{}", self.field_name(pattern.ty, field.index), self.pattern(field.pattern))).collect())),
            PatternKind::Type { local, ty } => {
                let ty = self.run.program.render(*ty, &self.run.interner);
                match local {
                    Some(local) => format!("{}: {ty}", self.name(self.body.locals[*local].name)),
                    None => format!("_: {ty}"),
                }
            }
            PatternKind::Error => "<error>".to_string(),
        }
    }

    fn arms(&self, arms: &[crate::tree::Arm]) -> String {
        let arms: Vec<String> = arms
            .iter()
            .map(|arm| {
                let guard = arm.guard.map_or(String::new(), |guard| format!(" if {}", self.expr(guard)));
                let body = match &arm.body {
                    crate::tree::ArmBody::Value(value) => self.expr(*value),
                    crate::tree::ArmBody::Block(block) => format!("{{{}}}", self.block(block)),
                };
                format!("{}{guard} => {body}", self.pattern(arm.pattern))
            })
            .collect();
        arms.join(", ")
    }

    fn op(&self, op: crate::tree::BinaryOp) -> String {
        format!("{op:?}").to_lowercase()
    }

    fn list(&self, items: &[Id<Expr>]) -> String {
        items.iter().map(|&item| self.expr(item)).collect::<Vec<_>>().join(" ")
    }

    fn expr(&self, id: Id<Expr>) -> String {
        let expr = &self.body.exprs[id];
        match &expr.kind {
            ExprKind::Int(value) => format!("{value}:{}", self.ty(expr)),
            ExprKind::Float(value) => format!("{value}:{}", self.ty(expr)),
            ExprKind::Bool(value) => value.to_string(),
            ExprKind::Null => "null".to_string(),
            ExprKind::String(text) => format!("{:?}", self.name(*text)),
            ExprKind::Concat(parts) => format!("(concat {})", self.list(parts)),
            ExprKind::Show(inner) => format!("(show {})", self.expr(*inner)),
            ExprKind::Local(local) => self.name(self.body.locals[*local].name).to_string(),
            ExprKind::Global(def) => format!("${}", self.name(self.run.program.defs[*def].name)),
            ExprKind::Function(function) => format!("fn:{}", self.name(self.run.program.defs[function.def].name)),
            ExprKind::Tuple(items) => format!("(tuple {})", self.list(items)),
            ExprKind::List(items) => format!("(list {})", self.list(items)).replace(" )", ")"),
            ExprKind::Closure(closure) => {
                let names = |locals: &[Id<crate::tree::Local>]| locals.iter().map(|&local| self.name(self.body.locals[local].name).to_string()).collect::<Vec<_>>().join(",");
                format!("(closure [{}] caps=[{}] {{{}}})", names(&closure.params), names(&closure.captures), self.block(&closure.body))
            }
            ExprKind::Spawn { task, scoped } => {
                let captures = task.captures.iter().map(|&local| self.name(self.body.locals[local].name).to_string()).collect::<Vec<_>>().join(",");
                format!("(spawn{} caps=[{captures}] {{{}}})", if *scoped { " scoped" } else { "" }, self.block(&task.body))
            }
            ExprKind::Call { callee, args } => {
                let callee = match callee {
                    Callee::Function(function) => {
                        let def = &self.run.program.defs[function.def];
                        let name = self.name(def.name).to_string();
                        if def.function().is_some_and(|item| !item.generics.is_empty()) {
                            let shown: Vec<String> = function
                                .args
                                .iter()
                                .map(|arg| match arg {
                                    crate::GenericArg::Type(ty) => self.run.program.render(*ty, &self.run.interner),
                                    crate::GenericArg::Const(arg) => self.run.program.render_const(arg, &self.run.interner),
                                })
                                .collect();
                            format!("{name}<{}>", shown.join(","))
                        } else {
                            name
                        }
                    }
                    Callee::Value(value) => format!("({})", self.expr(*value)),
                    Callee::TraitMethod { method, self_ty, .. } => format!("{}@{}", self.name(self.run.program.defs[*method].name), self.run.program.render(*self_ty, &self.run.interner)),
                    Callee::Builtin { name, .. } => format!("@{}", self.name(*name)),
                };
                format!("(call {callee} {})", self.list(args)).replace(" )", ")")
            }
            ExprKind::TupleField { object, index } => format!("(.{index} {})", self.expr(*object)),
            ExprKind::Field { object, index } => format!("(.{} {})", self.field_name(self.body.exprs[*object].ty, *index), self.expr(*object)),
            ExprKind::Record { fields, base } => {
                let fields: Vec<String> = fields.iter().map(|field| format!("{}={}", self.field_name(expr.ty, field.index), self.expr(field.value))).collect();
                let base = base.map_or(String::new(), |base| format!(" ..{}", self.expr(base)));
                format!("(record {}{base} {})", self.ty(expr), fields.join(" ")).replace(" )", ")")
            }
            ExprKind::Unary { op, operand } => format!("({} {})", format!("{op:?}").to_lowercase(), self.expr(*operand)),
            ExprKind::Binary { op, left, right } => format!("({} {} {})", self.op(*op), self.expr(*left), self.expr(*right)),
            ExprKind::Cast(inner) => format!("(cast {} {})", self.ty(expr), self.expr(*inner)),
            ExprKind::Convert { conversion, expr: inner } => {
                let name = match conversion {
                    Conversion::ToOptional => "to-optional",
                    Conversion::ToUnion => "to-union",
                    Conversion::WidenUnion => "widen-union",
                    Conversion::Narrow => "narrow",
                    Conversion::Adapt => "adapt",
                    Conversion::ToView => "to-view",
                };
                format!("({name} {})", self.expr(*inner))
            }
            ExprKind::If { condition, then, otherwise } => format!("(if {} {} {})", self.condition(condition), self.expr(*then), self.expr(*otherwise)),
            ExprKind::Is { expr: inner, ty } => format!("(is {} {})", self.expr(*inner), self.run.program.render(*ty, &self.run.interner)),
            ExprKind::Unwrap(inner) => format!("(unwrap {})", self.expr(*inner)),
            ExprKind::Propagate(inner) => format!("(try {})", self.expr(*inner)),
            ExprKind::Variant { variant, fields } => {
                let name = self.variant_name(expr.ty, *variant);
                format!("(variant {name} {})", self.list(&fields.iter().map(|field| field.value).collect::<Vec<_>>())).replace(" )", ")")
            }
            ExprKind::Match(matched) => format!("(match {} [{}])", self.expr(matched.subject), self.arms(&matched.arms)),
            ExprKind::Error => "<error>".to_string(),
            other => format!("{other:?}"),
        }
    }
}

/// The type of the nth `let` initializer in a function.
fn let_type(run: &Run, name: &str, nth: usize) -> String {
    let def = run.def(name);
    let body = &run.program.checked[&def];
    let init = body
        .block
        .stmts
        .iter()
        .filter_map(|&stmt| match &body.stmts[stmt].kind {
            StmtKind::Let { init, .. } => Some(*init),
            _ => None,
        })
        .nth(nth)
        .expect("a let");
    run.program.render(body.exprs[init].ty, &run.interner)
}

#[test]
fn literals_take_the_expected_type_and_default_to_int_and_float() {
    let run = main("fn f() {\n let a: UInt8 = 200\n let b = 5\n let c = 2.5\n let d: Float32 = 1.5\n let e = 7u16\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a 200:UInt8); (let b 5:Int); (let c 2.5:Float); (let d 1.5:Float32); (let e 7:UInt16)");
}

#[test]
fn a_literal_out_of_range_is_an_error() {
    let run = main("fn f() {\n let a: UInt8 = 256\n let b = -129i8\n let c = -128i8\n let d = 300u8\n}");
    assert_eq!(run.errors, ["256 does not fit UInt8", "-129 does not fit Int8", "300 does not fit UInt8"]);
}

#[test]
fn an_int_literal_is_never_a_float() {
    let run = main("fn f() {\n let a: Float = 3\n}");
    assert_eq!(run.errors, ["expected Float, found Int"]);
}

#[test]
fn arithmetic_needs_one_type_and_a_literal_adapts() {
    let run = main("fn f(a: UInt8, b: Int, c: Float) {\n let x = a + 1\n let y = 1 + a\n let z = 2 * 3\n let w = b - 1\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let x (add a 1:UInt8)); (let y (add 1:UInt8 a)); (let z (mul 2:Int 3:Int)); (let w (sub b 1:Int))");
    let bad = main("fn f(b: Int, c: Float) {\n let x = b + c\n let y = c + 1\n let z = \"a\" - \"b\"\n}");
    assert_eq!(
        bad.errors,
        ["`+` needs one type on both sides, found Int and Float", "`+` needs one type on both sides, found Float and Int", "`-` does not work on String"]
    );
}

#[test]
fn strings_join_and_order_and_numbers_compare() {
    let run = main("fn f(a: String, b: Int) -> Bool {\n let s = a + \"x\"\n let c = a < \"b\"\n return b >= 3 && c\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let s (add a \"x\")); (let c (lt a \"b\")); (return (and (gteq b 3:Int) c))");
    let bad = main("fn f(a: Int) {\n let c = a < 1.5\n if a {}\n let d = !a\n}");
    assert_eq!(bad.errors, ["`<` needs one type on both sides, found Int and Float", "expected Bool, found Int", "`!` does not work on Int"]);
}

#[test]
fn bit_operators_need_integers() {
    let run = main("fn f(a: UInt8, b: Float) {\n let x = a & 1\n let y = ~a\n let z = b | 1.0\n let w = -a\n}");
    assert_eq!(run.errors, ["`|` does not work on Float", "`-` does not work on UInt8"]);
}

#[test]
fn negative_literals_fold() {
    let run = main("fn f() {\n let a = -5\n let b: Int8 = -128\n let c = -2.5\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a -5:Int); (let b -128:Int8); (let c -2.5:Float)");
}

#[test]
fn a_value_converts_to_an_optional_and_a_union() {
    let run = main("fn f(n: Int) {\n let a: Int? = 4\n let b: Int? = null\n let c: Int | String = n\n let e: Int?? = n\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a (to-optional 4:Int)); (let b null); (let c (to-union n)); (let e (to-optional (to-optional n)))");
    assert_eq!(let_type(&run, "f", 1), "Int?");
}

#[test]
fn a_literal_looks_through_an_optional_or_a_union_for_its_number_type() {
    let run = main("fn f() {\n let a: UInt8? = 5\n let b: Int | String = 5\n let c: Int8 | Float = 5\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a (to-optional 5:UInt8)); (let b (to-union 5:Int)); (let c (to-union 5:Int8))");
}

#[test]
fn values_that_do_not_fit_are_errors() {
    let run = main("fn f(n: Int, o: Int?, u: Int | String) {\n let a: Int = null\n let b: Int = o\n let c: String = u\n let d: Float | String = n\n let e: Int? = \"x\"\n}");
    assert_eq!(
        run.errors,
        ["expected Int, found Null", "expected Int, found Int?", "expected String, found String | Int", "expected String | Float, found Int", "expected Int?, found String"]
    );
}

#[test]
fn a_union_widens_to_a_bigger_union() {
    let run = main("fn f(u: Int | String) {\n let a: Int | String | Float = u\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a (widen-union u))");
}

#[test]
fn equality_takes_the_same_type_or_an_optional_of_it() {
    let run = main("fn f(a: Int, o: Int?, s: String) {\n let x = o == null\n let y = o == 1\n let z = 1 == o\n let w = a == 2\n let v = s != \"q\"\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(
        dump(&run, "f"),
        "(let x (eq o null)); (let y (eq o (to-optional 1:Int))); (let z (eq (to-optional 1:Int) o)); (let w (eq a 2:Int)); (let v (noteq s \"q\"))"
    );
    let bad = main("fn f(a: Int, s: String, g: (Int) -> Int) {\n let x = a == null\n let y = s == 1\n let z = g == g\n let w = a == 1.5\n}");
    assert_eq!(
        bad.errors,
        ["`==` cannot compare Int with Null", "`==` cannot compare String with Int", "`==` does not work on (Int) -> Int", "`==` cannot compare Int with Float"]
    );
}

#[test]
fn casts_cover_numbers_and_bool() {
    let run = main("fn f(b: Bool, x: Float) {\n let a = 300 as UInt8\n let c = x as Int\n let d = b as Int\n let e = 7 as Float\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a (cast UInt8 300:Int)); (let c (cast Int x)); (let d (cast Int b)); (let e (cast Float 7:Int))");
    let bad = main("fn f(s: String, n: Int) {\n let a = s as Int\n let b = n as Bool\n let c = n as Missing\n}");
    assert_eq!(bad.errors, ["cannot cast String to Int", "cannot cast Int to Bool", "unknown name `Missing`"]);
}

#[test]
fn calls_match_positional_named_and_default_arguments() {
    let run = main("fn pad(s: String, width: Int = 8, fill: String = \" \") -> String { return s }\nfn f() {\n let a = pad(\"x\")\n let b = pad(\"x\", 4)\n let c = pad(\"x\", fill=\"-\")\n let d = pad(fill=\"*\", s=\"x\")\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(
        dump(&run, "f"),
        "(let a (call pad \"x\" 8:Int \" \")); (let b (call pad \"x\" 4:Int \" \")); (let c (call pad \"x\" 8:Int \"-\")); (let d (call pad \"x\" 8:Int \"*\"))"
    );
    assert_eq!(let_type(&run, "f", 0), "String");
}

#[test]
fn defaults_are_trailing_var_has_none_and_a_variadic_is_last() {
    let run = with_collections("fn a(x: Int = 1, y: Int) {}\nfn b(var x: Int = 1) {}\nfn c(...xs: Int, y: Int) {}\nfn d(x: Int, y: Int = 1, ...zs: Int) {}");
    assert_eq!(run.errors, ["a parameter after a default has a default", "a `var` parameter has no default", "`...` is the last parameter"]);
}

#[test]
fn named_arguments_record_the_order_they_are_written_in() {
    let run = main("fn pad(s: String, width: Int = 8, fill: String = \" \") -> String { return s }\nfn f() {\n let a = pad(\"x\", 4)\n let b = pad(fill=\"*\", s=\"x\")\n let c = pad(\"x\", fill=\"-\", width=2)\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    let body = &run.program.checked[&run.def("f")];
    let mut orders: Vec<&Vec<u32>> = body.arg_order.values().collect();
    orders.sort();
    assert_eq!(orders, [&vec![0, 2, 1], &vec![2, 0, 1]]);
}

#[test]
fn bad_calls_are_reported() {
    let run = main("fn add(a: Int, b: Int) -> Int { return a + b }\nfn f() {\n add(1)\n add(1, 2, 3)\n add(1, c=2)\n add(1, a=2)\n add(\"x\", 2)\n let n = 5\n n(1)\n add(b=1, 2)\n}");
    assert_eq!(
        run.errors,
        [
            "missing argument `b`",
            "too many arguments",
            "no parameter `c`",
            "`a` is given twice",
            "expected Int, found String",
            "Int is not callable",
            "a positional argument comes before the named ones"
        ]
    );
}

#[test]
fn a_function_value_is_called_through_its_type() {
    let run = main("fn add(a: Int, b: Int) -> Int { return a + b }\nfn f(g: (Int, Int) -> Int) {\n let h = add\n let x = h(1, 2)\n let y = g(3, 4)\n g(1)\n}");
    assert_eq!(run.errors, ["expected 2 arguments, found 1"]);
    assert_eq!(dump(&run, "f"), "(let h fn:add); (let x (call (h) 1:Int 2:Int)); (let y (call (g) 3:Int 4:Int)); <error>");
}

#[test]
fn a_default_argument_is_checked_in_the_declaring_module() {
    let run = run(&[("util", "let BASE = 4\npub fn pick(w: Int = BASE) -> Int { return w }"), ("main", "import { pick } from .util\nfn f() -> Int { return pick() }")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(return (call pick $BASE))");
}

#[test]
fn a_name_binds_once_per_scope_and_may_shadow_in_a_nested_one() {
    let run = main("fn f(a: Int) {\n let x = 1\n let x = 2\n if true {\n let x = \"s\"\n let a = 3\n }\n}");
    assert_eq!(run.errors, ["`x` is defined twice"]);
    let params = main("fn f(a: Int, a: Int) { let a = 1 }");
    assert_eq!(params.errors, ["`a` is defined twice", "`a` is defined twice"]);
}

#[test]
fn only_var_paths_are_written() {
    let run = main("fn f(p: Int, var q: Int) {\n let a = 1\n var b = 2\n a = 3\n b = 4\n p = 5\n q = 6\n b += 1\n b += 1.5\n 1 = 2\n}");
    assert_eq!(
        run.errors,
        [
            "cannot assign: no `var` path to this place",
            "cannot assign: no `var` path to this place",
            "`+` needs one type on both sides, found Int and Float",
            "cannot assign to this expression"
        ]
    );
}

#[test]
fn a_var_parameter_takes_a_var_place_or_a_value() {
    let run = main("fn bump(var n: Int) { n += 1 }\nfn f() {\n let a = 1\n var b = 2\n bump(a)\n bump(b)\n bump(1 + 2)\n}");
    assert_eq!(run.errors, ["this place has no `var` path to write through"]);
}

#[test]
fn compound_assignment_keeps_the_target_type() {
    let run = main("fn f() {\n var s = \"a\"\n s += \"b\"\n var n: UInt8 = 1\n n += 2\n n <<= 1\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let s \"a\"); (add= s \"b\"); (let n 1:UInt8); (add= n 2:UInt8); (shl= n 1:UInt8)");
}

#[test]
fn if_statements_and_value_ifs_check_their_branches() {
    let run = main(
        "fn f(c: Bool, n: Int) -> Int {\n let a = if c { 1 } else { 2 }\n let b = if c { n } else { null }\n let d: Int? = if c { 3 } else { null }\n if c { return a } else if n > 1 { return 2 } else { return 3 }\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 0), "Int");
    assert_eq!(let_type(&run, "f", 1), "Int?");
    assert_eq!(let_type(&run, "f", 2), "Int?");
    let bad = main("fn f(c: Bool) {\n let a = if c { 1 } else { \"s\" }\n}");
    assert_eq!(bad.errors, ["branches have different types: Int and String"]);
}

#[test]
fn loops_need_a_loop_for_break_and_continue() {
    let run = main("fn f(n: Int) {\n var i = 0\n while i < n {\n i += 1\n if i == 3 { continue }\n if i == 5 { break }\n }\n break\n continue\n}");
    assert_eq!(run.errors, ["outside a loop", "outside a loop"]);
}

#[test]
fn a_function_must_return_on_every_path() {
    let missing = main("fn f(c: Bool) -> Int {\n if c { return 1 }\n}");
    assert_eq!(missing.errors, ["missing return"]);
    let both = main("fn f(c: Bool) -> Int {\n if c { return 1 } else { return 2 }\n}");
    assert_eq!(both.errors, Vec::<String>::new());
    let forever = main("fn f() -> Int {\n while true { }\n}");
    assert_eq!(forever.errors, Vec::<String>::new());
    let broken = main("fn f() -> Int {\n while true { break }\n}");
    assert_eq!(broken.errors, ["missing return"]);
    let unit = main("fn f(c: Bool) {\n if c { return }\n}");
    assert_eq!(unit.errors, Vec::<String>::new());
}

#[test]
fn return_values_are_checked_against_the_signature() {
    let run = main("fn f() -> Int {\n return \"x\"\n}\nfn g() -> Int {\n return\n}\nfn h() {\n return 1\n}\nfn i() -> Int? {\n return 1\n}");
    assert_eq!(run.errors, ["expected Int, found String", "return needs a Int value", "expected Null, found Int"]);
}

#[test]
fn interpolation_joins_text_and_shows_scalars() {
    let run = main("fn f(n: Int, s: String, b: Bool) -> String {\n return \"a${n}b${s}${b}\"\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(return (concat \"a\" (show n) \"b\" s (show b)))");
}

#[test]
fn tuples_unions_and_functions_write_themselves() {
    let run = with_collections("class Opaque {}\nfn f(t: (Int, String), u: Int | String, g: (Int) -> Int, xs: [Int], fx: [Float; 3]) -> String {\n let a = t.to_string()\n let b = u.to_string()\n let c = g.to_string()\n let d = xs.to_string()\n let e = fx.to_string()\n return \"${t} ${u} ${g} ${xs} ${fx}\"\n}\nfn bad(t: (Int, Opaque), l: List<Int>, oa: [Opaque], ofx: [Opaque; 2]) -> String {\n let a = t.to_string()\n return \"${t}${l}${oa}${ofx}\"\n}");
    assert_eq!(run.errors, ["(Int, Opaque) has no methods", "(Int, Opaque) cannot be written in a string", "List<Int> has no methods", "[Opaque] cannot be written in a string", "[Opaque; 2] cannot be written in a string"]);
    assert_eq!(dump(&run, "f").split("; ").next(), Some("(let a (show t))"));
}

#[test]
fn tuples_build_destructure_and_index() {
    let run = main("fn f() {\n let t = (1, \"a\")\n let (n, s) = t\n let u = t.0\n let (a, (b, _)) = (1, (2, 3))\n let v: (UInt8, Int) = (1, 2)\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(
        dump(&run, "f"),
        "(let t (tuple 1:Int \"a\")); (let (n, s) t); (let u (.0 t)); (let (a, (b, _)) (tuple 1:Int (tuple 2:Int 3:Int))); (let v (tuple 1:UInt8 2:Int))"
    );
    assert_eq!(let_type(&run, "f", 2), "Int");
    let bad = main("fn f() {\n let t = (1, 2)\n let (a, b, c) = t\n let x = t.2\n let y = 5\n let z = y.0\n}");
    assert_eq!(bad.errors, ["a tuple pattern of 3 elements does not match (Int, Int)", "(Int, Int) has no element 2", "Int is not a tuple"]);
}

#[test]
fn globals_are_typed_in_any_order_and_read_by_functions() {
    let run = main("let A = B + 1\nlet B = 2\nlet C: Float = 1.5\nfn f() -> Int { return A }\nfn g() -> Float { return C }");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(return $A)");
    let a = run.def("A");
    assert_eq!(run.program.render(run.program.defs[a].kind_global_ty(), &run.interner), "Int");
}

impl crate::defs::Def {
    fn kind_global_ty(&self) -> crate::types::TyId {
        match &self.kind {
            crate::defs::DefKind::Global(global) => global.ty,
            _ => panic!("a global"),
        }
    }
}

#[test]
fn globals_that_depend_on_each_other_are_an_error_and_functions_do_not_assign_them() {
    let cycle = main("let A = B\nlet B = A");
    assert_eq!(cycle.errors, ["`A` depends on itself"]);
    let assign = main("var count = 0\nfn bump() { count += 1 }");
    assert_eq!(assign.errors, ["cannot assign: no `var` path to this place", "only the module's own top-level code assigns a global"]);
}

#[test]
fn functions_and_lambdas_do_not_write_into_a_var_global() {
    let source = "extend <T> List<T> {\n pub fn push(var self, x: T) {}\n}\nclass Cell {\n var n: Int\n pub fn bump(var self) { self.n += 1 }\n}\nvar items: List<Int> = [1]\nvar cell = Cell { n: 0 }\nfn fill(var xs: List<Int>) { xs.push(2) }\nfn a() { items.push(3) }\nfn b() { items[0] = 4 }\nfn c() { cell.bump() }\nfn d() { cell.n = 5 }\nfn e() { fill(items) }\nfn f() { let g = || items.push(6) }\nitems.push(7)\ncell.bump()\nfill(items)";
    let run = main(source);
    assert_eq!(run.errors.len(), 6, "{:?}", run.errors);
}

#[test]
fn test_blocks_are_checked_like_functions() {
    let run = main("test \"works\" {\n let a: Int = \"s\"\n}");
    assert_eq!(run.errors, ["expected Int, found String"]);
}

#[test]
fn an_error_in_one_place_does_not_hide_the_next() {
    let run = main("fn f() {\n let a = missing\n let b = a + 1\n let c: Int = \"x\"\n}");
    assert_eq!(run.errors, ["unknown name `missing`", "expected Int, found String"]);
}

const POINT: &str = "struct Point {\n x: Int\n y: Int\n pub fn sum(self) -> Int { return self.x + self.y }\n pub fn origin() -> Point { return Point { x: 0, y: 0 } }\n}\n";

#[test]
fn a_struct_literal_names_its_fields_in_any_order() {
    let run = main(&format!("{POINT}fn f() -> Int {{\n let p = Point {{ x: 1, y: 2 }}\n let q = Point {{ y: 5, x: 4 }}\n return p.x + q.y\n}}"));
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let p (record Point x=1:Int y=2:Int)); (let q (record Point y=5:Int x=4:Int)); (return (add (.x p) (.y q)))");
}

#[test]
fn a_literal_takes_shorthand_defaults_and_a_base() {
    let run = main("struct Config {\n name: String\n retries: Int = 3\n verbose: Bool = false\n}\nfn f(name: String, base: Config) {\n let a = Config { name }\n let b = Config { ..base, retries: 5 }\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a (record Config name=name retries=3:Int verbose=false)); (let b (record Config ..base retries=5:Int))");
}

#[test]
fn a_literal_reports_missing_unknown_repeated_and_mistyped_fields() {
    let run = main(&format!("{POINT}fn f(a: Point) {{\n let b = Point {{ x: 1 }}\n let c = Point {{ x: 1, y: 2, z: 3 }}\n let d = Point {{ x: 1, x: 2, y: 3 }}\n let e = Point {{ x: \"s\", y: 1 }}\n let g = Point {{ y: 1, ..a }}\n}}"));
    assert_eq!(run.errors, ["missing field `y`", "Point has no field `z`", "`x` is given twice", "expected Int, found String", "`..base` comes first"]);
}

#[test]
fn fields_are_written_through_var_fields_and_var_paths() {
    let run = main("struct P {\n var x: Int\n y: Int\n}\nfn f(var p: P, q: P) {\n p.x = 1\n p.x += 2\n p.y = 3\n q.x = 4\n}");
    assert_eq!(run.errors, ["cannot assign: no `var` path to this place", "cannot assign: no `var` path to this place"]);
}

#[test]
fn methods_are_called_on_values_and_on_the_type() {
    let run = main(&format!("{POINT}fn f() -> Int {{\n let p = Point.origin()\n return p.sum()\n}}"));
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let p (call origin)); (return (call sum p))");
    assert_eq!(dump_method(&run, "Point", "sum"), "(return (add (.x self) (.y self)))");
    let bad = main(&format!("{POINT}fn f(p: Point) {{\n p.origin()\n Point.sum()\n p.missing()\n p.x()\n let a = p.sum\n}}"));
    assert_eq!(bad.errors, ["`origin` is called on the type", "`sum` is called on a value", "Point has no method `missing`", "Point has no method `x`", "`sum` is a method; call it"]);
}

#[test]
fn a_var_self_method_needs_a_var_path() {
    let run = main("class Counter {\n var n: Int = 0\n pub fn inc(var self) { self.n += 1 }\n pub fn peek(self) { self.n += 1 }\n}\nfn f() {\n var a = Counter()\n a.inc()\n let b = Counter()\n b.inc()\n}");
    assert_eq!(run.errors, ["cannot assign: no `var` path to this place", "this place has no `var` path to write through"]);
}

#[test]
fn construction_calls_new_or_builds_from_defaults() {
    let run = main("struct Point {\n x: Int\n y: Int\n pub fn new(x: Int, y: Int) -> Point { return Point { x, y } }\n}\nclass Cache {\n var hits: Int = 0\n}\nfn f() {\n let p = Point(1, 2)\n let c = Cache()\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let p (call new 1:Int 2:Int)); (let c (record Cache hits=0:Int))");
    let bad = main("struct P {\n x: Int\n}\nclass Cache {\n var hits: Int = 0\n}\nfn f() {\n let a = P()\n let b = Cache(1)\n}");
    assert_eq!(bad.errors, ["P has no `new` and `x` has no default", "Cache has no `new`"]);
}

#[test]
fn another_module_reads_pub_fields_and_calls_pub_methods_only() {
    let util = "pub struct Point {\n pub var x: Int\n y: Int\n pub fn show(self) {}\n fn hidden(self) {}\n}";
    let run = run(&[("util", util), ("main", "import { Point } from .util\nfn f(var p: Point) {\n let a = p.x\n let b = p.y\n p.show()\n p.hidden()\n p.x = 1\n let c = Point { x: 1, y: 2 }\n}")]);
    assert_eq!(run.errors, ["`y` is private", "`hidden` is private", "cannot assign: no `var` path to this place", "`y` is private"]);
}

#[test]
fn a_module_gives_its_functions_and_globals_by_qualified_name() {
    let run = run(&[("util", "pub fn make() -> Int { return 1 }\npub let LIMIT = 3\nfn secret() {}"), ("main", "import .util\nfn f() -> Int { return util.make() + util.LIMIT }")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(return (add (call make) $LIMIT))");
    let bad = run_with_missing();
    assert_eq!(bad.errors, ["`secret` is private in util", "no `nothing` in util"]);
}

fn run_with_missing() -> Run {
    run(&[("util", "pub fn make() -> Int { return 1 }\nfn secret() {}"), ("main", "import .util\nfn f() {\n util.secret()\n let a = util.nothing\n}")])
}

#[test]
fn a_module_name_is_not_a_value_and_its_function_is() {
    let run = run(&[("util", "pub fn make() -> Int { return 1 }"), ("main", "import .util\nfn f() {\n let a = util\n let b = util.make\n}")]);
    assert_eq!(run.errors, ["`util` is a module, not a value"]);
}

#[test]
fn another_module_writes_a_field_or_a_global_only_through_a_method_of_its_owner() {
    let util = "pub class Inner {\n pub var n: Int = 0\n pub fn bump(var self) { self.n += 1 }\n}\npub class Bag {\n pub var n: Int = 0\n pub var inner: Inner = Inner()\n pub fn add(var self) { self.inner.bump() }\n}\npub var counter = 0";
    let run = run(&[("util", util), ("main", "import .util\nimport { Bag } from .util\nfn f(var b: Bag) {\n b.n = 1\n b.inner.bump()\n b.add()\n util.counter = 5\n}")]);
    assert_eq!(run.errors, ["cannot assign: no `var` path to this place", "this place has no `var` path to write through", "cannot assign: no `var` path to this place", "only the module's own top-level code assigns a global"]);
}

#[test]
fn a_generic_struct_takes_its_arguments_from_the_context() {
    let run = main("struct Box<T> {\n value: T\n pub fn get(self) -> T { return self.value }\n}\nfn f(b: Box<Int>) -> Int { return b.get() }\nfn g() {\n let b: Box<Int> = Box { value: 1 }\n}\nfn h() {\n let c = Box { value: 1 }\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(return (call get b))");
    assert_eq!(dump(&run, "g"), "(let b (record Box<Int> value=1:Int))");
    assert_eq!(dump(&run, "h"), "(let c (record Box<Int> value=1:Int))");
}

#[test]
fn a_literal_finds_its_type_arguments_from_its_fields() {
    let run = main("struct Pair<A, B> {\n a: A\n b: B\n}\nenum Either<L, R> {\n Left(L)\n Right(R)\n}\nfn f() {\n let p = Pair { a: 1, b: \"x\" }\n let q = Pair { a: 2.5, b: true }\n let e: Either<Int, String> = Either.Left(1)\n}\nfn g() {\n let e = Either.Left(1)\n let p = Pair { a: 1 }\n}");
    assert_eq!(run.errors, ["cannot infer the generic arguments of `Either`", "missing field `b`"]);
    assert_eq!(dump(&run, "f"), "(let p (record Pair<Int, String> a=1:Int b=\"x\")); (let q (record Pair<Float, Bool> a=2.5:Float b=true)); (let e (variant Left 1:Int))");
}

#[test]
fn a_struct_holds_only_values_that_cannot_change_under_it() {
    let run = main("class Cell { var n: Int }\nclass Frozen { n: Int }\nstruct A { items: [Int] }\nstruct B { c: Cell }\nstruct C { f: Frozen }\nstruct D { next: D? }\nstruct E { d: D }\nclass N { next: N? }\nstruct T { a: (Int, [Int]) }\nstruct Vec3 { data: [Float; 3] }\nstruct BadFixed { cells: [Cell; 2] }\nstruct BadSelfFixed { arr: [BadSelfFixed; 2] }\nfn rep() -> [Vec3] { return [Vec3 { data: [1.0, 2.0, 3.0] }; 2] }");
    assert_eq!(run.errors, ["a struct cannot hold [Int]", "a struct cannot hold Cell", "struct `D` contains itself", "a struct cannot hold (Int, [Int])", "a struct cannot hold [Cell; 2]", "struct `BadSelfFixed` contains itself"]);
}

#[test]
fn a_field_and_a_method_do_not_share_a_name() {
    let run = main("struct P {\n x: Int\n fn x(self) {}\n}");
    assert_eq!(run.errors, ["`x` is both a field and a method"]);
}

const SHAPE: &str = "enum Shape {\n Circle(Int)\n Rect(Int, Int)\n Named { name: String, sides: Int }\n Empty\n}\n";

#[test]
fn variants_are_built_qualified_or_bare_where_the_type_is_known() {
    let run = main(&format!("{SHAPE}fn make() -> Shape {{\n let a = Shape.Circle(2)\n let b: Shape = Rect(1, 2)\n let c: Shape = Empty\n let d = Shape.Named {{ name: \"a\", sides: 3 }}\n return Named {{ name: \"b\", sides: 4 }}\n}}"));
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(
        dump(&run, "make"),
        "(let a (variant Circle 2:Int)); (let b (variant Rect 1:Int 2:Int)); (let c (variant Empty)); (let d (variant Named \"a\" 3:Int)); (return (variant Named \"b\" 4:Int))"
    );
    let bad = main(&format!("{SHAPE}fn f() {{\n let a = Circle(1)\n let b = Empty\n let c: Shape = Shape.Circle\n let d: Shape = Circle(1, 2)\n let e: Shape = Empty(1)\n let g: Shape = Shape.Oval(1)\n let h: Shape = Named(1)\n}}"));
    assert_eq!(
        bad.errors,
        ["unknown name `Circle`", "unknown name `Empty`", "`Circle` has a payload", "expected 1 arguments, found 2", "`Empty` has no payload", "Shape has no method `Oval`", "`Named` is built with fields"]
    );
}

#[test]
fn match_takes_every_variant_and_arms_that_return_end_the_function() {
    let run = main(&format!("{SHAPE}fn area(s: Shape) -> Int {{\n match s {{\n Circle(r) => return 3 * r * r\n Rect(w, h) => return w * h\n Named {{ sides, .. }} => return sides\n Empty => return 0\n }}\n}}"));
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(
        dump(&run, "area"),
        "(match s [Circle(0:r) => {(return (mul (mul 3:Int r) r))}, Rect(0:w, 1:h) => {(return (mul w h))}, Named(1:sides) => {(return sides)}, Empty => {(return 0:Int)}])"
    );
}

#[test]
fn a_match_must_cover_its_subject() {
    let run = main(&format!(
        "{SHAPE}fn f(s: Shape, b: Bool, n: Int, o: Int??, t: (Bool, Bool)) {{\n match s {{\n Circle(r) => {{}}\n Empty => {{}}\n }}\n match s {{\n Circle(r) if r > 1 => {{}}\n Rect(w, h) => {{}}\n Named {{ .. }} => {{}}\n Empty => {{}}\n }}\n match b {{\n true => {{}}\n }}\n match n {{\n 0 => {{}}\n 1..=5 => {{}}\n }}\n match o {{\n Some(Some(x)) => {{}}\n None => {{}}\n }}\n match t {{\n (true, _) => {{}}\n (false, true) => {{}}\n }}\n match s {{\n Circle(_) | Rect(_, _) => {{}}\n Named {{ .. }} | Empty => {{}}\n }}\n match n {{\n 0 => {{}}\n other => {{}}\n }}\n}}"
    ));
    assert_eq!(
        run.errors,
        ["match does not cover `Rect(_, _)`", "match does not cover `Circle(_)`", "match does not cover `false`", "match does not cover `_`", "match does not cover `Some(None)`", "match does not cover `(false, false)`"]
    );
}

#[test]
fn patterns_match_literals_ranges_alternatives_tuples_and_optionals() {
    let run = main("fn f(n: Int, t: (Int, String), o: Int?, s: String) -> Int {\n match n {\n 0 => return 1\n 1 | 2 => return 2\n d @ 3..=9 => return d\n x if x > 100 => return 4\n _ => return 0\n }\n match o {\n Some(v) => return v\n None => return 0\n }\n match t {\n (0, text) => {}\n (x, _) => {}\n }\n match s {\n \"a\" => {}\n _ => {}\n }\n return 0\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(
        dump(&run, "f").split("; ").next().unwrap(),
        "(match n [0 => {(return 1:Int)}, 1 | 2 => {(return 2:Int)}, d@3..=9 => {(return d)}, x if (gt x 100:Int) => {(return 4:Int)}, _ => {(return 0:Int)}])"
    );
}

#[test]
fn bad_patterns_are_reported() {
    let run = main(&format!(
        "{SHAPE}fn f(s: Shape, n: Int, v: Int | String, t: (Int, Int)) {{\n match n {{\n \"a\" => {{}}\n 300 => {{}}\n _ => {{}}\n }}\n match s {{\n Oval => {{}}\n Circle(a, b) => {{}}\n Empty(x) => {{}}\n Circle => {{}}\n Named {{ name }} => {{}}\n Rect {{ w }} => {{}}\n _ => {{}}\n }}\n match n {{\n Circle(r) => {{}}\n _ => {{}}\n }}\n match v {{\n b: Bool => {{}}\n _ => {{}}\n }}\n match t {{\n (a, b, c) => {{}}\n (a | b, c) => {{}}\n }}\n}}"
    ));
    assert_eq!(
        run.errors,
        [
            "a string pattern does not match Int",
            "Shape has no variant `Oval`",
            "`Circle` has 1 fields",
            "`Empty` has no tuple payload",
            "`Circle` has a payload",
            "the pattern leaves out `sides`",
            "`Rect` has no named fields",
            "Int has no variants",
            "String | Int is never Bool",
            "a tuple pattern of 3 elements does not match (Int, Int)",
            "`b` is not bound by every alternative",
            "`a` is not bound by every alternative",
        ]
    );
}

#[test]
fn a_let_pattern_must_always_match() {
    let run = main(&format!("{POINT}fn f(o: Int?, p: Point, t: (Int, Int)) {{\n let (a, b) = t\n let Point {{ x, y }} = p\n let Some(n) = o\n let 3 = a\n}}"));
    assert_eq!(run.errors, ["this pattern can fail; use `if let` or `match`", "this pattern can fail; use `if let` or `match`"]);
}

#[test]
fn if_let_and_while_let_bind_in_their_body() {
    let run = main(&format!(
        "{POINT}fn f(o: Int?, p: Point) -> Int {{\n if let Some(x) = o {{\n return x\n }} else if let None = o {{\n return 0\n }}\n if let Point {{ x: 0, y }} = p {{\n return y\n }}\n let n = if let Some(x) = o {{ x }} else {{ 0 }}\n while let Some(x) = o {{\n break\n }}\n return n\n}}\nfn g(o: Int?) -> Int {{\n if let Some(x) = o {{ }}\n return x\n}}"
    ));
    assert_eq!(run.errors, ["unknown name `x`"]);
    assert_eq!(let_type(&run, "f", 0), "Int");
}

#[test]
fn a_value_match_joins_its_arms() {
    let run = main(&format!("{SHAPE}fn f(s: Shape) {{\n let n = match s {{\n Circle(r) => r\n Rect(w, h) => w + h\n _ => 0\n }}\n let m = match s {{\n Empty => null\n _ => 1\n }}\n let k = match s {{\n Empty => 1\n _ => \"a\"\n }}\n}}"));
    assert_eq!(run.errors, ["branches have different types: Int and String"]);
    assert_eq!(let_type(&run, "f", 0), "Int");
    assert_eq!(let_type(&run, "f", 1), "Int?");
}

#[test]
fn a_generic_enum_takes_its_arguments_from_the_context() {
    let run = main("enum Either<L, R> {\n Left(L)\n Right(R)\n}\nfn f() -> Int {\n let e: Either<Int, String> = Either.Left(1)\n let g = Either<Int, String>.Right(\"a\")\n match e {\n Left(n) => return n\n Right(s) => return 0\n }\n}\nfn h() {\n let x = Either.Left(1)\n}");
    assert_eq!(run.errors, ["cannot infer the generic arguments of `Either`"]);
}

#[test]
fn enum_methods_take_self_and_compare_by_variant() {
    let run = main("enum Light {\n Red\n Green\n pub fn next(self) -> Light {\n return match self {\n Red => Green\n Green => Red\n }\n }\n}\nfn f(a: Light) -> Bool {\n let b = Light.Red.next()\n return a == b\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump_method(&run, "Light", "next"), "(return (match self [Red => (variant Green), Green => (variant Red)]))");
    assert_eq!(dump(&run, "f"), "(let b (call next (variant Red))); (return (eq a b))");
}

#[test]
fn alternatives_share_their_names_and_a_lowercase_name_that_is_no_variant_binds() {
    let run = main(&format!("{SHAPE}fn f(t: (Int, Int), s: Shape) -> Int {{\n match s {{\n Empty => {{}}\n other => {{}}\n }}\n match t {{\n (0, x) | (x, 0) => return x\n _ => return 0\n }}\n}}"));
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f").split("; ").nth(1).unwrap(), "(match t [(0, x) | (x, 0) => {(return x)}, _ => {(return 0:Int)}])");
}

#[test]
fn a_qualified_variant_pattern_names_the_subjects_enum() {
    let run = main(&format!("{SHAPE}enum Other {{\n Circle(Int)\n}}\nfn f(s: Shape) {{\n match s {{\n Shape.Circle(r) => {{}}\n Other.Circle(r) => {{}}\n _ => {{}}\n }}\n}}"));
    assert_eq!(run.errors, ["expected Shape, found Other"]);
}

#[test]
fn type_patterns_cover_a_union_by_member() {
    let run = main("fn f(v: Int | String) -> Int {\n match v {\n n: Int => return n\n s: String => return 0\n }\n}\nfn g(v: Int | String) {\n match v {\n n: Int => {}\n }\n}\nfn h(v: Int | String | Bool) {\n match v {\n _: (Int | String) => {}\n _: Bool => {}\n }\n}");
    assert_eq!(run.errors, ["match does not cover `_: String`"]);
}

#[test]
fn a_test_narrows_a_let_name_in_the_branch_it_guards() {
    let run = main("fn f(x: Int?, v: Int | String) -> Int {\n if x != null { let a = x + 1 }\n if x == null { return 0 }\n let b = x + 1\n if v is Int { let c = v + 1 }\n if !(v is Int) { let d = v + \"s\" }\n return b\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(
        dump(&run, "f"),
        "(if (noteq x null) {(let a (add (narrow x) 1:Int))}); (if (eq x null) {(return 0:Int)}); (let b (add (narrow x) 1:Int)); (if (is v Int) {(let c (add (narrow v) 1:Int))}); (if (not (is v Int)) {(let d (add (narrow v) \"s\"))}); (return b)"
    );
    let bad = main("fn f(x: Int?) {\n let a = x + 1\n}");
    assert_eq!(bad.errors, ["`+` needs one type on both sides, found Int? and Int"]);
}

#[test]
fn narrowing_follows_and_or_else_loops_and_early_exits() {
    let run = main(
        "fn f(x: Int?, n: Int) -> Int {\n if x != null && x > 1 { return 1 }\n if x == null || x > 1 { return 2 }\n if x == null { } else { let a = x + 1 }\n while x != null { let b = x + 1\n break }\n let c = if x != null { x + 1 } else { 0 }\n var i = 0\n while i < n {\n i += 1\n if x == null { continue }\n let d = x + 1\n }\n return c\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
}

#[test]
fn only_a_let_name_narrows() {
    let run = main("struct P {\n n: Int?\n}\nfn f(var x: Int?, p: P, y: Int?) {\n if x != null { let a = x + 1 }\n if p.n != null { let b = p.n + 1 }\n if y != null || y > 1 { }\n}");
    assert_eq!(
        run.errors,
        ["`+` needs one type on both sides, found Int? and Int", "`+` needs one type on both sides, found Int? and Int", "`>` needs one type on both sides, found Int? and Int"]
    );
}

#[test]
fn coalesce_takes_the_payload_or_the_default() {
    let run = main("fn f(x: Int?, y: Int?) {\n let a = x ?? 0\n let b = x ?? y\n let c = x ?? null\n let d = x ?? 1 + 1\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 0), "Int");
    assert_eq!(let_type(&run, "f", 1), "Int?");
    assert_eq!(let_type(&run, "f", 2), "Int?");
    assert_eq!(dump(&run, "f").split("; ").next().unwrap(), "(let a (match x [Some(?) => ?, None => 0:Int]))");
    let bad = main("fn f(x: Int?, n: Int) {\n let a = n ?? 1\n let b = x ?? \"s\"\n}");
    assert_eq!(bad.errors, ["`??` needs an optional on the left, found Int", "expected Int?, found String"]);
}

#[test]
fn an_optional_member_is_none_when_the_object_is() {
    let run = main("class Node {\n next: Node?\n name: String\n pub fn size(self) -> Int { return 1 }\n}\nfn f(n: Node?) {\n let a = n?.name\n let b = n?.next\n let c = n?.size()\n let d = n?.next?.name ?? \"none\"\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 0), "String?");
    assert_eq!(let_type(&run, "f", 1), "Node?");
    assert_eq!(let_type(&run, "f", 2), "Int?");
    assert_eq!(let_type(&run, "f", 3), "String");
    let bad = main("struct Node {\n name: String\n}\nfn f(n: Node?, m: Node) {\n let a = n.name\n let b = m?.name\n}");
    assert_eq!(bad.errors, ["Node? has no fields", "`?.` needs an optional, found Node"]);
}

#[test]
fn propagate_and_unwrap_need_a_matching_function() {
    let result = ("std.result", "pub enum Result<T, E> {\n Ok(T)\n Err(E)\n}");
    let run = run(&[
        result,
        (
            "main",
            "import Result from .std.result\nfn half(n: Int) -> Int? { return null }\nfn quarter(n: Int) -> Int? {\n let h = half(n)?\n return half(h)\n}\nfn unwrapped(n: Int) -> Int { return half(n)! }\nfn parse(s: String) -> Result<Int, String> { return Err(\"x\") }\nfn both(s: String) -> Result<Int, String> {\n let n = parse(s)?\n return Ok(n)\n}\nfn plain(s: String) -> Int { return parse(s)! }",
        ),
    ]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "quarter"), "(let h (try (call half n))); (return (call half h))");
    assert_eq!(let_type(&run, "quarter", 0), "Int");
    let bad = main("fn half(n: Int) -> Int? { return null }\nfn f(n: Int) -> Int {\n let h = half(n)?\n return h\n}\nfn g(n: Int) {\n let a = n?\n let b = n!\n}");
    assert_eq!(bad.errors, ["`?` on an optional needs a function returning an optional", "`?` needs an optional or a Result, found Int", "`!` needs an optional or a Result, found Int"]);
    let wrong = run_result("fn parse(s: String) -> Result<Int, String> { return Err(\"x\") }\nfn f(s: String) -> Result<Int, Int> {\n let n = parse(s)?\n return Ok(n)\n}\nfn g(s: String) -> Result<Int, String | Int> {\n let n = parse(s)?\n return Ok(n)\n}\nfn h(s: String) -> Int? {\n let n = parse(s)?\n return n\n}\nfn k() {\n let r = Ok(1)\n}");
    assert_eq!(wrong.errors, ["the error String does not fit Int", "`?` on a Result needs a function returning a Result", "write the type arguments of `Result`"]);
    let top = main("fn half(n: Int) -> Int? { return null }\nlet a = half(1)?");
    assert_eq!(top.errors, ["`?` on an optional needs a function returning an optional"]);
}

fn run_result(text: &str) -> Run {
    let source = format!("import Result from .std.result\n{text}");
    run(&[("std.result", "pub enum Result<T, E> {\n Ok(T)\n Err(E)\n}"), ("main", &source)])
}

#[test]
fn is_asks_about_a_type_the_value_can_have() {
    let run = main("fn f(n: Int, v: Int | String, o: Int?) {\n let a = v is Int\n let b = v is (Int | String)\n let c = o is Int\n let d = n is String\n}");
    assert_eq!(run.errors, ["Int is never String"]);
}

#[test]
fn some_and_none_make_optionals() {
    let run = main("fn f() {\n let a: Int? = Some(1)\n let b = Some(2)\n let c: Int? = None\n let d: Int?? = Some(None)\n let e: UInt8? = Some(7)\n let g = None\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 0), "Int?");
    assert_eq!(let_type(&run, "f", 1), "Int?");
    assert_eq!(dump(&run, "f").split("; ").nth(4).unwrap(), "(let e (to-optional 7:UInt8))");
}

#[test]
fn coalesce_assignment_needs_an_optional_var_place() {
    let run = main("fn f(var o: Int?, p: Int?, var n: Int) {\n o ??= 5\n p ??= 1\n n ??= 2\n}");
    assert_eq!(run.errors, ["cannot assign: no `var` path to this place", "`??=` needs an optional place, found Int"]);
}

#[test]
fn built_in_types_get_their_methods_from_std() {
    let std = "extend <T> Option<T> {\n pub fn is_some(self) -> Bool {\n return match self {\n Some(_) => true\n None => false\n }\n }\n}\nextend Int {\n pub fn double(self) -> Int { return self * 2 }\n pub fn parse(s: String) -> Int { return 0 }\n}";
    let run = run(&[("std.types", std), ("main", "fn f(x: Int?, n: Int) -> Bool {\n let a = n.double()\n let b = Int.parse(\"1\")\n return x.is_some()\n}")]);
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(dump(&run, "f"), "(let a (call double n)); (let b (call parse \"1\")); (return (call is_some x))");
}

#[test]
fn a_generic_call_finds_its_type_arguments_from_the_arguments() {
    let run = main(
        "struct Wrap<T> { pub v: T }\nfn id<T>(x: T) -> T { return x }\nfn pair<A, B>(a: A, b: B) -> (A, B) { return (a, b) }\nfn first<T>(xs: [T]) -> T? { return null }\nfn some<T>(x: T?) -> T? { return x }\nfn unwrap<T>(w: Wrap<T>) -> T { return w.v }\nfn f(xs: [Int], w: Wrap<String>) {\n let a = id(1)\n let b = id(\"x\")\n let c = pair(1, \"x\")\n let d = first(xs)\n let e = id<String>(\"y\")\n let g = id(id(2))\n let h = some(1)\n let i = unwrap(w)\n let j = id(2.5)\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    let types: Vec<String> = (0..9).map(|n| let_type(&run, "f", n)).collect();
    assert_eq!(types, ["Int", "String", "(Int, String)", "Int?", "String", "Int", "Int?", "String", "Float"]);
    assert_eq!(dump(&run, "f").split("; ").next().unwrap(), "(let a (call id<Int> 1:Int))");
}

#[test]
fn a_lambda_without_a_value_fixes_a_result_variable_to_null() {
    let run = main("fn once<R>(work: () -> R) -> R { return work() }\nfn f() {\n once(|| { })\n let n = once(|| 3)\n}");
    assert_eq!(run.errors, Vec::<String>::new());
}

#[test]
fn an_expected_type_fixes_what_the_arguments_leave_open() {
    let run = main("fn make<T>() -> T? { return null }\nfn f() {\n let a: Int? = make()\n let b = make()\n}");
    assert_eq!(run.errors, ["cannot infer the generic argument `T`"]);
    assert_eq!(dump(&run, "f").split("; ").next().unwrap(), "(let a (call make<Int>))");
}

#[test]
fn bad_type_arguments_are_reported() {
    let run = main("fn same<T>(a: T, b: T) -> T { return a }\nfn id<T>(x: T) -> T { return x }\nfn f() {\n same(1, \"x\")\n id<Int, Int>(1)\n id<Int>(\"x\")\n}");
    assert_eq!(run.errors, ["expected Int, found String", "`id` takes 1 type argument", "expected Int, found String"]);
}

#[test]
fn a_trait_takes_type_arguments_in_bounds_and_lists() {
    let run = main(
        "trait Convert<T> {\n fn convert(self) -> T\n}\nstruct A: (Convert<Int>) { pub fn convert(self) -> Int { return 1 } }\nstruct B: (Convert<String>) { pub fn convert(self) -> Int { return 1 } }\nstruct C {}\nstruct D: (Convert) {}\nfn take<U: Convert<Int>>(u: U) -> Int { return u.convert() }\nfn takes<U: Convert<String>>(u: U) {}\nfn bad<U: Convert<Int, Int>>(u: U) {}\nfn f(a: A, c: C) {\n take(a)\n take(c)\n takes(a)\n}",
    );
    assert_eq!(
        run.errors,
        [
            "`Convert` takes 1 type argument",
            "write the type arguments of `Convert`",
            "`B.convert` does not match `Convert<String>.convert`",
            "C does not satisfy `Convert<Int>`: it has no method `convert`",
            "A does not satisfy `Convert<String>`: `convert` does not match the trait's"
        ]
    );
    assert_eq!(dump(&run, "take"), "(return (call convert@U u))");
}

#[test]
fn a_default_method_of_a_generic_trait_reads_the_listed_arguments() {
    let run = main(
        "trait Convert<T> {\n fn convert(self) -> T\n fn twice(self) -> T {\n let first: T = self.convert()\n return first\n }\n}\nstruct A: (Convert<Int>) { pub fn convert(self) -> Int { return 1 } }\nstruct B: (Convert<String>) { pub fn convert(self) -> String { return \"b\" } }\nfn f(a: A, b: B) {\n let n = a.twice()\n let s = b.twice()\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 0), "Int");
    assert_eq!(let_type(&run, "f", 1), "String");
}

#[test]
fn a_types_own_bounds_are_checked_where_it_is_used() {
    let run = main(
        "trait Show { fn show(self) -> String }\nstruct A { pub fn show(self) -> String { return \"a\" } }\nstruct B {}\nstruct Box<T: Show> {\n value: T\n pub fn text(self) -> String { return self.value.show() }\n}\nfn f(ok: Box<A>, bad: Box<B>) -> Box<B>? { return null }\nfn g() {\n let a: Box<B>? = null\n let b = Box { value: B() }\n let c = Box { value: A() }\n}",
    );
    assert_eq!(
        run.errors,
        [
            "B does not satisfy `Show`: it has no method `show`",
            "B does not satisfy `Show`: it has no method `show`",
            "B does not satisfy `Show`: it has no method `show`",
            "B does not satisfy `Show`: it has no method `show`"
        ]
    );
}

#[test]
fn a_bound_is_met_by_a_type_with_the_methods_and_checked_at_each_call() {
    let run = main(
        "trait Show { fn show(self) -> String }\ntrait Ord { fn compare(self, other: Self) -> Int }\nstruct A { pub fn show(self) -> String { return \"a\" } }\nstruct B {}\nstruct C { pub fn show(self) -> Int { return 1 } }\nfn put<T: Show>(x: T) -> String { return x.show() }\nfn larger<T: Ord>(a: T, b: T) -> T {\n if a.compare(b) > 0 { return a }\n return b\n}\nfn f(a: A, b: B, c: C) {\n put(a)\n put(b)\n put(c)\n put(1)\n}",
    );
    assert_eq!(
        run.errors,
        ["B does not satisfy `Show`: it has no method `show`", "C does not satisfy `Show`: `show` does not match the trait's", "Int does not satisfy `Show`: it has no method `show`"]
    );
    assert_eq!(dump(&run, "put"), "(return (call show@T x))");
    assert_eq!(dump(&run, "larger").split("; ").next().unwrap(), "(if (gt (call compare@T a b) 0:Int) {(return a)})");
}

#[test]
fn a_bound_needs_methods_the_caller_sees_and_a_pub_trait_is_imported_like_a_type() {
    let util = "pub trait Show { fn show(self) -> String }\npub struct Open { pub fn show(self) -> String { return \"o\" } }\npub struct Hidden { fn show(self) -> String { return \"h\" } }";
    let main = "import { Show, Open, Hidden } from .util\nstruct Mine: (Show) {\n fn show(self) -> String { return \"m\" }\n}\nfn put<T: Show>(x: T) -> String { return x.show() }\nfn f() {\n put(Open {})\n put(Hidden {})\n put(Mine {})\n}";
    let run = run(&[("util", util), ("main", main)]);
    assert_eq!(run.errors, ["Hidden does not satisfy `Show`: it has no method `show`"]);
}

#[test]
fn a_type_parameter_has_only_the_methods_of_its_bounds() {
    let run = main("trait Show { fn show(self) -> String }\nfn put<T>(x: T) -> String { return x.show() }\nfn bound<T: Show>(x: T) -> Int { return x.size() }");
    assert_eq!(run.errors, ["T has no method `show`", "T has no method `size`"]);
}

#[test]
fn a_type_parameter_is_comparable_to_null_and_to_itself_only_with_eq() {
    let run = run(&[
        ("std.traits", "pub trait Eq { fn eq(self, other: Self) -> Bool }"),
        ("main", "import Eq from .std.traits\nfn none<T>(x: T) -> Bool { return x == null }\nfn good<T: Eq>(a: T, b: T) -> Bool { return a == b }\nfn bad<T>(a: T, b: T) -> Bool { return a == b }"),
    ]);
    assert_eq!(run.errors, ["`==` does not work on T"]);
}

#[test]
fn a_method_takes_its_types_arguments_and_its_own() {
    let run = main(
        "class Box<T> {\n v: T\n pub fn new(v: T) -> Self { return Box { v: v } }\n pub fn pair<U>(self, other: U) -> (T, U) { return (self.v, other) }\n}\nfn f() {\n let b = Box(1)\n let p = b.pair(\"x\")\n let q = Box<String>(\"y\")\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    let types: Vec<String> = (0..3).map(|n| let_type(&run, "f", n)).collect();
    assert_eq!(types, ["Box<Int>", "(Int, String)", "Box<String>"]);
}

#[test]
fn a_method_call_accepts_explicit_type_arguments() {
    let run = main(
        "class Box<T> {\n v: T\n pub fn new(v: T) -> Self { return Box { v: v } }\n pub fn pair<U>(self, other: U) -> (T, U) { return (self.v, other) }\n}\nfn f() {\n let b = Box(1)\n let p = b.pair<String>(\"x\")\n let opt: Box<Int>? = b\n let q = opt?.pair<String>(\"y\")\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 1), "(Int, String)");
    assert_eq!(let_type(&run, "f", 3), "(Int, String)?");
}

#[test]
fn a_struct_method_accepts_explicit_type_arguments() {
    let run = main(
        "struct Pair<A> {\n a: A\n pub fn map<B>(self, b: B) -> (A, B) { return (self.a, b) }\n}\nfn f() {\n let p = Pair { a: 42 }\n let r = p.map<String>(\"hello\")\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 1), "(Int, String)");
}

#[test]
fn a_static_method_accepts_explicit_type_arguments() {
    let run = main(
        "class Factory {\n pub fn create<T>(val: T) -> T { return val }\n}\nfn f() {\n let x = Factory.create<Int>(10)\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 0), "Int");
}

#[test]
fn method_type_argument_count_mismatch_is_reported() {
    let run = main(
        "class Box<T> {\n v: T\n pub fn new(v: T) -> Self { return Box { v: v } }\n pub fn pair<U>(self, other: U) -> (T, U) { return (self.v, other) }\n}\nfn f() {\n let b = Box(1)\n b.pair<String, Int>(\"x\")\n}",
    );
    assert_eq!(run.errors, ["`pair` takes 1 type argument"]);
}

#[test]
fn cast_test() {
    let run = main("class T<V> {\n val: V\n pub fn new(val: V) -> T<V> { return T { val: val } }\n pub fn cast<U>(self) -> Int { return 10 }\n}\nfn main() {\n let obj = T(\"String\")\n let v = obj.cast<String>()\n}");
    assert_eq!(run.errors, Vec::<String>::new());
}

#[test]
fn trait_methods_cannot_have_type_parameters() {
    let run = main(
        "trait Transform {\n fn transform<T>(self, val: T) -> T\n}",
    );
    assert_eq!(run.errors, ["trait methods cannot have type parameters"]);
}

#[test]
fn trailing_arguments_of_a_variadic_call_make_a_list() {
    let run = main("fn sum(...xs: Int) -> Int {\n let n: List<Int> = xs\n return 0\n}\nfn count<T>(...xs: T) -> Int { return 0 }\nfn f() {\n sum()\n sum(1, 2, 3)\n count(\"a\", \"b\")\n count()\n sum(1, \"x\")\n sum(xs=1)\n}");
    assert_eq!(run.errors, ["cannot infer the generic argument `T`", "expected Int, found String", "a variadic function takes no named arguments"]);
    assert_eq!(dump(&run, "f").split("; ").nth(1).unwrap(), "(call sum (list 1:Int 2:Int 3:Int))");
}

#[test]
fn a_generic_function_is_a_value_when_its_type_arguments_are_known() {
    let run = main("fn id<T>(x: T) -> T { return x }\nfn f() {\n let a: (Int) -> Int = id\n let c = id\n}");
    assert_eq!(run.errors, ["cannot infer the generic argument `T`"]);
    assert_eq!(let_type(&run, "f", 0), "(Int) -> Int");
}

#[test]
fn from_is_picked_by_the_argument_type() {
    let run = main(
        "struct Id {\n n: Int\n pub fn from(n: Int) -> Id { return Id { n: n } }\n pub fn from(s: String) -> Id { return Id { n: 0 } }\n}\nfn f() {\n let a = Id.from(1)\n let b = Id.from(\"x\")\n let c = Id.from(true)\n}",
    );
    assert_eq!(run.errors, ["Id has no `from` for Bool"]);
    let callee = |n: usize| {
        let body = &run.program.checked[&run.def("f")];
        let init = body.block.stmts.iter().filter_map(|&stmt| match &body.stmts[stmt].kind {
            StmtKind::Let { init, .. } => Some(*init),
            _ => None,
        });
        match &body.exprs[init.clone().nth(n).unwrap()].kind {
            ExprKind::Call { callee: Callee::Function(function), .. } => function.def,
            other => panic!("{other:?}"),
        }
    };
    assert_ne!(callee(0), callee(1));
}

const COLLECTIONS: &str = "pub class Map<K, V> {}\npub class Set<T> {}\npub struct Range<T> {}";

/// Runs `text` as `main` with std's collections imported.
fn with_collections(text: &str) -> Run {
    run(&[("std.collections", COLLECTIONS), ("main", &format!("import Map from .std.collections\n{text}"))])
}

#[test]
fn list_and_array_literals_take_their_shape_from_the_context() {
    let run = with_collections(
        "fn f() {\n let a = [1, 2, 3]\n let b: List<UInt8> = [1, 2]\n let c: [Int] = [1, 2]\n let d: [Int; 2] = [1, 2]\n let e: List<Int | String> = [1, \"a\"]\n let g = [1, null]\n let h: List<Int>? = [1]\n let i: List<Int> = []\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    let types: Vec<String> = (0..8).map(|n| let_type(&run, "f", n)).collect();
    assert_eq!(types, ["List<Int>", "List<UInt8>", "[Int]", "[Int; 2]", "List<String | Int>", "List<Int?>", "List<Int>?", "List<Int>"]);
}

#[test]
fn bad_literals_are_reported() {
    let run = with_collections(
        "fn f() {\n let a = []\n let b = [1, \"a\"]\n let c: [Int; 3] = [1, 2]\n let e = {\"a\": 1, \"b\": \"x\"}\n let g: Map<String, Int> = {1: 2}\n}",
    );
    assert_eq!(
        run.errors,
        ["an empty list needs its type from context", "elements have different types: Int and String", "expected 3 elements, found 2", "values have different types: Int and String", "expected String, found Int"]
    );
}

#[test]
fn an_array_repeat_copies_a_value_and_fixes_its_length_by_the_annotation() {
    let run = with_collections("fn f(n: Int) {\n let a: [Int] = [0; n]\n let b: [UInt8; 4] = [0; 4]\n let c = [1; 3]\n let d: [Int; 4] = [0; n]\n let e: [Int; 4] = [0; 5]\n let g = [[1]; 2]\n}");
    assert_eq!(run.errors, ["the length must be the literal 4", "the length must be the literal 4", "`[v; n]` copies its value, and List<Int> can change"]);
    assert_eq!(let_type(&run, "f", 0), "[Int]");
    assert_eq!(let_type(&run, "f", 1), "[UInt8; 4]");
    assert_eq!(let_type(&run, "f", 2), "[Int]");
}

#[test]
fn maps_take_their_types_from_the_entries_or_the_context() {
    let run = with_collections("fn f() {\n let a = {\"a\": 1}\n let b: Map<String, UInt8> = {\"a\": 1}\n let c: Map<String, Int> = {}\n let d = Map<String, Int>()\n let e: List<Int> = [1]\n let g = a == a\n let h = e == [1]\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    assert_eq!(let_type(&run, "f", 3), "Map<String, Int>");
    assert_eq!(let_type(&run, "f", 5), "Bool");
    assert_eq!(let_type(&run, "f", 0), "Map<String, Int>");
    assert_eq!(let_type(&run, "f", 1), "Map<String, UInt8>");
    assert_eq!(let_type(&run, "f", 2), "Map<String, Int>");
}

#[test]
fn indexing_slicing_and_ranges_type_their_results() {
    let run = with_collections(
        "fn f(xs: List<Int>, a: [Int], m: Map<String, Int>, s: String, var ys: List<Int>, var mm: Map<String, Int>, fx: [Int; 3]) {\n let x = xs[0]\n let y = a[1]\n let z = m[\"k\"]\n let t = xs[0..2]\n let u = s[1..]\n let v = xs[..=1]\n let w = xs[0..4..2]\n let r = 0..3\n let p = a[0..1]\n let q = fx[1..]\n let o = a[0..4..2]\n ys[0] = 1\n mm[\"b\"] += 1\n ys[1..2] = 0\n ys[0..2] = [1, 2]\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    let types: Vec<String> = (0..11).map(|n| let_type(&run, "f", n)).collect();
    assert_eq!(types, ["Int", "Int", "Int", "List<Int>", "String", "List<Int>", "List<Int>", "Range<Int>", "[Int]", "[Int]", "[Int]"]);
}

#[test]
fn bad_indexing_slicing_and_ranges_are_reported() {
    let run = with_collections(
        "fn f(xs: List<Int>, a: [Int], s: String, var ys: List<Int>) {\n let b = s[0]\n let d = s[0..2..1]\n let e = 1[0]\n let g = 0..true\n xs[0] = 1\n ys[0..2] += 1\n ys[0..2..1] = 0\n}",
    );
    assert_eq!(
        run.errors,
        [
            "a String cannot be indexed; use `slice`",
            "a String slice takes no step",
            "Int cannot be indexed",
            "a range needs integers of one type, found Int and Bool",
            "cannot assign: no `var` path to this place",
            "a slice is only assigned with `=`",
            "only a `List` slice without a step is assigned",
        ]
    );
}

#[test]
fn for_walks_ranges_lists_arrays_and_receivers() {
    let run = with_collections(
        "fn f(xs: List<Int>, a: [Int; 2], m: Map<String, Int>, pairs: List<(Int, String)>, rx: Receiver<Int>) {\n for i in 0..3 { let j = i + 1 }\n for x in xs { let y = x + 1 }\n for x in a { let y = x + 1 }\n for (n, s) in pairs { let t = n + 1\n let u = s + \"\" }\n for v in rx { let w = v + 1 }\n for k in m {}\n for 3 in xs {}\n for x in 5 {}\n while true { break 1 }\n}",
    );
    assert_eq!(run.errors, ["Map<String, Int> is not iterable", "this pattern can fail; use `match` inside the loop", "Int is not iterable", "`break` takes no value"]);
}

#[test]
fn a_string_part_calls_to_string() {
    let run = with_collections("trait Display { fn to_string(self) -> String }\nstruct P { pub fn to_string(self) -> String { return \"p\" } }\nstruct Q {}\nfn f(p: P) -> String { return \"p=${p}\" }\nfn g<T: Display>(x: T) -> String { return \"${x}\" }\nfn h<T>(x: T) -> String { return \"${x}\" }\nfn i(q: Q) -> String { return \"${q}\" }");
    assert_eq!(run.errors, ["T has no method `to_string`", "Q has no method `to_string`"]);
}

#[test]
fn a_lambda_takes_its_parameter_types_from_the_context() {
    let run = with_collections(
        "fn apply(f: (Int) -> Int, x: Int) -> Int { return f(x) }\nfn map<T, U>(xs: List<T>, f: (T) -> U) -> List<U> { return [] }\nfn f(xs: List<Int>) {\n let inc = |n: Int| n + 1\n let twice: (Int) -> Int = |n| n * 2\n let add = |a: Int, b: Int| -> Int { return a + b }\n let hi = || {}\n let r = apply(|n| n * 2, 4)\n let ys = map(xs, |n| n + 1)\n let zs = map(xs, |n| \"x\")\n let g = |n: Int| { if n > 0 { return 1 }\n return 2 }\n}",
    );
    assert_eq!(run.errors, Vec::<String>::new());
    let types: Vec<String> = (0..8).map(|n| let_type(&run, "f", n)).collect();
    assert_eq!(types, ["(Int) -> Int", "(Int) -> Int", "(Int, Int) -> Int", "() -> Null", "Int", "List<Int>", "List<String>", "(Int) -> Int"]);
}

#[test]
fn a_lambda_needs_its_types_and_its_results_agree() {
    let run = with_collections("fn f() {\n let a = |x| x\n let b: (Int, Int) -> Int = |a| a\n let c = |n: Int| { if n > 0 { return 1 }\n return \"a\" }\n let d: (Int) -> Int = |n| { return \"a\" }\n let e: (Int) -> Int = |n| { }\n}");
    assert_eq!(
        run.errors,
        ["parameter `x` needs a type", "expected a function of 2 parameters, found 1", "parameter `a` needs a type", "expected Int, found String", "expected Int, found String", "missing return"]
    );
}

#[test]
fn a_lambda_captures_what_it_uses_from_outside() {
    let run = with_collections("fn f() {\n var n = 0\n let k = 5\n let inc = || { n += 1 }\n let get = || k + n\n let own = |m: Int| m + 1\n let outer = || { let inner = || n + k\n return inner }\n}");
    assert_eq!(run.errors, Vec::<String>::new());
    let dumped = dump(&run, "f");
    let lets: Vec<&str> = dumped.split("; ").collect();
    assert!(lets[2].contains("caps=[n]"), "{}", lets[2]);
    assert!(lets[3].contains("caps=[k,n]"), "{}", lets[3]);
    assert!(lets[4].contains("caps=[]"), "{}", lets[4]);
    assert!(lets[5].contains("caps=[n,k]"), "{}", lets[5]);
}

#[test]
fn a_lambda_writes_a_captured_var_but_not_a_captured_let() {
    let run = with_collections("fn f() {\n let m = 0\n var n = 0\n let a = || { m += 1 }\n let b = || { n += 1 }\n}");
    assert_eq!(run.errors, ["cannot assign: no `var` path to this place"]);
}

#[test]
fn a_send_lambda_captures_no_var() {
    let run = with_collections("fn f() {\n var n = 0\n let k = 1\n let a: Send () -> Int = || k\n let b: Send () -> Int = || n\n let c: () -> Int = || n\n}");
    assert_eq!(run.errors, ["a lambda that captures a `var` is not Sendable"]);
}

#[test]
fn an_enum_with_constants_casts_to_an_int_that_holds_them() {
    let run = main(
        "enum Level {\n Low = 1\n High = 10\n Neg = -5\n}\nenum Dup {\n A = 1\n B = 1\n}\nenum Mixed {\n A = 1\n B\n}\nenum Small {\n A = 300\n}\nfn f(l: Level, m: Mixed, s: Small) {\n let a = l as Int\n let b = l as Int8\n let c = l as UInt8\n let d = m as Int\n let e = s as UInt8\n let g = l as Float\n}",
    );
    assert_eq!(
        run.errors,
        [
            "constant 1 is used twice",
            "constant -5 does not fit UInt8",
            "cannot cast Mixed to Int: a variant has no constant",
            "constant 300 does not fit UInt8",
            "cannot cast Level to Float"
        ]
    );
    assert_eq!(let_type(&run, "f", 0), "Int");
}

#[test]
fn a_pub_declaration_names_only_pub_types() {
    let run = main(
        "struct Secret {}\npub struct Open {}\npub fn a(s: Secret) {}\npub fn b(o: Open) {}\nfn c(s: Secret) {}\npub struct Holder {\n pub s: Secret\n hidden: Secret\n pub fn get(self) -> Secret { return self.hidden }\n}\nstruct Inner {\n pub fn get(self) -> Secret { return Secret {} }\n}\npub enum E {\n A(Secret)\n B(Open)\n}\npub type Alias = Secret",
    );
    let message = "private type `Secret` in a public declaration";
    assert_eq!(run.errors, [message; 5]);
}

#[test]
fn assertions_are_calls_in_a_test_block() {
    let std = "pub fn assert(cond: Bool, msg: String) {}\npub fn assert_eq<T>(a: T, b: T) {}\npub fn fail(msg: String) {}";
    let run = run(&[("std.test", std), ("main", "import { assert, assert_eq, fail } from .std.test\ntest \"sums\" {\n assert_eq(1 + 1, 2)\n assert(true, \"yes\")\n assert_eq(1, \"one\")\n fail(\"no\")\n}\n@ignore\ntest \"slow\" {\n assert_eq(1, 1)\n}")]);
    assert_eq!(run.errors, ["expected Int, found String"]);
}

#[test]
fn an_attribute_the_compiler_does_not_define_is_unknown_on_an_item_and_on_a_variant() {
    let run = main("@made_up\npub fn m() {}\n@made_up(m)\ntrait T {}\n@made_up(f = x)\nstruct S { x: Int }\nenum E {\n @made_up(v = x)\n A\n @other\n B\n @stable\n C\n}");
    assert_eq!(run.errors, ["unknown attribute `@made_up`", "unknown attribute `@made_up`", "unknown attribute `@made_up`", "unknown attribute `@made_up`", "unknown attribute `@other`", "`@stable` does not go on a variant"]);
}

#[test]
fn the_attributes_the_compiler_defines_are_these_and_no_others() {
    assert_eq!(crate::resolve::KNOWN, ["blocking", "stable", "ignore"]);
}

/// Runs `text` as `main` with universal concurrency types available.
fn with_sync(text: &str) -> Run {
    main(text)
}

#[test]
fn spawn_makes_a_task_of_what_its_body_returns() {
    let run = with_sync("fn work(n: Int) -> Int { return n }\nfn f(n: Int) {\n let a = spawn { return n + 1 }\n let b = spawn work(n)\n let c = spawn { }\n spawn work(2)\n}");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(let_type(&run, "f", 0), "Task<Int>");
    assert_eq!(let_type(&run, "f", 1), "Task<Int>");
    assert_eq!(let_type(&run, "f", 2), "Task<Null>");
    assert_eq!(dump(&run, "f").split("; ").next(), Some("(let a (spawn caps=[n] {(return (add n 1:Int))}))"));
}

#[test]
fn a_spawn_in_a_scope_is_scoped_but_not_through_a_lambda() {
    let run = with_sync("fn f() {\n let a = spawn { return 1 }\n scope {\n let b = spawn { return 2 }\n let g = || spawn { return 3 }\n }\n}");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    let text = dump(&run, "f");
    assert!(text.starts_with("(let a (spawn caps=[] {(return 1:Int)})); (scope {(let b (spawn scoped caps=[] {(return 2:Int)})); (let g (closure [] caps=[] {(return (spawn caps=[] {(return 3:Int)}))}))})"), "{text}");
}

#[test]
fn a_spawn_block_uses_only_sendable_values() {
    let run = with_sync(
        "class Frozen { n: Int }\nclass Cell { var n: Int }\nstruct P { x: Int }\nenum E {\n A(Int)\n B(List<Int>)\n}\nfn f(a: Frozen, b: Cell, c: P, d: (Int, String), e: Int?, g: E, h: List<Int>, tx: Sender<Int>) {\n var n = 0\n spawn { let x = a }\n spawn { let x = b }\n spawn { let x = c }\n spawn { let x = d }\n spawn { let x = e }\n spawn { let x = g }\n spawn { let x = h }\n spawn { let x = tx }\n spawn { let x = n }\n}",
    );
    assert_eq!(
        run.errors,
        ["`b` is not Sendable: Cell", "`g` is not Sendable: E", "`h` is not Sendable: List<Int>", "a spawn block that captures a `var` is not Sendable"]
    );
}

#[test]
fn an_array_of_plain_elements_is_sendable_by_copy() {
    let run = with_sync(
        "class Frozen { n: Int }\nfn f(a: [UInt8], b: [[Int]], c: [String; 2], d: [Frozen], e: [List<Int>], tx: Sender<[UInt8]>) {\n spawn { let x = a }\n spawn { let x = b }\n spawn { let x = c }\n spawn { let x = d }\n spawn { let x = e }\n spawn { return a }\n}",
    );
    assert_eq!(run.errors, ["`d` is not Sendable: [Frozen]", "`e` is not Sendable: [List<Int>]"]);
}

#[test]
fn a_task_returns_only_a_sendable_value() {
    let run = with_sync("class Cell { var n: Int }\nfn make() -> Cell { return Cell { n: 1 } }\nfn f() {\n spawn { return 1 }\n spawn { return make() }\n spawn { return [1] }\n spawn { return (1, \"a\") }\n}");
    assert_eq!(run.errors, ["a task cannot return Cell", "a task cannot return List<Int>"]);
}

#[test]
fn a_send_function_returns_only_a_sendable_value() {
    let run = with_sync("class Cell { var n: Int }\nfn run(f: Send () -> Cell) {}\nfn ok(f: Send () -> Int) {}\nfn g() {\n ok(|| 1)\n run(|| Cell { n: 1 })\n}");
    assert_eq!(run.errors, ["a Send function cannot return Cell"]);
}

#[test]
fn a_send_function_takes_named_functions_and_lambdas_of_sendable_captures() {
    let run = with_sync(
        "fn run(f: Send (Int) -> Int) -> Int { return f(1) }\nfn add(n: Int) -> Int { return n + 1 }\nfn f() {\n var k = 0\n let xs = [1]\n let a = run(add)\n let b = run(|n| n + xs[0])\n let c = run(|n| n + k)\n let g = |n: Int| n\n let d = run(g)\n}",
    );
    assert_eq!(run.errors, ["`xs` is not Sendable: List<Int>", "a lambda that captures a `var` is not Sendable", "expected Send (Int) -> Int, found (Int) -> Int"]);
}

#[test]
fn with_binds_a_resource_that_has_close() {
    let run = with_sync(
        "class File { pub fn close(self) {} }\nclass Bad {}\nclass Mut { pub fn close(var self) {} }\nfn open() -> File { return File {} }\nfn bad() -> Bad { return Bad {} }\nfn mutable() -> Mut { return Mut {} }\nfn f() {\n with a = open() { }\n with b = bad() { }\n with c = mutable() { }\n with d = open() { d = open() }\n}",
    );
    assert_eq!(run.errors, ["Bad has no `close(self)` method", "Mut has no `close(self)` method", "cannot assign: no `var` path to this place"]);
    assert!(dump(&run, "f").starts_with("(with a (call open) {})"));
}

#[test]
fn a_channel_carries_only_what_may_enter_another_task() {
    let run = with_sync("fn a(tx: Sender<List<Int>>) {}\nfn d(tx: Sender<Int>, rx: Receiver<String>) {}\nfn e(tx: Sender<List<Int>>) {}");
    assert_eq!(run.errors, ["a channel cannot carry List<Int>"]);
}

#[test]
fn a_channel_of_optional_messages_is_refused() {
    let run = with_sync("fn a(tx: Sender<Int?>) {}\nfn b(rx: Receiver<String?>, tx: Sender<Int>) {}");
    assert_eq!(run.errors, ["a channel cannot carry optional messages"; 2]);
}

#[test]
fn a_union_member_is_not_an_optional() {
    let run = with_sync("fn a(x: Int | String?) {}\nfn b(x: (Int | String)?) {}");
    assert_eq!(run.errors, ["a union member is not an optional; write `(A | B)?`"]);
}

#[test]
fn a_once_holds_only_what_cannot_change_inside() {
    let run = with_sync("fn a(o: Once<List<Int>>) {}\nfn b(o: Once<Int>, p: Once<String>) {}");
    assert_eq!(run.errors, ["a `Once` cannot hold List<Int>"]);
}

#[test]
fn a_mutex_or_once_cannot_hold_a_function_or_a_stream() {
    let run = with_sync("fn a(m: Mutex<() -> Int>) {}\nfn b(o: Once<Stream<Int>>) {}\nfn c(m: Mutex<List<Int>>, n: Mutex<Int>) {}");
    assert_eq!(run.errors, ["a `Mutex` or `Once` cannot hold a function or a Stream"; 2]);
}

#[test]
fn a_mutex_and_a_once_are_sendable_handles() {
    let run = with_sync("struct Holder {\n m: Mutex<List<Int>>\n o: Once<Int>\n}\nfn f(m: Mutex<List<Int>>, o: Once<Int>) {\n spawn { m }\n spawn { o }\n}");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn a_port_end_is_a_sendable_handle_a_struct_may_hold_and_a_native_may_take() {
    let main_src = "extern \"pane\" {\n fn start(p: PortEnd)\n}\nstruct Holder {\n p: PortEnd\n}\nfn f(p: PortEnd, h: Holder) {\n spawn { start(p) }\n spawn { start(h.p) }\n}";
    let run = main(main_src);
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn a_channel_of_a_type_parameter_is_not_checked_until_it_is_written_out() {
    let run = with_sync("fn a<T>(tx: Sender<T>, rx: Receiver<List<T>>) {}");
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn a_function_fits_a_function_type_by_its_parameters_and_result() {
    let run = with_collections(
        "fn one(n: Int) -> Int { return n }\nfn bump(var n: Int) { n += 1 }\nfn f() {\n let a: (Int) -> Int? = one\n let b: (Int?) -> Int = one\n let c: (Int) -> Null = one\n let d: (var Int) -> Int = one\n let e: (Int) -> Int = one\n let g: (Int) -> Null = bump\n let h: (var Int) -> Null = bump\n let s: Stream<Int?> = count(1)\n let t: Stream<String> = count(1)\n}\nfn count(n: Int) -> Stream<Int> {\n yield n\n}",
    );
    assert_eq!(run.errors, ["expected (Int?) -> Int, found (Int) -> Int", "expected (Int) -> Null, found (var Int) -> Null", "expected Stream<String>, found Stream<Int>"]);
}

#[test]
fn a_nested_function_is_a_local_that_sees_what_came_before_it() {
    let run = main("fn outer() -> Int {\n let base = 10\n fn inner(n: Int) -> Int { return n + base }\n let x = inner(1)\n return x\n}\nfn g() { fn rec(n: Int) -> Int { return rec(n) } }\nfn h() { fn gen<T>(x: T) {} }\nfn k() { fn v(...xs: Int) {} }");
    assert_eq!(run.errors, ["a nested function takes no type parameters", "a nested function takes plain parameters"]);
    assert_eq!(let_type(&run, "outer", 2), "Int");
    assert!(dump(&run, "outer").contains("caps=[base]"));
    assert!(dump(&run, "g").contains("caps=[rec]"), "{}", dump(&run, "g"));
}

#[test]
fn a_function_that_yields_is_a_generator() {
    let run = with_collections(
        "fn count(n: Int) -> Stream<Int> {\n for i in 0..n { yield i }\n}\nfn bad() -> Int { yield 1\nreturn 1 }\nfn text() -> Stream<Int> { yield \"a\" }\nfn early() -> Stream<Int> {\n yield 1\n return 2\n}\nfn plain() -> Stream<Int> { return count(1) }\nfn lam() { let l = || { yield 1 } }\nfn use() {\n for v in count(3) { let w = v + 1 }\n}",
    );
    assert_eq!(run.errors, ["`yield` is only in a function that returns a Stream", "expected Int, found String", "a generator returns no value", "`yield` is only in a function that returns a Stream"]);
    assert!(run.program.checked[&run.def("count")].is_generator);
    assert!(!run.program.checked[&run.def("plain")].is_generator);
}

#[test]
fn a_generator_cannot_yield_inside_with_or_scope() {
    let run = with_collections(
        "struct Res { var id: Int\n fn close(self) {} }\nfn a() -> Stream<Int> {\n with r = Res { id: 1 } {\n yield 1\n }\n}\nfn b() -> Stream<Int> {\n scope {\n for i in 0..2 { yield i }\n }\n}\nfn c() -> Stream<Int> {\n with r = Res { id: 1 } {\n let n = 1\n }\n yield 2\n}",
    );
    assert_eq!(run.errors, ["a generator cannot yield inside `with` or `scope`", "a generator cannot yield inside `with` or `scope`"]);
}

#[test]
fn top_level_code_assigns_its_own_globals_and_destructures() {
    let run = with_collections(
        "var count = 0\nlet K = 1\nlet (a, b) = (1, \"x\")\ncount += 1\nfor i in 0..3 { count += i }\nK = 2\nreturn\nfn f() { count += 1 }\nfn g() -> String { return b }",
    );
    assert_eq!(run.errors, ["cannot assign: no `var` path to this place", "only the module's own top-level code assigns a global", "cannot assign: no `var` path to this place", "`return` is only in a function"]);
    assert_eq!(run.program.top_level.len(), 1);
    assert_eq!(run.program.global_inits.len(), 3);
}

#[test]
fn a_top_level_let_binding_no_name_is_top_level_code() {
    let run = with_collections("fn f() -> Int { return 1 }\nlet _ = f()\nlet 1 = f()");
    assert_eq!(run.errors, ["this pattern can fail; use `if let` or `match`"]);
    assert_eq!(run.program.top_level.len(), 1);
    assert_eq!(run.program.global_inits.len(), 0);
}

#[test]
fn a_listed_trait_is_checked_and_gives_its_defaults() {
    let run = main(
        "trait Shape {\n fn area(self) -> Int\n fn describe(self) -> String { return \"shape\" }\n}\nstruct Square: (Shape) {\n side: Int\n fn area(self) -> Int { return self.side }\n}\nstruct Bad: (Shape) {\n fn area(self) -> String { return \"\" }\n}\nstruct Missing: (Shape) {}\nfn f(s: Square) -> String { return s.describe() }\nfn g<T: Shape>(x: T) -> Int { return x.area() }\nfn h(s: Square) -> Int { return g(s) }",
    );
    assert_eq!(run.errors, ["`Bad.area` does not match `Shape.area`", "`Missing` has no `area`, which `Shape` requires"]);
    assert_eq!(dump(&run, "f"), "(return (call describe s))");
    assert_eq!(dump_method(&run, "Square", "describe"), "(return \"shape\")");
}

#[test]
fn unary_and_binary_dunder_operators() {
    let run = main(
        "struct Vec2 {\n x: Int\n y: Int\n pub fn __neg__(self) -> Vec2 { return Vec2 { x: -self.x, y: -self.y } }\n pub fn __add__(self, other: Vec2) -> Vec2 { return Vec2 { x: self.x + other.x, y: self.y + other.y } }\n pub fn __mul__(self, s: Int) -> Vec2 { return Vec2 { x: self.x * s, y: self.y * s } }\n pub fn __rmul__(self, s: Int) -> Vec2 { return Vec2 { x: self.x * s, y: self.y * s } }\n pub fn __eq__(self, other: Vec2) -> Bool { return self.x == other.x && self.y == other.y }\n pub fn __lt__(self, other: Vec2) -> Bool { return self.x < other.x }\n pub fn __gt__(self, other: Int) -> Bool { return self.x > other }\n}\nfn f(a: Vec2, b: Vec2, opt: Vec2?) {\n let neg = -a\n let sum = a + b\n let fwd = a * 2\n let rev = 2 * a\n let eq = a == b\n let neq = a != b\n let cmp = a < b\n let rcmp = 5 < a\n let opt_cmp = a == opt\n let null_cmp = opt == null\n}\nfn bad(a: Vec2, b: Vec2) {\n let x = a - b\n let y = !a\n}",
    );
    assert_eq!(run.errors, ["`-` does not work on Vec2", "`!` does not work on Vec2"]);
    assert_eq!(
        dump(&run, "f"),
        "(let neg (call __neg__ a)); (let sum (call __add__ a b)); (let fwd (call __mul__ a 2:Int)); (let rev (call __rmul__ a 2:Int)); (let eq (call __eq__ a b)); (let neq (not (call __eq__ a b))); (let cmp (call __lt__ a b)); (let rcmp (call __gt__ a 5:Int)); (let opt_cmp (eq (to-optional a) opt)); (let null_cmp (eq opt null))",
    );
    let orders: Vec<&Vec<u32>> = run.program.checked[&run.def("f")].arg_order.values().collect();
    assert_eq!(orders, [&vec![1, 0], &vec![1, 0]]);
}

#[test]
fn compound_assignment_ladder() {
    let run = main(
        "struct Vec2 {\n var x: Int\n var y: Int\n pub fn __iadd__(var self, other: Vec2) {\n  self.x += other.x\n  self.y += other.y\n }\n pub fn __isub__(self, other: Vec2) {}\n pub fn __mul__(self, s: Int) -> Vec2 {\n  return Vec2 { x: self.x * s, y: self.y * s }\n }\n}\nfn ok(var a: Vec2, b: Vec2) {\n a += b\n a *= 2\n}\nfn bad(var a: Vec2, let_v: Vec2, b: Vec2) {\n a -= b\n a /= 2\n let_v += b\n}",
    );
    assert_eq!(
        run.errors,
        ["`__isub__` must take `var self`", "`/` needs one type on both sides, found Vec2 and Int", "cannot assign: no `var` path to this place"],
    );
    assert_eq!(dump(&run, "ok"), "(call __iadd__ a b); (= a (call __mul__ a 2:Int))");
}

#[test]
fn index_dunder_operators() {
    let run = main(
        "struct Grid {\n var data: Int\n pub fn __getitem__(self, idx: Int) -> Int { return self.data + idx }\n pub fn __setitem__(var self, idx: Int, val: Int) { self.data = idx + val }\n}\nstruct ReadOnly {\n pub fn __setitem__(self, idx: Int, val: Int) {}\n}\nstruct Plain {\n x: Int\n}\nfn ok(var g: Grid) -> Int {\n g[1] = 10\n return g[2]\n}\nfn bad(var g: Grid, let_g: Grid, var ro: ReadOnly, p: Plain) {\n g[0] += 1\n let_g[0] = 1\n ro[0] = 1\n p[0] = 1\n}",
    );
    assert_eq!(
        run.errors,
        [
            "custom index assignment only supports `=`",
            "this place has no `var` path to write through",
            "`__setitem__` must take `var self`",
            "Plain cannot be indexed",
        ],
    );
    assert_eq!(dump(&run, "ok"), "(call __setitem__ g 1:Int 10:Int); (return (call __getitem__ g 2:Int))");
}

#[test]
fn sized_array_elementwise_broadcast_and_mask_ops() {
    let run = main(
        "fn ok(var a: [Int; 3], b: [Int; 3], c: [Float; 2], u: [Int8; 2], m1: [Bool; 3], m2: [Bool; 3]) {\n let neg = -a\n let inv = ~a\n let not_m = !m1\n let add = a + b\n let bc_r = c + 2.0\n let bc_l = 3.0 * c\n let bc_i8 = u + 2\n let lt = a < b\n let eq = a == b\n let bc_cmp = 2 <= a\n let mask = (a > 0) & m2\n a += b\n a *= 2\n}\nfn bad(a: [Int; 2], b: [Int; 3], f: [Float; 2]) {\n let x = a + b\n let y = a + true\n let z = f << 1\n}",
    );
    assert_eq!(
        run.errors,
        [
            "`+` needs one type on both sides, found [Int; 2] and [Int; 3]",
            "`+` needs one type on both sides, found [Int; 2] and Bool",
            "`<<` needs one type on both sides, found [Float; 2] and Int",
        ],
    );
    let types: Vec<String> = (0..11).map(|n| let_type(&run, "ok", n)).collect();
    assert_eq!(
        types,
        ["[Int; 3]", "[Int; 3]", "[Bool; 3]", "[Int; 3]", "[Float; 2]", "[Float; 2]", "[Int8; 2]", "[Bool; 3]", "[Bool; 3]", "[Bool; 3]", "[Bool; 3]"],
    );
}

#[test]
fn const_generics_and_const_expressions_evaluate_and_monomorphize() {
    let run = main(
        "const WIDTH = 3\nstruct Vec<T, const N: Int> {\n data: [T; N]\n}\nfn pad<T, const N: Int>(xs: [T; N]) -> [T; N + 1] {\n return xs\n}\nfn f(a: Vec<Int, WIDTH>, b: [Int; 4]) -> [Int; 3] {\n return a.data\n}\nfn bad_mismatch(a: Vec<Int, 3>, b: Vec<Int, 4>) {\n let x: Vec<Int, 4> = a\n}",
    );
    assert_eq!(run.errors, ["expected [T; <const>], found [T; N]", "expected Vec<Int, 4>, found Vec<Int, 3>"]);
}

#[test]
fn comptime_if_for_and_builtins_lower_at_monomorphization() {
    let reflect = "pub enum Kind { Struct\n Class\n Enum\n Primitive\n List\n Array\n FixedArray\n Option\n Tuple\n Union\n Function\n Pointer\n Other }\npub struct FieldInfo {\n pub name: String\n pub index: Int\n pub is_var: Bool\n}\npub struct VariantInfo {\n pub name: String\n pub index: Int\n pub arity: Int\n}\npub enum Shape {\n Fields([FieldInfo])\n Variants([VariantInfo])\n None\n}\npub struct TypeInfo {\n pub name: String\n pub kind: Kind\n pub size: Int\n pub align: Int\n pub shape: Shape\n}";
    let main_src = "import TypeInfo from .std.reflect\nstruct Pair {\n a: Int\n var b: Int\n}\nfn inspect<T>(var p: T) -> Int {\n comptime if @has_field(T, \"b\") {\n  @set_field(p, \"b\", 42)\n  return @field(p, \"b\")\n } else {\n  @compile_error(\"missing field b\")\n }\n}\nfn sum_fields(p: Pair) -> Int {\n var total = 0\n comptime for f in @fields(Pair) {\n  total += @field(p, f.name)\n }\n return total\n}\nfn sizes() -> Int {\n let s = @size_of(Int)\n let a = @align_of(Int)\n let n = @type_name(Pair)\n let info: TypeInfo = @type_info(Pair)\n let x: @typeof(s) = 10\n @compile_assert(s == 8, \"64-bit Int\")\n return s + a + x\n}\nfn main_ok(var p: Pair) -> Int {\n return inspect(p) + sum_fields(p) + sizes()\n}";
    let run = run(&[("std.reflect", reflect), ("main", main_src)]);
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn extend_blocks_work_on_user_and_universal_types_with_traits() {
    let run = main(
        "trait Show {\n fn show(self) -> String\n}\nextend Int: (Show) {\n pub fn show(self) -> String { return \"int\" }\n pub fn twice(self) -> Int { return self * 2 }\n}\nextend <const N: Int, T> [T; N] {\n pub fn first_elem(self) -> T { return self[0] }\n}\nfn print_any<T: Show>(x: T) -> String { return x.show() }\nfn f(arr: [Int; 3]) -> String {\n let n = 5.twice()\n let first = arr.first_elem()\n return print_any(n) + print_any(first)\n}",
    );
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn bound_static_call_works_with_comptime_monomorphization() {
    let run = main(
        "trait Make {\n fn make(x: Int) -> Self\n}\nstruct Item: (Make) {\n value: Int\n pub fn make(x: Int) -> Item { return Item { value: x } }\n}\nfn build<T: Make>(x: Int) -> (String, T) {\n let name = @type_name(T)\n return (name, T.make(x))\n}\nfn entry() -> (String, Item) {\n return build<Item>(7)\n}",
    );
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn any_type_boxing_narrowing_match_and_constraints() {
    let run = main(
        "struct Pair {\n x: Int\n}\nfn accept(v: Any) -> String {\n let n: Int? = v.to<Int>()\n if v is Int {\n  return \"${v + 1}\"\n }\n return match v {\n  s: String => s\n  p: Pair => \"${p.x}\"\n  _ => \"other:${v}\"\n }\n}\nfn callers(u: Int | String) {\n let list: List<Any> = [1, \"hi\", Pair { x: 2 }, u, null]\n let _ = accept(list[0])\n}\nfn bad(a: Any, b: Any) -> Bool {\n let missing = match a {\n  n: Int => n\n }\n return a == b\n}",
    );
    assert_eq!(
        run.errors,
        [
            "match does not cover `_`",
            "`==` does not work on Any",
        ],
    );
}

#[test]
fn any_is_not_sendable() {
    let run = main(
        "fn send_it<T: Send>(x: T) {}\nfn bad(a: Any) {\n send_it(a)\n}",
    );
    assert!(
        run.errors.iter().any(|e| e.contains("Send")),
        "expected Send bound error, got {:?}",
        run.errors
    );
}

#[test]
fn typeof_works_in_type_positions_and_rejects_value_use() {
    let ok_run = main(
        "fn ok(a: Int) -> String {\n let x: @typeof(a) = a\n const t = @typeof(a)\n return @type_name(@typeof(a)) + @type_name(t)\n}",
    );
    assert!(ok_run.errors.is_empty(), "{:?}", ok_run.errors);

    let bad_run = main(
        "fn bad(a: Int) {\n let v = @typeof(a)\n}",
    );
    assert!(
        bad_run.errors.iter().any(|e| e.contains("is a type, not a value")),
        "expected type-not-value error, got {:?}",
        bad_run.errors
    );
}

#[test]
fn array_repeat_assigns_to_typeof_and_const_type_aliases() {
    let ok = with_collections(
        "fn fill_val<T>(x: T, n: Int) -> [T] {\n let res: [@typeof(x)] = [x; n]\n return res\n}\nfn ok(n: Int) {\n let i: Int32 = 5\n let a: [@typeof(i)] = [i; n]\n let b: [@typeof(i); 4] = [i; 4]\n const Elem = @typeof(i)\n let c: [Elem] = [0; n]\n let d: [@typeof(i)] = [0; n]\n let e = fill_val(i, 3)\n}",
    );
    assert!(ok.errors.is_empty(), "{:?}", ok.errors);
    assert_eq!(let_type(&ok, "ok", 1), "[Int32]");
    assert_eq!(let_type(&ok, "ok", 2), "[Int32; 4]");
    assert_eq!(let_type(&ok, "ok", 3), "[Int32]");
    assert_eq!(let_type(&ok, "ok", 4), "[Int32]");
    assert_eq!(let_type(&ok, "ok", 5), "[Int32]");
}

#[test]
fn custom_slice_and_stream_for_loop_dispatch() {
    let ok = with_collections(
        "class BoxView {\n pub fn __slice__(self, start: Int?, end: Int?, inclusive: Bool, step: Int?) -> BoxView {\n  return self\n }\n pub fn stream(self) -> Stream<Int> {\n  yield 1\n }\n}\nfn f(v: BoxView) -> Int {\n let a = v[1..3]\n let b = a[..=2]\n let c = b[0..4..2]\n var total = 0\n for x in c {\n  total += x\n }\n return total\n}",
    );
    assert!(ok.errors.is_empty(), "{:?}", ok.errors);
    assert_eq!(let_type(&ok, "f", 0), "BoxView");
    assert_eq!(let_type(&ok, "f", 1), "BoxView");
    assert_eq!(let_type(&ok, "f", 2), "BoxView");
}




