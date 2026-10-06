//! The full type a generic value's descriptor records.

use std::fmt;

/// A type as the runtime records it: a name with arguments, or a structural form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeTerm {
    /// A primitive, a std type or a user type, with its type arguments (`Int`, `List<Int>`, `Box<String>`).
    Named(String, Vec<TypeTerm>),
    Nullable(Box<TypeTerm>),
    Tuple(Vec<TypeTerm>),
    Function { sendable: bool, params: Vec<TypeTerm>, ret: Box<TypeTerm> },
    /// A union's members, sorted by their printed form.
    Union(Vec<TypeTerm>),
    /// In a pattern only: the type whose interned id register `r` holds (a type argument).
    Reg(u8),
    /// In a pattern only: argument `i` of the type recorded on the object in register `r` (`self`'s `T`).
    ArgOf(u8, u8),
}

impl TypeTerm {
    /// A type with no arguments.
    pub fn named(name: &str) -> Self {
        TypeTerm::Named(name.to_string(), Vec::new())
    }

    /// A type argument the checker left open (`Ok(1)`'s error type): it matches any type, and a record's `_` is fixed by its first typed use.
    pub fn wildcard() -> Self {
        TypeTerm::named("_")
    }

    fn is_named(&self, name: &str) -> bool {
        matches!(self, TypeTerm::Named(n, args) if n == name && args.is_empty())
    }

    /// Whether a value whose type is `self` fits `target`: the checker's assignability, where an `Any` inside fits only `Any`.
    pub fn fits(&self, target: &TypeTerm) -> bool {
        if target.is_named("Any") || target.is_named("_") || self.is_named("_") {
            return true;
        }
        match (self, target) {
            (TypeTerm::Union(ms), _) => ms.iter().all(|m| m.fits(target)),
            (_, TypeTerm::Union(ms)) => ms.iter().any(|m| self.fits(m)),
            (TypeTerm::Nullable(a), TypeTerm::Nullable(b)) => a.fits(b),
            (_, TypeTerm::Nullable(inner)) => self.is_named("Null") || self.fits(inner),
            (TypeTerm::Named(a, aa), TypeTerm::Named(b, ba)) => {
                let covariant = matches!(a.as_str(), "Stream" | "Task" | "Shared");
                a == b && aa.len() == ba.len() && aa.iter().zip(ba).all(|(x, y)| if covariant { x.fits(y) } else { x.same_as(y) })
            }
            (TypeTerm::Tuple(a), TypeTerm::Tuple(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.fits(y)),
            (TypeTerm::Function { sendable: sa, params: pa, ret: ra }, TypeTerm::Function { sendable: sb, params: pb, ret: rb }) => {
                (*sa || !*sb) && pa.len() == pb.len() && pb.iter().zip(pa).all(|(want, have)| want.fits(have)) && (rb.is_named("Null") || ra.fits(rb))
            }
            _ => false,
        }
    }

    fn same_as(&self, target: &TypeTerm) -> bool {
        let all = |a: &[TypeTerm], b: &[TypeTerm]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_as(y));
        match (self, target) {
            _ if self.is_named("_") || target.is_named("_") => true,
            (TypeTerm::Named(a, aa), TypeTerm::Named(b, ba)) => a == b && all(aa, ba),
            (TypeTerm::Nullable(a), TypeTerm::Nullable(b)) => a.same_as(b),
            (TypeTerm::Tuple(a), TypeTerm::Tuple(b)) => all(a, b),
            (TypeTerm::Function { sendable: sa, params: pa, ret: ra }, TypeTerm::Function { sendable: sb, params: pb, ret: rb }) => {
                sa == sb && all(pa, pb) && ra.same_as(rb)
            }
            (TypeTerm::Union(a), TypeTerm::Union(b)) => {
                a.iter().all(|x| b.iter().any(|y| x.same_as(y))) && b.iter().all(|y| a.iter().any(|x| x.same_as(y)))
            }
            _ => false,
        }
    }

