use isa::type_term::TypeTerm;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// A checked type.
pub enum Type {
    Int,
    Float,
    Bool,
    Char,
    String,
    Null,
    Any,
    /// A type argument inference has not fixed yet (`Map()`, `Ok(1)`'s error type); context fills it, a binding may not keep it.
    Hole,
    Struct {
        name: String,
        /// The type arguments of a generic type, in declaration order; empty otherwise.
        args: Vec<Type>,
        fields: Vec<(String, Type)>,
    },
    Class {
        name: String,
        /// The type arguments of a generic type, in declaration order; empty otherwise.
        args: Vec<Type>,
        fields: Vec<(String, Type)>,
    },
    Nullable(Box<Type>),
    /// An anonymous structural tuple: equal by arity and element types, never by declaration. `()` is `Tuple(vec![])`.
    Tuple(Vec<Type>),
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Set(Box<Type>),
    Bytes,
    /// The receiving end of a channel; the channel object itself at run time.
    Receiver(Box<Type>),
    /// A sending end of a channel; a handle counted as one sender.
    Sender(Box<Type>),
    /// A lazy, single-pass, task-local sequence: a generator's result. Never `Send`.
    Stream(Box<Type>),
    /// A `spawn { }` handle; `T` is the body's result, `.join()` returns `Result<T, String>` and `.cancel()` requests cancellation.
    Task(Box<Type>),
    /// A read-only, Sendable view of a sealed graph; the runtime value is the sealed object itself.
    Shared(Box<Type>),
    /// A type parameter inside its declaration: opaque, assignable only to itself and `Any`.
    Param(String),
    Result(Box<Type>, Box<Type>),
    /// A function value. `sendable`: every capture is Sendable and none is a local `var`. `modes[i]`: parameter `i` is `var`; empty when none is.
    Function { params: Vec<Type>, ret: Box<Type>, sendable: bool, modes: Vec<bool> },
    /// A user or `std` `enum`; `variants` maps each variant name to its payload types (empty for a unit variant).
    Enum {
        name: String,
        /// The type arguments of a generic enum, in declaration order; empty otherwise.
        args: Vec<Type>,
        variants: Vec<(String, Vec<Type>)>,
    },
    /// `A | B`: a value is one member's value, with its own tag or descriptor.
    Union(Members),
}

/// A union's members, flat and distinct, in the order written; equal as sets.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Members(pub Vec<Type>);

impl PartialEq for Members {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().all(|t| other.0.contains(t))
    }
}

impl Eq for Members {}

impl Type {
    /// The union of `members`, flattened and deduplicated; one member is itself. A `Null` or optional member makes the union optional.
    pub fn union(members: Vec<Type>) -> Type {
        let mut flat: Vec<Type> = Vec::new();
        let mut optional = false;
        let add = |t: Type, flat: &mut Vec<Type>| {
            if !flat.iter().any(|m| m.same_as(&t) && !t.has_hole()) {
                flat.push(t);
            }
        };
        for m in members {
            match m {
                Type::Union(Members(ms)) => ms.into_iter().for_each(|t| add(t, &mut flat)),
                Type::Null => optional = true,
                Type::Nullable(inner) if !matches!(*inner, Type::Nullable(_)) => {
                    optional = true;
                    match *inner {
                        Type::Union(Members(ms)) => ms.into_iter().for_each(|t| add(t, &mut flat)),
                        t => add(t, &mut flat),
                    }
                }
                t => add(t, &mut flat),
            }
        }
        if flat.contains(&Type::Any) {
            flat = vec![Type::Any];
        }
        let base = match flat.len() {
            0 => Type::Null,
            1 => flat.pop().unwrap(),
            _ => Type::Union(Members(flat)),
        };
        if optional && base != Type::Null { Type::Nullable(Box::new(base)) } else { base }
    }

