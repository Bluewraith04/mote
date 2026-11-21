use mote_abi::{FrameDescriptor, Header, Slot, FLAG_STATIC};

use crate::fixture::{address, Env, Held, CLOSURE_CAPTURE, HOLDER_PAYLOAD, HOLDER_TAG, LIST_BUFFER, NODE_NEXT, NODE_VALUE, PAIRS_PAYLOAD, PAIRS_TYPE, TRIPLE_FIRST};
use crate::{Object, LINE_SIZE};

#[test]
fn mark_a_chain_keeps_every_node_and_only_those() {
    let mut env = Env::new();
    let mut head = None;
    let mut chain = Vec::new();
    for value in 0..1000 {
        head = Some(env.node(value, head));
        chain.push(head.unwrap());
        env.node(9999, None);
    }
    let found = env.collect(&[head.unwrap()]);
    assert_eq!((found.marked_objects, found.marked_bytes), (1000, 24_000));
    assert!(chain.iter().all(|node| env.heap.is_marked(*node)));
    assert_eq!(env.heap.stats().marked_objects, 1000);
}

#[test]
fn mark_an_unreferenced_object_is_not_marked() {
    let mut env = Env::new();
    let live = env.node(1, None);
    let dead = env.node(2, None);
    env.collect(&[live]);
    assert!(env.heap.is_marked(live));
    assert!(!env.heap.is_marked(dead));
}

#[test]
fn mark_a_cycle_and_a_shared_object_are_visited_once() {
    let mut env = Env::new();
    let a = env.node(1, None);
    let b = env.node(2, Some(a));
    a.set_word(NODE_NEXT, address(Some(b)));
    let found = env.collect(&[a, b, a]);
    assert_eq!(found.marked_objects, 2);
}

#[test]
fn mark_a_null_root_marks_nothing() {
    let mut env = Env::new();
    env.node(1, None);
    let mut held = Held::default();
    let found = env.collect_held(&mut held, 1);
    assert_eq!(found.marked_objects, 0);
}

#[test]
fn mark_scans_the_elements_of_an_array_of_references() {
    let mut env = Env::new();
    let array = env.alloc(env.fx.nodes, 4);
    let kept: Vec<_> = (0..3).map(|value| env.node(value, None)).collect();
    for (at, node) in kept.iter().enumerate() {
        array.set_word(16 + 8 * at as u32, address(Some(*node)));
    }
    let dead = env.node(7, None);
    env.collect(&[array]);
    assert!(kept.iter().all(|node| env.heap.is_marked(*node)));
    assert!(!env.heap.is_marked(dead));
}

#[test]
fn mark_does_not_scan_an_array_of_integers() {
    let mut env = Env::new();
    let target = env.node(1, None);
    let ints = env.alloc(env.fx.ints, 2);
    ints.set_word(16, address(Some(target)));
    let text = env.alloc(env.fx.string, 8);
    text.set_word(16, address(Some(target)));
    env.collect(&[ints, text]);
    assert!(!env.heap.is_marked(target));
    assert!(env.heap.is_marked(ints) && env.heap.is_marked(text));
}

#[test]
fn mark_follows_an_enum_variant_chosen_by_its_tag() {
    let mut env = Env::new();
    let target = env.node(1, None);
    let holds_node = env.alloc(env.fx.holder, 0);
    holds_node.set_word(HOLDER_TAG, 0);
    holds_node.set_word(HOLDER_PAYLOAD, address(Some(target)));
    let holds_int = env.alloc(env.fx.holder, 0);
    holds_int.set_word(HOLDER_TAG, 1);
    holds_int.set_word(HOLDER_PAYLOAD, address(Some(target)));
    env.collect(&[holds_int]);
    assert!(!env.heap.is_marked(target), "an integer that looks like a pointer is not one");
    env.collect(&[holds_node]);
    assert!(env.heap.is_marked(target));
}

#[test]
fn mark_follows_a_pair_payload_only_when_its_type_says_reference() {
    let mut env = Env::new();
    let target = env.node(1, None);
    let pair = env.alloc(env.fx.pairs, 0);
    pair.set_word(PAIRS_PAYLOAD, address(Some(target)));
    env.collect(&[pair]);
    assert!(!env.heap.is_marked(target), "the none pair holds no reference");
    pair.set_word(PAIRS_TYPE, u64::from(env.fx.int.raw()));
    env.collect(&[pair]);
    assert!(!env.heap.is_marked(target), "an inline Int is not a reference");
    pair.set_word(PAIRS_TYPE, u64::from(env.fx.node.raw()));
    env.collect(&[pair]);
    assert!(env.heap.is_marked(target));
}