    /// Whether this term has a `_` in it.
    pub fn has_wildcard(&self) -> bool {
        match self {
            TypeTerm::Named(n, ts) => n == "_" || ts.iter().any(TypeTerm::has_wildcard),
            TypeTerm::Tuple(ts) | TypeTerm::Union(ts) => ts.iter().any(TypeTerm::has_wildcard),
            TypeTerm::Nullable(t) => t.has_wildcard(),
            TypeTerm::Function { params, ret, .. } => params.iter().any(TypeTerm::has_wildcard) || ret.has_wildcard(),
            TypeTerm::Reg(_) | TypeTerm::ArgOf(..) => false,
        }
    }

    /// This type with each `_` replaced by the part of `target` it was checked against.
    pub fn refined(&self, target: &TypeTerm) -> TypeTerm {
        let all = |a: &[TypeTerm], b: &[TypeTerm]| a.iter().zip(b).map(|(x, y)| x.refined(y)).collect();
        match (self, target) {
            _ if self.is_named("_") => target.clone(),
            (_, TypeTerm::Nullable(t)) if !matches!(self, TypeTerm::Nullable(_)) => self.refined(t),
            (TypeTerm::Named(a, aa), TypeTerm::Named(_, ba)) if aa.len() == ba.len() => TypeTerm::Named(a.clone(), all(aa, ba)),
            (TypeTerm::Nullable(a), TypeTerm::Nullable(b)) => TypeTerm::Nullable(Box::new(a.refined(b))),
            (TypeTerm::Tuple(a), TypeTerm::Tuple(b)) if a.len() == b.len() => TypeTerm::Tuple(all(a, b)),
            (TypeTerm::Function { sendable, params: pa, ret: ra }, TypeTerm::Function { params: pb, ret: rb, .. }) if pa.len() == pb.len() => {
                TypeTerm::Function { sendable: *sendable, params: all(pa, pb), ret: Box::new(ra.refined(rb)) }
            }
            _ => self.clone(),
        }
    }

    /// Appends the `.mbc` encoding of this term to `buf`.
    pub fn encode(&self, buf: &mut Vec<u8>) {
        let list = |buf: &mut Vec<u8>, ts: &[TypeTerm]| {
            buf.extend_from_slice(&(ts.len() as u16).to_le_bytes());
            ts.iter().for_each(|t| t.encode(buf));
        };
        match self {
            TypeTerm::Named(name, args) => {
                buf.push(0);
                buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
                buf.extend_from_slice(name.as_bytes());
                list(buf, args);
            }
            TypeTerm::Nullable(inner) => {
                buf.push(1);
                inner.encode(buf);
            }
            TypeTerm::Tuple(elems) => {
                buf.push(2);
                list(buf, elems);
            }
            TypeTerm::Function { sendable, params, ret } => {
                buf.push(3);
                buf.push(*sendable as u8);
                list(buf, params);
                ret.encode(buf);
            }
            TypeTerm::Union(members) => {
                buf.push(6);
                list(buf, members);
            }
            TypeTerm::Reg(r) => buf.extend_from_slice(&[4, *r]),
            TypeTerm::ArgOf(r, i) => buf.extend_from_slice(&[5, *r, *i]),
        }
    }

    /// Whether this term reads registers, so it must be resolved per frame.
    pub fn is_pattern(&self) -> bool {
        match self {
            TypeTerm::Reg(_) | TypeTerm::ArgOf(..) => true,
            TypeTerm::Named(_, ts) | TypeTerm::Tuple(ts) | TypeTerm::Union(ts) => ts.iter().any(TypeTerm::is_pattern),
            TypeTerm::Nullable(t) => t.is_pattern(),
            TypeTerm::Function { params, ret, .. } => params.iter().any(TypeTerm::is_pattern) || ret.is_pattern(),
        }
    }

