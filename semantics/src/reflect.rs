//! What `std.meta` knows about a type: gathered for each type a program asks about, with its type arguments in.

use mote_base::{Interner, Span};
use mote_syntax as ast;

use crate::defs::{Def, DefId, DefKind, LangItem, Payload, Receiver};
use crate::program::Program;
use crate::types::{TyId, TyKind};

/// The most types reflection reaches in one program.
pub const MAX_REFLECTED: usize = 4096;

/// What a type is, in the order `std.meta.Kind` declares.
pub use mote_abi::Reflected as Kind;

/// How a variant carries its payload, in the order `std.meta.Shape` declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Unit,
    Tuple,
    Named,
}

/// `Field::flags` bits.
pub mod field_flag {
    pub const VAR: i128 = 1;
    pub const PUB: i128 = 2;
    pub const DEFAULT: i128 = 4;
}

/// `MethodInfo::flags` bits and the receiver in the bits above.
pub mod method_flag {
    pub const PUB: i128 = 1;
    /// The receiver is `self`: 1 shifted left of this many bits for `self`, 2 for `var self`.
    pub const RECEIVER_SHIFT: u32 = 1;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldInfo {
    pub name: String,
    pub ty: TyId,
    pub doc: String,
    pub flags: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantInfo {
    pub name: String,
    pub doc: String,
    pub shape: Shape,
    pub discriminant: Option<i128>,
    pub fields: Vec<FieldInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodInfo {
    pub name: String,
    pub signature: String,
    pub flags: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeInfo {
    pub name: String,
    /// Each argument as its key (empty when bare) and its value as written.
    pub args: Vec<(String, String)>,
}

/// A type that `std.meta` can describe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reflected {
    pub ty: TyId,
    pub name: String,
    /// The declared name of a struct, class or enum without its module or type arguments; the same as `name` for any other type.
    pub short: String,
    pub doc: String,
    pub kind: Kind,
    /// The type arguments, the element type, the members of a tuple or union, or a function's parameters then its result.
    pub args: Vec<TyId>,
    pub generics: Vec<String>,
    pub traits: Vec<String>,
    pub methods: Vec<MethodInfo>,
    pub attributes: Vec<AttributeInfo>,
    /// The fields of a struct or class, with the type arguments in.
    pub fields: Vec<FieldInfo>,
    pub variants: Vec<VariantInfo>,
}

impl Reflected {
    /// Every type this one holds or is made of.
    fn inner(&self) -> impl Iterator<Item = TyId> + '_ {
        self.args.iter().copied().chain(self.fields.iter().map(|field| field.ty)).chain(self.variants.iter().flat_map(|variant| variant.fields.iter().map(|field| field.ty)))
    }
}

/// Why a type cannot be described.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReflectError {
    /// The type still has a parameter, a variable or an error.
    Open,
    /// The program reaches more than [`MAX_REFLECTED`] types, or types nested too deeply.
    TooMany,
}

impl Program {
    /// The position of the type in `reflected`, once it and everything inside it is described.
    pub fn reflect(&mut self, ty: TyId, interner: &Interner) -> Result<u32, ReflectError> {
        if let Some(&found) = self.reflect_index.get(&ty) {
            return Ok(found);
        }
        let mut work = vec![ty];
        while let Some(next) = work.pop() {
            if self.reflect_index.contains_key(&next) {
                continue;
            }
            let types = &self.types;
            if types.has_param(next) || types.has_infer(next) || types.has_error(next) {
                return Err(ReflectError::Open);
            }
            if self.reflected.len() >= MAX_REFLECTED || types.depth(next) > 32 {
                return Err(ReflectError::TooMany);
            }
            let entry = self.describe(next, interner);
            self.reflect_index.insert(next, self.reflected.len() as u32);
            work.extend(entry.inner());
            self.reflected.push(entry);
        }
        Ok(self.reflect_index[&ty])
    }