#[test]
fn mark_scans_a_fixed_array_with_its_repeating_slot() {
    let mut env = Env::new();
    let triple = env.alloc(env.fx.triple, 0);
    let nodes: Vec<_> = (0..3).map(|value| env.node(value, None)).collect();
    for (at, node) in nodes.iter().enumerate() {
        triple.set_word(TRIPLE_FIRST + 8 * at as u32, address(Some(*node)));
    }
    env.collect(&[triple]);
    assert!(nodes.iter().all(|node| env.heap.is_marked(*node)));
}

#[test]
fn mark_a_list_keeps_its_buffer_and_a_closure_its_captures() {
    let mut env = Env::new();
    let buffer = env.alloc(env.fx.ints, 4);
    let list = env.alloc(env.fx.list, 0);
    list.set_word(LIST_BUFFER, address(Some(buffer)));
    let captured = env.node(3, None);
    let closure = env.alloc(env.fx.closure, 0);
    closure.set_word(CLOSURE_CAPTURE, address(Some(captured)));
    let dead = env.alloc(env.fx.ints, 4);
    env.collect(&[list, closure]);
    assert!(env.heap.is_marked(buffer) && env.heap.is_marked(captured));
    assert!(!env.heap.is_marked(dead));
}

#[test]
fn mark_sets_the_object_bit_at_the_header_and_every_line_the_object_covers() {
    let mut env = Env::new();
    let padding = env.node(0, None);
    let wide = env.alloc(env.fx.ints, 200);
    let size = 16 + 200 * 8;
    env.collect(&[wide]);
    assert!(env.heap.is_marked(wide) && !env.heap.is_marked(padding));
    let first = wide.address() / LINE_SIZE;
    let last = (wide.address() + size - 1) / LINE_SIZE;
    assert_eq!(env.heap.marked_lines(wide, size), last - first + 1);
    assert!(last - first >= 12);
}

#[test]
fn mark_the_mark_bits_are_outside_the_object() {
    let mut env = Env::new();
    let node = env.node(41, None);
    let header = node.header();
    env.collect(&[node]);
    assert_eq!((node.header(), node.word(NODE_VALUE), node.word(NODE_NEXT)), (header, 41, 0));
}

#[test]
fn mark_a_frame_marks_its_references_and_its_dynamic_slots() {
    let mut env = Env::new();
    let direct = env.node(1, None);
    let inner = env.node(2, None);
    let ignored = env.node(3, None);
    let descriptor = FrameDescriptor::new(32, vec![0], vec![Slot::Enum { offset: 16, ty: env.fx.shape }]).unwrap();
    let frame = vec![address(Some(direct)), address(Some(ignored)), 0, 0];
    let mut held = Held { objects: Vec::new(), frame: Some((frame, descriptor)) };
    // Words 2 and 3 are the shape: tag 0 at word 2, a node at word 3.
    held.frame.as_mut().unwrap().0[3] = address(Some(inner));
    env.collect_held(&mut held, 1);
    assert!(env.heap.is_marked(direct) && env.heap.is_marked(inner));
    assert!(!env.heap.is_marked(ignored));
}

#[test]
fn mark_large_objects_are_marked_and_scanned_and_dead_ones_are_freed() {
    let mut env = Env::new();
    let kept = env.alloc(env.fx.nodes, 2000);
    let dead = env.alloc(env.fx.nodes, 2000);
    let target = env.node(5, None);
    kept.set_word(16 + 8 * 1999, address(Some(target)));
    let found = env.collect(&[kept]);
    assert!(env.heap.is_marked(kept) && env.heap.is_marked(target) && !env.heap.is_marked(dead));
    assert_eq!(found.large_freed, 1);
    assert_eq!(env.heap.census().large, 1);
    assert_eq!(env.heap.used_bytes(), 16384 + 32768);
}

#[test]
fn mark_a_second_collection_starts_from_clear_marks() {
    let mut env = Env::new();
    let a = env.node(1, None);
    let b = env.node(2, None);
    env.collect(&[a, b]);
    assert!(env.heap.is_marked(b));
    env.collect(&[a]);
    assert!(!env.heap.is_marked(b));
}

#[test]
fn static_a_static_object_is_neither_marked_nor_scanned() {
    let mut env = Env::new();
    let words: &'static mut [u64; 3] = Box::leak(Box::new([Header::new(env.fx.node, FLAG_STATIC).bits(), 7, 0x10]));
    let statics = Object::from_address(words.as_ptr() as usize).unwrap();
    let live = env.node(1, Some(statics));
    let found = env.collect(&[live]);
    assert_eq!(found.marked_objects, 1);
    assert!(env.heap.is_marked(live));
}
