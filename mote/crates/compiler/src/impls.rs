//! Trait lists on declarations: default methods copied into the types that list the trait.

use std::collections::HashMap;

use crate::ast::*;

/// The trait's own name in a trait reference.
pub(crate) fn trait_ref_name(node: &TypeNode) -> Option<&str> {
    match node {
        TypeNode::Named(n, _) | TypeNode::Generic(n, _, _) => Some(n),
        _ => None,
    }
}

/// For every type that lists a trait, copies each default method the type lacks into the type. Run once over all modules
/// after names are resolved, so a trait from another module is found by its final name.
pub fn copy_trait_defaults(programs: &mut [&mut Program]) {
    let mut traits: HashMap<String, TraitDecl> = HashMap::new();
    for p in programs.iter() {
        for item in &p.items {
            if let Item::Trait(t) = item {
                traits.insert(t.name.clone(), t.clone());
            }
        }
    }
    for p in programs.iter_mut() {
        for item in p.items.iter_mut() {
            let (listed, methods): (&[TypeNode], &mut Vec<FunctionDecl>) = match item {
                Item::Struct(s) => (&s.traits, &mut s.methods),
                Item::Class(c) => (&c.traits, &mut c.methods),
                Item::Enum(e) => (&e.traits, &mut e.methods),
                _ => continue,
            };
            for node in listed {
                let Some(t) = trait_ref_name(node).and_then(|n| traits.get(n)) else { continue };
                for method in methods.iter_mut().filter(|x| t.members.iter().any(|m| m.name == x.name)) {
                    method.is_pub = true;
                }
                for m in &t.members {
                    let Some(body) = &m.default_body else { continue };
                    if methods.iter().any(|x| x.name == m.name) {
                        continue;
                    }
                    methods.push(FunctionDecl {
                        name: m.name.clone(),
                        generic_params: m.generic_params.clone(),
                        params: m.params.clone(),
                        return_type: m.return_type.clone(),
                        body: body.clone(),
                        is_pub: true,
                        span: m.span,
                    });
                }
            }
        }
    }
}
