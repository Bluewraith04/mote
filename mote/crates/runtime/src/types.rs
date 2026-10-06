//! Runtime type arguments: interned type ids and the descriptors that record them.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use isa::type_term::TypeTerm;
use isa::value::TypeDescriptor;

/// Interned types and per-instantiation descriptors made at run time; shared by every worker.
#[derive(Default)]
pub struct TypeInterner {
    static_ids: RwLock<HashMap<usize, u32>>,
    interned: RwLock<Interned>,
    instances: Mutex<HashMap<(u64, usize, u32), &'static TypeDescriptor>>,
}

#[derive(Default)]
struct Interned {
    terms: Vec<TypeTerm>,
    ids: HashMap<TypeTerm, u32>,
}

impl TypeInterner {
    /// The id of `term`, the register-free term on descriptor `idx`; interned once.
    pub(crate) fn static_id(&self, idx: usize, term: &TypeTerm) -> u32 {
        if let Some(&id) = self.static_ids.read().unwrap().get(&idx) {
            return id;
        }
        let id = self.intern(term);
        self.static_ids.write().unwrap().insert(idx, id);
        id
    }

    /// The id of `term`, interning it on first sight.
    pub(crate) fn intern(&self, term: &TypeTerm) -> u32 {
        if let Some(&id) = self.interned.read().unwrap().ids.get(term) {
            return id;
        }
        let mut w = self.interned.write().unwrap();
        if let Some(&id) = w.ids.get(term) {
            return id;
        }
        let id = w.terms.len() as u32;
        w.terms.push(term.clone());
        w.ids.insert(term.clone(), id);
        id
    }

    /// The type with id `id`.
    pub fn get(&self, id: u32) -> Option<TypeTerm> {
        self.interned.read().unwrap().terms.get(id as usize).cloned()
    }

    /// `template` recording type `id`: one descriptor per `(template, type)`.
    pub fn instance(&self, template: &TypeDescriptor, id: u32) -> Option<&'static TypeDescriptor> {
        let key = (template.id, template.fields.len(), id);
        let mut map = self.instances.lock().unwrap();
        if let Some(&d) = map.get(&key) {
            return Some(d);
        }
        let desc: &'static TypeDescriptor = Box::leak(Box::new(TypeDescriptor { instance: Some(self.get(id)?), ..template.clone() }));
        map.insert(key, desc);
        Some(desc)
    }
}