    /// A union's members, or this type alone.
    pub fn members(&self) -> Vec<Type> {
        match self {
            Type::Union(Members(ms)) => ms.clone(),
            t => vec![t.clone()],
        }
    }

    /// This type as the runtime records it; `None` when it names a type parameter. A hole is recorded as `_`.
    pub(crate) fn runtime_term(&self) -> Option<TypeTerm> {
        self.term_with(&|_| None)
    }

    /// This type as a runtime term, each type parameter replaced by `param`'s answer; `None` if one has none. A hole is `_`.
    pub(crate) fn term_with(&self, param: &dyn Fn(&str) -> Option<TypeTerm>) -> Option<TypeTerm> {
        let terms = |tys: &[Type]| tys.iter().map(|t| t.term_with(param)).collect::<Option<Vec<_>>>();
        let named = |name: &str, args: &[Type]| Some(TypeTerm::Named(name.to_string(), terms(args)?));
        match self {
            Type::Param(name) => param(name),
            Type::Hole => Some(TypeTerm::wildcard()),
            Type::Struct { name, args, .. } | Type::Class { name, args, .. } | Type::Enum { name, args, .. } => named(name, args),
            Type::Nullable(inner) => Some(TypeTerm::Nullable(Box::new(inner.term_with(param)?))),
            Type::Tuple(elems) => Some(TypeTerm::Tuple(terms(elems)?)),
            Type::List(t) => named("List", std::slice::from_ref(t)),
            Type::Set(t) => named("Set", std::slice::from_ref(t)),
            Type::Receiver(t) => named("Receiver", std::slice::from_ref(t)),
            Type::Sender(t) => named("Sender", std::slice::from_ref(t)),
            Type::Stream(t) => named("Stream", std::slice::from_ref(t)),
            Type::Task(t) => named("Task", std::slice::from_ref(t)),
            Type::Shared(t) => named("Shared", std::slice::from_ref(t)),
            Type::Map(k, v) => named("Map", &[(**k).clone(), (**v).clone()]),
            Type::Result(t, e) => named("Result", &[(**t).clone(), (**e).clone()]),
            Type::Function { params, ret, sendable, .. } => {
                Some(TypeTerm::Function { sendable: *sendable, params: terms(params)?, ret: Box::new(ret.term_with(param)?) })
            }
            Type::Union(Members(ms)) => {
                let mut members = terms(ms)?;
                members.sort_by_cached_key(TypeTerm::to_string);
                Some(TypeTerm::Union(members))
            }
            _ => Some(TypeTerm::named(&format!("{self:?}"))),
        }
    }

    /// A function type's `var` parameter positions; empty for any other type.
    pub(crate) fn var_modes(&self) -> Vec<bool> {
        match self {
            Type::Function { modes, .. } => modes.clone(),
            _ => Vec::new(),
        }
    }

    /// The name of every struct, class and enum this type mentions, type arguments included.
    pub(crate) fn named_types(&self, out: &mut Vec<String>) {
        match self {
            Type::Struct { name, args, .. } | Type::Class { name, args, .. } | Type::Enum { name, args, .. } => {
                out.push(name.clone());
                args.iter().for_each(|a| a.named_types(out));
            }
            Type::Nullable(t) | Type::List(t) | Type::Set(t) | Type::Receiver(t) | Type::Sender(t) | Type::Stream(t) | Type::Task(t) | Type::Shared(t) => t.named_types(out),
            Type::Map(a, b) | Type::Result(a, b) => {
                a.named_types(out);
                b.named_types(out);
            }
            Type::Tuple(ts) | Type::Union(Members(ts)) => ts.iter().for_each(|t| t.named_types(out)),
            Type::Function { params, ret, .. } => {
                params.iter().for_each(|t| t.named_types(out));
                ret.named_types(out);
            }
            _ => {}
        }
    }

    /// Whether a value of this type is, or holds, a `Sender` or `Receiver` (which `spawn` moves).
    pub(crate) fn holds_end(&self) -> bool {
        self.holds_end_in(&mut Vec::new())
    }