    /// This pattern with each register form replaced by `lookup`'s answer; `None` if one has none.
    pub fn resolve(&self, lookup: &impl Fn(&TypeTerm) -> Option<TypeTerm>) -> Option<TypeTerm> {
        let all = |ts: &[TypeTerm]| ts.iter().map(|t| t.resolve(lookup)).collect::<Option<Vec<_>>>();
        Some(match self {
            TypeTerm::Reg(_) | TypeTerm::ArgOf(..) => lookup(self)?,
            TypeTerm::Named(n, ts) => TypeTerm::Named(n.clone(), all(ts)?),
            TypeTerm::Tuple(ts) => TypeTerm::Tuple(all(ts)?),
            TypeTerm::Union(ts) => TypeTerm::Union(all(ts)?),
            TypeTerm::Nullable(t) => TypeTerm::Nullable(Box::new(t.resolve(lookup)?)),
            TypeTerm::Function { sendable, params, ret } => {
                TypeTerm::Function { sendable: *sendable, params: all(params)?, ret: Box::new(ret.resolve(lookup)?) }
            }
        })
    }

    /// Reads a term written by [`TypeTerm::encode`] at `*cur`, advancing it.
    pub fn decode(bytes: &[u8], cur: &mut usize) -> Result<Self, String> {
        let take = |cur: &mut usize, n: usize| -> Result<&[u8], String> {
            let s = bytes.get(*cur..*cur + n).ok_or("Unexpected EOF reading a type term")?;
            *cur += n;
            Ok(s)
        };
        let count = |cur: &mut usize| -> Result<usize, String> {
            let b = take(cur, 2)?;
            Ok(u16::from_le_bytes([b[0], b[1]]) as usize)
        };
        let list = |cur: &mut usize| -> Result<Vec<TypeTerm>, String> {
            let n = count(cur)?;
            (0..n).map(|_| TypeTerm::decode(bytes, cur)).collect()
        };
        Ok(match take(cur, 1)?[0] {
            0 => {
                let n = count(cur)?;
                let name = std::str::from_utf8(take(cur, n)?).map_err(|e| format!("Invalid UTF-8 type name: {e}"))?.to_string();
                TypeTerm::Named(name, list(cur)?)
            }
            1 => TypeTerm::Nullable(Box::new(TypeTerm::decode(bytes, cur)?)),
            2 => TypeTerm::Tuple(list(cur)?),
            3 => {
                let sendable = take(cur, 1)?[0] != 0;
                let params = list(cur)?;
                TypeTerm::Function { sendable, params, ret: Box::new(TypeTerm::decode(bytes, cur)?) }
            }
            6 => TypeTerm::Union(list(cur)?),
            4 => TypeTerm::Reg(take(cur, 1)?[0]),
            5 => {
                let b = take(cur, 2)?;
                TypeTerm::ArgOf(b[0], b[1])
            }
            tag => return Err(format!("Unknown type term tag {tag}")),
        })
    }
}

/// Strips a `_m<hex>_<stem>_` module-mangle prefix, so a cross-module `Point` shows as `Point`.
pub fn display_type_name(mangled: &str) -> &str {
    let Some(rest) = mangled.strip_prefix("_m") else { return mangled };
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_hexdigit());
    let Some(rest) = rest.strip_prefix('_') else { return mangled };
    match rest.split_once('_') {
        Some((stem, name)) if !stem.is_empty() && !name.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric()) => {
            name
        }
        _ => mangled,
    }
}