    fn describe(&mut self, ty: TyId, interner: &Interner) -> Reflected {
        let name = self.render_source(ty, interner);
        let mut entry = Reflected {
            ty,
            short: name.clone(),
            name,
            doc: String::new(),
            kind: Kind::Null,
            args: Vec::new(),
            generics: Vec::new(),
            traits: Vec::new(),
            methods: Vec::new(),
            attributes: Vec::new(),
            fields: Vec::new(),
            variants: Vec::new(),
        };
        match self.types.kind(ty).clone() {
            TyKind::Unit => entry.kind = Kind::Null,
            TyKind::Bool => entry.kind = Kind::Bool,
            TyKind::String => entry.kind = Kind::String,
            TyKind::Int(_) => entry.kind = Kind::Int,
            TyKind::Float(_) => entry.kind = Kind::Float,
            TyKind::Tuple(members) => {
                entry.kind = Kind::Tuple;
                entry.args = members;
            }
            TyKind::Union(members) => {
                entry.kind = Kind::Union;
                entry.args = members;
            }
            TyKind::Array(element) | TyKind::FixedArray(element, _) => {
                entry.kind = Kind::Array;
                entry.args = vec![element];
            }
            TyKind::Optional(element) => {
                entry.kind = Kind::Optional;
                entry.args = vec![element];
            }
            TyKind::Function(function) => {
                entry.kind = Kind::Function;
                entry.args = function.params.iter().map(|param| param.ty).chain([function.ret]).collect();
            }
            TyKind::Adt { def, args } => self.describe_adt(&mut entry, def, args, interner),
            TyKind::Error | TyKind::Param { .. } | TyKind::Infer(_) => {}
        }
        entry
    }

    fn describe_adt(&mut self, entry: &mut Reflected, def: DefId, args: Vec<TyId>, interner: &Interner) {
        let item = &self.defs[def];
        let module = &self.modules[item.module];
        let ast_item = item.origin.map(|origin| &module.ast[origin.item]);
        entry.short = interner.resolve(item.name).to_string();
        entry.doc = module.doc_text(ast_item.and_then(|found| found.doc));
        entry.attributes = ast_item.map(|found| found.attrs.iter().map(|attr| attribute_info(attr, interner)).collect()).unwrap_or_default();
        let opaque = match item.lang {
            Some(LangItem::List) => {
                entry.kind = Kind::List;
                true
            }
            Some(LangItem::Map) => {
                entry.kind = Kind::Map;
                true
            }
            Some(LangItem::Set) => {
                entry.kind = Kind::Set;
                true
            }
            Some(LangItem::Array) => {
                entry.kind = Kind::Array;
                true
            }
            Some(lang) if lang.is_handle() => {
                entry.kind = Kind::Handle;
                true
            }
            _ => false,
        };
        entry.args = args.clone();
        let (generics, traits, methods) = match &item.kind {
            DefKind::Record(record) => (&record.generics, &record.traits, &record.methods),
            DefKind::Enum(en) => (&en.generics, &en.traits, &en.methods),
            _ => return,
        };
        entry.generics = generics.iter().map(|param| interner.resolve(param.name).to_string()).collect();
        entry.traits = traits.iter().map(|bound| self.bound_text(bound.def, &bound.args, interner)).collect();
        entry.methods = methods.iter().map(|&method| self.method_info(method, interner)).collect();
        match &item.kind {
            DefKind::Record(record) if !opaque => {
                entry.kind = if record.is_class { Kind::Class } else { Kind::Struct };
                let docs = match ast_item.map(|found| &found.kind) {
                    Some(ast::ItemKind::Record(found)) => found.members.iter().filter_map(|member| if let ast::Member::Field(field) = member { Some(field.doc) } else { None }).collect(),
                    _ => Vec::new(),
                };
                let declared: Vec<(String, TyId, bool, bool, bool)> = record.fields.iter().map(|field| (interner.resolve(field.name).to_string(), field.ty, field.is_var, field.is_pub, field.default.is_some())).collect();
                let types = &mut self.types;
                entry.fields = declared
                    .into_iter()
                    .enumerate()
                    .map(|(at, (name, ty, is_var, is_pub, has_default))| FieldInfo { name, ty: types.subst(ty, &args), doc: module.doc_text(docs.get(at).copied().flatten()), flags: field_flags(is_var, is_pub, has_default) })
                    .collect();
            }
            DefKind::Enum(en) => {
                entry.kind = Kind::Enum;
                let written: &[ast::Variant] = match ast_item.map(|found| &found.kind) {
                    Some(ast::ItemKind::Enum(found)) => &found.variants,
                    _ => &[],
                };
                let types = &mut self.types;
                entry.variants = en
                    .variants
                    .iter()
                    .enumerate()
                    .map(|(at, variant)| {
                        let source = written.get(at);
                        let doc = module.doc_text(source.and_then(|found| found.doc));
                        let (shape, fields) = match &variant.payload {
                            Payload::Unit => (Shape::Unit, Vec::new()),
                            Payload::Tuple(members) => (Shape::Tuple, members.iter().map(|&ty| FieldInfo { name: String::new(), ty: types.subst(ty, &args), doc: String::new(), flags: field_flag::PUB }).collect()),
                            Payload::Struct(members) => {
                                let docs: Vec<Option<Span>> = match source.map(|found| &found.payload) {
                                    Some(ast::VariantPayload::Struct(found)) => found.iter().map(|field| field.doc).collect(),
                                    _ => Vec::new(),
                                };
                                let fields = members
                                    .iter()
                                    .enumerate()
                                    .map(|(index, field)| FieldInfo {
                                        name: interner.resolve(field.name).to_string(),
                                        ty: types.subst(field.ty, &args),
                                        doc: module.doc_text(docs.get(index).copied().flatten()),
                                        flags: field_flags(field.is_var, true, field.default.is_some()),
                                    })
                                    .collect();
                                (Shape::Named, fields)
                            }
                        };
                        VariantInfo { name: interner.resolve(variant.name).to_string(), doc, shape, discriminant: variant.discriminant, fields }
                    })
                    .collect();
            }
            _ => {}
        }
    }