    fn holds_end_in(&self, seen: &mut Vec<String>) -> bool {
        match self {
            Type::Sender(_) | Type::Receiver(_) => true,
            Type::Nullable(t) | Type::List(t) | Type::Set(t) | Type::Stream(t) | Type::Task(t) | Type::Shared(t) => t.holds_end_in(seen),
            Type::Map(a, b) | Type::Result(a, b) => a.holds_end_in(seen) || b.holds_end_in(seen),
            Type::Tuple(ts) | Type::Union(Members(ts)) => ts.iter().any(|t| t.holds_end_in(seen)),
            Type::Struct { name, fields, .. } | Type::Class { name, fields, .. } => {
                if seen.contains(name) {
                    return false;
                }
                seen.push(name.clone());
                fields.iter().any(|(_, t)| t.holds_end_in(seen))
            }
            _ => false,
        }
    }

    /// The full type a value of this generic type records on its descriptor; `None` for a non-generic type or an unknown argument.
    pub(crate) fn instance_term(&self) -> Option<TypeTerm> {
        if self.records_instance() { self.runtime_term() } else { None }
    }

    /// Whether a value of this type records its full type: a user type, a generic container, tuple, function or stream.
    pub(crate) fn records_instance(&self) -> bool {
        match self {
            Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Receiver(_) | Type::Sender(_) | Type::Task(_) | Type::Stream(_) => true,
            Type::Result(..) | Type::Function { .. } => true,
            Type::Tuple(elems) => !elems.is_empty(),
            Type::Struct { .. } | Type::Class { .. } | Type::Enum { .. } => true,
            _ => false,
        }
    }

    /// Whether this type is exactly the type of any value it describes (no covariant view), so a value may be recorded from it.
    pub(crate) fn is_exact(&self) -> bool {
        matches!(
            self,
            Type::List(_)
                | Type::Map(..)
                | Type::Set(_)
                | Type::Receiver(_)
                | Type::Sender(_)
                | Type::Result(..)
                | Type::Struct { .. }
                | Type::Class { .. }
                | Type::Enum { .. }
        )
    }

    /// The reserved id of a generic intrinsic (`List`, `Map`, `Set`, `Receiver`, `Sender`, `Task`).
    pub fn intrinsic_id(&self) -> Option<u64> {
        use isa::value::{CHANNEL_TYPE_ID, LIST_TYPE_ID, MAP_TYPE_ID, SENDER_TYPE_ID, SET_TYPE_ID, TASK_TYPE_ID};
        Some(match self {
            Type::List(_) => LIST_TYPE_ID,
            Type::Map(..) => MAP_TYPE_ID,
            Type::Set(_) => SET_TYPE_ID,
            Type::Receiver(_) => CHANNEL_TYPE_ID,
            Type::Sender(_) => SENDER_TYPE_ID,
            Type::Task(_) => TASK_TYPE_ID,
            _ => return None,
        })
    }

    /// The `List`, `Map` or `Set` that makes this unfit as a `Map`/`Set` key (it could change after insert).
    pub(crate) fn mutable_key_part(&self) -> Option<&Type> {
        match self {
            Type::List(_) | Type::Map(..) | Type::Set(_) => Some(self),
            Type::Tuple(elems) | Type::Union(Members(elems)) => elems.iter().find_map(Type::mutable_key_part),
            Type::Nullable(inner) => inner.mutable_key_part(),
            _ => None,
        }
    }

    /// Whether a value of this type fits `target`; an `Any` value fits anything and is checked at run time.
    pub(crate) fn is_assignable_to(&self, target: &Type) -> bool {
        matches!(self, Type::Any) || self.fits_within(target)
    }

