//! A synthetic type table and helpers for the heap tests.

use std::sync::Arc;

use mote_abi::{FrameDescriptor, HandleKind, Kind, Region, Scalar, TypeId, TypeTable};

use crate::{Collection, Config, Heap, Marker, Object, Roots};

/// Offsets in the fixture's types.
pub const NODE_VALUE: u32 = 8;
pub const NODE_NEXT: u32 = 16;
pub const HOLDER_TAG: u32 = 8;
pub const HOLDER_PAYLOAD: u32 = 16;
pub const PAIRS_TYPE: u32 = 8;
pub const PAIRS_PAYLOAD: u32 = 16;
pub const TRIPLE_FIRST: u32 = 8;
pub const LIST_BUFFER: u32 = 24;
pub const CLOSURE_CAPTURE: u32 = 16;

pub struct Fixture {
    pub table: Arc<TypeTable>,
    pub int: TypeId,
    /// A class of an `Int` and a `Node?`.
    pub node: TypeId,
    pub string: TypeId,
    /// `[Int]`, with no references.
    pub ints: TypeId,
    /// `[Node?]`.
    pub nodes: TypeId,
    /// An enum of a `Node` or an `Int`.
    pub shape: TypeId,
    /// A class holding a `Shape`.
    pub holder: TypeId,
    /// A class holding a union pair.
    pub pairs: TypeId,
    /// A class holding `[Node?; 3]`.
    pub triple: TypeId,
    pub list: TypeId,
    pub closure: TypeId,
    pub port: TypeId,
}

impl Fixture {
    pub fn new() -> Self {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let node = table.reserve_object("Node");
        let maybe_node = table.add("Node?", Kind::Optional(node));
        table.define(node, Kind::Class(vec![int, maybe_node]));
        let string = table.add("String", Kind::String);
        let ints = table.add("[Int]", Kind::Array(int));
        let nodes = table.add("[Node?]", Kind::Array(maybe_node));
        let shape = table.add("Shape", Kind::Enum(vec![vec![node], vec![int]]));
        let holder = table.add("Holder", Kind::Class(vec![shape]));
        let union = table.add("Node | Int", Kind::Pair);
        let pairs = table.add("Pairs", Kind::Class(vec![union]));
        let fixed = table.add("[Node?; 3]", Kind::FixedArray(maybe_node, 3));
        let triple = table.add("Triple", Kind::Class(vec![fixed]));
        let list = table.add("List<Int>", Kind::List(int));
        let closure = table.add("fn#1", Kind::Closure(vec![node]));
        let port = table.add("Port", Kind::Handle(HandleKind::Port, Vec::new()));
        Self { table: Arc::new(table), int, node, string, ints, nodes, shape, holder, pairs, triple, list, closure, port }
    }
}

/// The roots of a test: a set of objects, and optionally a frame.
#[derive(Default)]
pub struct Held {
    pub objects: Vec<Object>,
    pub frame: Option<(Vec<u64>, FrameDescriptor)>,
}

impl Roots for Held {
    fn visit(&mut self, marker: &mut Marker<'_>) {
        for object in &self.objects {
            marker.mark(Some(*object));
        }
        if let Some((words, descriptor)) = &self.frame {
            marker.mark_frame(words.as_ptr() as usize, descriptor);
        }
    }

    fn retire(&mut self) {}
}

/// A heap with its fixture and one region.
pub struct Env {
    pub fx: Fixture,
    pub heap: Heap,
    pub region: Region,
}

pub fn config() -> Config {
    Config { limit: 256 << 20, trigger: usize::MAX, retain: 16, torture: false, verify: true }
}

impl Env {
    pub fn new() -> Self {
        Self::with(config())
    }

    pub fn with(config: Config) -> Self {
        let fx = Fixture::new();
        let heap = Heap::new(fx.table.clone(), config);
        Self { fx, heap, region: Region { cursor: 0, limit: 0 } }
    }

    pub fn alloc(&mut self, ty: TypeId, count: u64) -> Object {
        self.heap.alloc(&mut self.region, ty, count).expect("room in the heap")
    }

    pub fn node(&mut self, value: u64, next: Option<Object>) -> Object {
        let node = self.alloc(self.fx.node, 0);
        node.set_word(NODE_VALUE, value);
        node.set_word(NODE_NEXT, next.map_or(0, |next| next.address() as u64));
        node
    }

    /// Collects from the objects, checks the result, and empties the region.
    pub fn collect(&mut self, roots: &[Object]) -> Collection {
        let mut held = Held { objects: roots.to_vec(), frame: None };
        self.collect_held(&mut held, 1)
    }

    pub fn collect_held(&mut self, held: &mut Held, workers: usize) -> Collection {
        let found = self.heap.collect_parallel(held, workers);
        self.region = Region { cursor: 0, limit: 0 };
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        found
    }
}

pub fn address(object: Option<Object>) -> u64 {
    object.map_or(0, |object| object.address() as u64)
}