    fn bound_text(&self, def: DefId, args: &[TyId], interner: &Interner) -> String {
        let name = interner.resolve(self.defs[def].name);
        if args.is_empty() {
            return name.to_string();
        }
        let shown: Vec<String> = args.iter().map(|&arg| self.render(arg, interner)).collect();
        format!("{name}<{}>", shown.join(", "))
    }

    fn method_info(&self, method: DefId, interner: &Interner) -> MethodInfo {
        let item: &Def = &self.defs[method];
        let name = interner.resolve(item.name).to_string();
        let DefKind::Function(function) = &item.kind else { return MethodInfo { signature: format!("fn {name}"), name, flags: 0 } };
        let mut params = Vec::new();
        match function.receiver {
            Receiver::Read => params.push("self".to_string()),
            Receiver::Write => params.push("var self".to_string()),
            Receiver::None => {}
        }
        for param in &function.params {
            let ty = self.render(param.ty, interner);
            let name = interner.resolve(param.name);
            params.push(match (param.variadic, param.is_var) {
                (true, _) => format!("...{name}: {ty}"),
                (false, true) => format!("var {name}: {ty}"),
                (false, false) => format!("{name}: {ty}"),
            });
        }
        let generics = if function.generics.is_empty() { String::new() } else { format!("<{}>", function.generics.iter().map(|param| interner.resolve(param.name)).collect::<Vec<_>>().join(", ")) };
        let ret = if function.ret == TyId::UNIT { String::new() } else { format!(" -> {}", self.render(function.ret, interner)) };
        let receiver = match function.receiver {
            Receiver::None => 0,
            Receiver::Read => 1,
            Receiver::Write => 2,
        };
        MethodInfo { signature: format!("fn {name}{generics}({}){ret}", params.join(", ")), name, flags: i128::from(item.is_pub) | (receiver << method_flag::RECEIVER_SHIFT) }
    }
}

fn field_flags(is_var: bool, is_pub: bool, has_default: bool) -> i128 {
    let mut flags = 0;
    if is_var {
        flags |= field_flag::VAR;
    }
    if is_pub {
        flags |= field_flag::PUB;
    }
    if has_default {
        flags |= field_flag::DEFAULT;
    }
    flags
}

fn attribute_info(attr: &ast::Attribute, interner: &Interner) -> AttributeInfo {
    let args = attr
        .args
        .iter()
        .map(|arg| {
            let key = arg.key.as_ref().map(|path| path_text(path, interner)).unwrap_or_default();
            let value = match &arg.value {
                ast::AttrValue::Path(path) => path_text(path, interner),
                ast::AttrValue::String(text) => interner.resolve(*text).to_string(),
                ast::AttrValue::Int(literal) => literal.value.to_string(),
                ast::AttrValue::Float(literal) => literal.value.to_string(),
                ast::AttrValue::Bool(flag) => flag.to_string(),
            };
            (key, value)
        })
        .collect();
    AttributeInfo { name: interner.resolve(attr.name.name).to_string(), args }
}

fn path_text(path: &ast::Path, interner: &Interner) -> String {
    path.segments.iter().map(|segment| interner.resolve(segment.name)).collect::<Vec<_>>().join(".")
}