    fn fits_within(&self, target: &Type) -> bool {
        if self == target || matches!(target, Type::Any | Type::Hole) || matches!(self, Type::Hole) {
            return true;
        }
        if let Type::Union(Members(ms)) = self {
            return ms.iter().all(|m| m.fits_within(target));
        }
        if let Type::Union(Members(ms)) = target {
            return ms.iter().any(|m| self.fits_within(m));
        }
        if let Type::Nullable(inner) = target
            && (self == &Type::Null || self.fits_within(inner)) {
                return true;
            }
        match (self, target) {
            (Type::Nullable(a), Type::Nullable(b)) => a.fits_within(b),
            (Type::List(a), Type::List(b))
            | (Type::Set(a), Type::Set(b))
            | (Type::Receiver(a), Type::Receiver(b))
            | (Type::Sender(a), Type::Sender(b))
            | (Type::Shared(a), Type::Shared(b)) => a.same_as(b),
            (Type::Map(ak, av), Type::Map(bk, bv)) => ak.same_as(bk) && av.same_as(bv),
            (Type::Stream(a), Type::Stream(b)) | (Type::Task(a), Type::Task(b)) => a.fits_within(b),
            (Type::Class { name: a, args: aa, .. }, Type::Class { name: b, args: ba, .. })
            | (Type::Struct { name: a, args: aa, .. }, Type::Struct { name: b, args: ba, .. })
            | (Type::Enum { name: a, args: aa, .. }, Type::Enum { name: b, args: ba, .. }) => {
                a == b && aa.len() == ba.len() && aa.iter().zip(ba).all(|(x, y)| x.same_as(y))
            }
            (Type::Tuple(a), Type::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.fits_within(y))
            }
            (
                Type::Function { params: pa, ret: ra, sendable: sa, modes: ma },
                Type::Function { params: pb, ret: rb, sendable: sb, modes: mb },
            ) => {
                (*sa || !*sb)
                    && (0..pa.len()).all(|i| !ma.get(i).copied().unwrap_or(false) || mb.get(i).copied().unwrap_or(false))
                    && pa.len() == pb.len()
                    && pb.iter().zip(pa).all(|(want, have)| want.fits_within(have))
                    && (**rb == Type::Null || ra.fits_within(rb))
            }
            _ => false,
        }
    }

    /// Whether inference left part of this type unfixed.
    pub(crate) fn has_hole(&self) -> bool {
        match self {
            Type::Hole => true,
            Type::Struct { args, .. } | Type::Class { args, .. } | Type::Enum { args, .. } | Type::Tuple(args) | Type::Union(Members(args)) => {
                args.iter().any(Type::has_hole)
            }
            Type::Nullable(a)
            | Type::List(a)
            | Type::Set(a)
            | Type::Receiver(a)
            | Type::Sender(a)
            | Type::Stream(a)
            | Type::Task(a)
            | Type::Shared(a) => a.has_hole(),
            Type::Map(a, b) | Type::Result(a, b) => a.has_hole() || b.has_hole(),
            Type::Function { params, ret, .. } => params.iter().any(Type::has_hole) || ret.has_hole(),
            _ => false,
        }
    }

    /// The same type, as a type argument must be: `Any` matches only `Any`, a hole matches anything.
    pub fn same_as(&self, other: &Type) -> bool {
        let all = |a: &[Type], b: &[Type]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_as(y));
        match (self, other) {
            (Type::Hole, _) | (_, Type::Hole) => true,
            (Type::Struct { name: a, args: aa, .. }, Type::Struct { name: b, args: ba, .. })
            | (Type::Class { name: a, args: aa, .. }, Type::Class { name: b, args: ba, .. })
            | (Type::Enum { name: a, args: aa, .. }, Type::Enum { name: b, args: ba, .. }) => a == b && all(aa, ba),
            (Type::Nullable(a), Type::Nullable(b))
            | (Type::List(a), Type::List(b))
            | (Type::Set(a), Type::Set(b))
            | (Type::Receiver(a), Type::Receiver(b))
            | (Type::Sender(a), Type::Sender(b))
            | (Type::Stream(a), Type::Stream(b))
            | (Type::Task(a), Type::Task(b))
            | (Type::Shared(a), Type::Shared(b)) => a.same_as(b),
            (Type::Map(ak, av), Type::Map(bk, bv)) | (Type::Result(ak, av), Type::Result(bk, bv)) => ak.same_as(bk) && av.same_as(bv),
            (Type::Tuple(a), Type::Tuple(b)) => all(a, b),
            (Type::Function { params: pa, ret: ra, sendable: sa, modes: ma }, Type::Function { params: pb, ret: rb, sendable: sb, modes: mb }) => {
                sa == sb && ma == mb && all(pa, pb) && ra.same_as(rb)
            }
            (Type::Union(Members(a)), Type::Union(Members(b))) => {
                a.len() == b.len() && a.iter().all(|x| b.iter().any(|y| x.same_as(y)))
            }
            _ => self == other,
        }
    }
}