impl fmt::Display for TypeTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn list(ts: &[TypeTerm]) -> String {
            ts.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ")
        }
        match self {
            TypeTerm::Named(name, args) if args.is_empty() => write!(f, "{}", display_type_name(name)),
            TypeTerm::Named(name, args) => write!(f, "{}<{}>", display_type_name(name), list(args)),
            TypeTerm::Nullable(inner) if matches!(**inner, TypeTerm::Union(_)) => write!(f, "({inner})?"),
            TypeTerm::Nullable(inner) => write!(f, "{inner}?"),
            TypeTerm::Tuple(elems) => write!(f, "({})", list(elems)),
            TypeTerm::Union(members) => {
                let shown: Vec<String> = members
                    .iter()
                    .map(|m| if matches!(m, TypeTerm::Function { .. }) { format!("({m})") } else { m.to_string() })
                    .collect();
                write!(f, "{}", shown.join(" | "))
            }
            TypeTerm::Function { sendable, params, ret } => {
                write!(f, "{}({}) -> {ret}", if *sendable { "Send " } else { "" }, list(params))
            }
            TypeTerm::Reg(r) => write!(f, "<type in r{r}>"),
            TypeTerm::ArgOf(r, i) => write!(f, "<argument {i} of r{r}>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_term_round_trips_and_prints_like_the_checker() {
        let term = TypeTerm::Named(
            "Map".into(),
            vec![
                TypeTerm::named("String"),
                TypeTerm::Function {
                    sendable: false,
                    params: vec![TypeTerm::Tuple(vec![TypeTerm::named("Int"), TypeTerm::named("Bool")])],
                    ret: Box::new(TypeTerm::Nullable(Box::new(TypeTerm::named("_m1f_shapes_Point")))),
                },
            ],
        );
        let mut buf = Vec::new();
        term.encode(&mut buf);
        let mut cur = 0;
        assert_eq!(TypeTerm::decode(&buf, &mut cur).unwrap(), term);
        assert_eq!(cur, buf.len());
        assert_eq!(term.to_string(), "Map<String, ((Int, Bool)) -> Point?>");
    }

    #[test]
    fn a_pattern_round_trips_and_resolves_its_registers() {
        let pattern = TypeTerm::Named("Map".into(), vec![TypeTerm::Reg(3), TypeTerm::ArgOf(0, 1)]);
        let mut buf = Vec::new();
        pattern.encode(&mut buf);
        let mut cur = 0;
        assert_eq!(TypeTerm::decode(&buf, &mut cur).unwrap(), pattern);
        assert!(pattern.is_pattern() && !TypeTerm::named("Int").is_pattern());
        let lookup = |t: &TypeTerm| match t {
            TypeTerm::Reg(3) => Some(TypeTerm::named("String")),
            TypeTerm::ArgOf(0, 1) => Some(TypeTerm::named("Int")),
            _ => None,
        };
        assert_eq!(pattern.resolve(&lookup).unwrap().to_string(), "Map<String, Int>");
        assert!(TypeTerm::Reg(9).resolve(&lookup).is_none());
    }

    #[test]
    fn fits_follows_variance_and_open_arguments() {
        let n = |s: &str| TypeTerm::named(s);
        let of = |s: &str, args: Vec<TypeTerm>| TypeTerm::Named(s.into(), args);
        assert!(!of("List", vec![n("Int")]).fits(&of("List", vec![n("Any")])));
        assert!(of("Task", vec![n("Int")]).fits(&of("Task", vec![n("Any")])));
        assert!(!TypeTerm::Tuple(vec![n("Any")]).fits(&TypeTerm::Tuple(vec![n("Int")])));
        assert!(n("Null").fits(&TypeTerm::Nullable(Box::new(n("Int")))));
        let open = of("Result", vec![n("Int"), TypeTerm::wildcard()]);
        let target = of("Result", vec![n("Int"), n("String")]);
        assert!(open.fits(&target) && open.has_wildcard());
        assert_eq!(open.refined(&target), target);
    }

    #[test]
    fn a_union_holds_each_member_and_compares_as_a_set() {
        let n = |s: &str| TypeTerm::named(s);
        let of = |s: &str, args: Vec<TypeTerm>| TypeTerm::Named(s.into(), args);
        let u = TypeTerm::Union(vec![n("Int"), n("String")]);
        assert!(n("Int").fits(&u) && !n("Bool").fits(&u));
        assert!(u.fits(&TypeTerm::Union(vec![n("Bool"), n("String"), n("Int")])) && !u.fits(&n("Int")));
        let flipped = of("List", vec![TypeTerm::Union(vec![n("String"), n("Int")])]);
        assert!(of("List", vec![u.clone()]).fits(&flipped));
        let opt = TypeTerm::Nullable(Box::new(u.clone()));
        assert!(n("Null").fits(&opt) && n("String").fits(&opt));
        let mut buf = Vec::new();
        opt.encode(&mut buf);
        assert_eq!(TypeTerm::decode(&buf, &mut 0).unwrap(), opt);
        assert_eq!(opt.to_string(), "(Int | String)?");
    }
}