/// Whether `name` is a type the language provides without a declaration.
pub(crate) fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "Int" | "Float" | "Bool" | "Char" | "String" | "Null" | "__Any" | "Bytes"
            | "List" | "Map" | "Set" | "Sender" | "Receiver" | "Stream" | "Task" | "Shared" | "Result"
            | "Option" | "Frozen"
    )
}

impl std::fmt::Debug for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn list(tys: &[Type]) -> String {
            tys.iter().map(|t| format!("{t:?}")).collect::<Vec<_>>().join(", ")
        }
        let generic = |f: &mut std::fmt::Formatter<'_>, name: &str, args: &[Type]| {
            if args.is_empty() { write!(f, "{name}") } else { write!(f, "{name}<{}>", list(args)) }
        };
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Bool => write!(f, "Bool"),
            Type::Char => write!(f, "Char"),
            Type::String => write!(f, "String"),
            Type::Null => write!(f, "Null"),
            Type::Any => write!(f, "Any"),
            Type::Hole => write!(f, "_"),
            Type::Bytes => write!(f, "Bytes"),
            Type::Struct { name, args, .. } | Type::Class { name, args, .. } | Type::Enum { name, args, .. } => generic(f, name, args),
            Type::Nullable(t) if matches!(**t, Type::Union(_)) => write!(f, "({t:?})?"),
            Type::Nullable(t) => write!(f, "{t:?}?"),
            Type::Union(Members(ms)) => {
                let shown: Vec<String> =
                    ms.iter().map(|m| if matches!(m, Type::Function { .. }) { format!("({m:?})") } else { format!("{m:?}") }).collect();
                write!(f, "{}", shown.join(" | "))
            }
            Type::Tuple(ts) => write!(f, "({})", list(ts)),
            Type::List(t) => generic(f, "List", std::slice::from_ref(t)),
            Type::Set(t) => generic(f, "Set", std::slice::from_ref(t)),
            Type::Receiver(t) => generic(f, "Receiver", std::slice::from_ref(t)),
            Type::Sender(t) => generic(f, "Sender", std::slice::from_ref(t)),
            Type::Stream(t) => generic(f, "Stream", std::slice::from_ref(t)),
            Type::Task(t) => generic(f, "Task", std::slice::from_ref(t)),
            Type::Shared(t) => generic(f, "Shared", std::slice::from_ref(t)),
            Type::Map(k, v) => write!(f, "Map<{k:?}, {v:?}>"),
            Type::Result(t, e) => write!(f, "Result<{t:?}, {e:?}>"),
            Type::Param(n) => write!(f, "{n}"),
            Type::Function { params, ret, sendable, modes } => {
                let shown: Vec<String> =
                    params.iter().enumerate().map(|(i, p)| format!("{}{p:?}", if modes.get(i) == Some(&true) { "var " } else { "" })).collect();
                write!(f, "{}({}) -> {ret:?}", if *sendable { "Send " } else { "" }, shown.join(", "))
            }
        }
    }
}
