//! The node tree: flat arrays indexed by node id, with taffy layout, damage tracking and hit testing.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use taffy::{AvailableSpace, Size, TaffyTree};

use crate::geom::Rect;
use crate::style::{NO_STYLE, Overflow, StyleDef, to_taffy};

mod edit;
mod interact;

pub use interact::{Input, UiEvent};

/// The id meaning "no node" in a link.
pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A freed slot.
    Free,
    Box,
    Text,
    Image,
    Button,
    Input,
    /// A multi-line input: wraps at its width, Enter inserts a line break.
    TextArea,
    Scroll,
}

impl Kind {
    pub fn from_code(code: i64) -> Option<Kind> {
        match code {
            0 => Some(Kind::Box),
            1 => Some(Kind::Text),
            2 => Some(Kind::Image),
            3 => Some(Kind::Button),
            4 => Some(Kind::Input),
            5 => Some(Kind::Scroll),
            6 => Some(Kind::TextArea),
            _ => None,
        }
    }

    /// Whether the kind edits text: `Input` and `TextArea`.
    pub fn is_input(self) -> bool {
        matches!(self, Kind::Input | Kind::TextArea)
    }

    /// Whether the node's own content (text or an image) gives it a size.
    fn has_content(self) -> bool {
        matches!(self, Kind::Text | Kind::Image | Kind::Button | Kind::Input | Kind::TextArea)
    }
}

/// One node: links, kind and the id of its style.
#[derive(Clone, Copy, Debug)]
pub struct UiNode {
    pub parent: u32,
    pub first_child: u32,
    pub last_child: u32,
    pub next_sibling: u32,
    pub kind: Kind,
    pub style_id: u32,
}

impl UiNode {
    const FREE: UiNode = UiNode { parent: NONE, first_child: NONE, last_child: NONE, next_sibling: NONE, kind: Kind::Free, style_id: NO_STYLE };
}

/// A stretch of a text node's text that looks different from the node's style: `len` bytes with the fields that are set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Run {
    pub len: usize,
    /// 0 keeps the node's weight.
    pub weight: u16,
    pub italic: bool,
    pub mono: bool,
    pub underline: bool,
    pub strike: bool,
    /// Packed `0xRRGGBBAA`; 0 keeps the node's colour.
    pub color: u32,
    /// What a click on the run reports; 0 is no link.
    pub link: u32,
}

impl Run {
    /// A run of `len` bytes that looks like the node.
    pub fn plain(len: usize) -> Run {
        Run { len, weight: 0, italic: false, mono: false, underline: false, strike: false, color: 0, link: 0 }
    }
}

/// What a text or image node holds, for measuring.
pub struct MeasureNode<'a> {
    pub kind: Kind,
    pub text: &'a str,
    pub runs: &'a [Run],
    pub style: &'a StyleDef,
}

/// Gives text its size. `max_width` is the width the text may wrap to, or `None` for no limit.
pub trait Measure {
    fn measure(&mut self, node: &MeasureNode<'_>, max_width: Option<f32>) -> [f32; 2];
}

pub struct Tree {
    nodes: Vec<UiNode>,
    rects: Vec<Rect>,
    clips: Vec<Rect>,
    visible: Vec<Rect>,
    scroll: Vec<[f32; 2]>,
    content: Vec<[f32; 2]>,
    image_sizes: Vec<[f32; 2]>,
    paint_dirty: Vec<bool>,
    taffy: TaffyTree<u32>,
    tnodes: Vec<taffy::NodeId>,
    free: Vec<u32>,
    /// Per node: its text.
    texts: Vec<String>,
    styles: Vec<StyleDef>,
    style_lookup: HashMap<u64, Vec<u32>>,
    default_style: StyleDef,
    order: Vec<u32>,
    order_dirty: bool,
    damage: Vec<Rect>,
    removed: Vec<u32>,
    viewport: Rect,
    /// Per node: the `interact` flags for hover, press and focus.
    state: Vec<u8>,
    /// The nodes under the pointer, innermost first.
    hover: Vec<u32>,
    /// The nodes held by the pressed pointer, innermost first.
    pressed: Vec<u32>,
    focus: u32,
    /// Caret and selection of the inputs that have had one.
    edits: HashMap<u32, edit::Edit>,
    /// Undo and redo snapshots of the inputs that have been edited.
    history: HashMap<u32, edit::History>,
    /// The runs of the text nodes that have them; they cover the node's whole text.
    runs: HashMap<u32, Vec<Run>>,
}

impl Tree {
    /// A tree holding only the root box, which fills a `width` by `height` window.
    pub fn new(width: f32, height: f32) -> Tree {
        let mut tree = Tree {
            nodes: Vec::new(),
            rects: Vec::new(),
            clips: Vec::new(),
            visible: Vec::new(),
            scroll: Vec::new(),
            content: Vec::new(),
            image_sizes: Vec::new(),
            paint_dirty: Vec::new(),
            taffy: TaffyTree::new(),
            tnodes: Vec::new(),
            free: Vec::new(),
            texts: Vec::new(),
            styles: Vec::new(),
            style_lookup: HashMap::new(),
            default_style: StyleDef::default(),
            order: Vec::new(),
            order_dirty: true,
            damage: Vec::new(),
            removed: Vec::new(),
            viewport: Rect::new(0.0, 0.0, width, height),
            state: Vec::new(),
            hover: Vec::new(),
            pressed: Vec::new(),
            focus: NONE,
            edits: HashMap::new(),
            history: HashMap::new(),
            runs: HashMap::new(),
        };
        tree.alloc(Kind::Box);
        tree.damage.push(tree.viewport);
        tree
    }

    pub fn root(&self) -> u32 {
        0
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// Resizes the window; everything is damaged.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        let next = Rect::new(0.0, 0.0, width, height);
        if next == self.viewport {
            return;
        }
        self.viewport = next;
        self.sync_style(0);
        self.damage.push(next);
    }

    fn alloc(&mut self, kind: Kind) -> u32 {
        let idx = match self.free.pop() {
            Some(i) => i,
            None => {
                self.nodes.push(UiNode::FREE);
                self.rects.push(Rect::ZERO);
                self.clips.push(Rect::ZERO);
                self.visible.push(Rect::ZERO);
                self.scroll.push([0.0; 2]);
                self.content.push([0.0; 2]);
                self.image_sizes.push([0.0; 2]);
                self.paint_dirty.push(false);
                self.texts.push(String::new());
                self.state.push(0);
                self.tnodes.push(taffy::NodeId::new(0));
                (self.nodes.len() - 1) as u32
            }
        };
        let i = idx as usize;
        self.nodes[i] = UiNode { kind, ..UiNode::FREE };
        self.rects[i] = Rect::ZERO;
        self.clips[i] = Rect::ZERO;
        self.visible[i] = Rect::ZERO;
        self.scroll[i] = [0.0; 2];
        self.content[i] = [0.0; 2];
        self.image_sizes[i] = [0.0; 2];
        self.paint_dirty[i] = true;
        self.texts[i].clear();
        self.state[i] = 0;
        let style = self.taffy_style(idx);
        self.tnodes[i] = self.taffy.new_leaf_with_context(style, idx).expect("taffy leaf");
        self.order_dirty = true;
        idx
    }

    fn live(&self, node: u32) -> Result<usize, String> {
        match self.nodes.get(node as usize) {
            Some(n) if n.kind != Kind::Free => Ok(node as usize),
            _ => Err(format!("no node {node}")),
        }
    }

    /// Adds a child of `kind` after the parent's last child.
    pub fn add(&mut self, parent: u32, kind: Kind) -> Result<u32, String> {
        let p = self.live(parent)?;
        if kind == Kind::Free {
            return Err("cannot add a free node".to_string());
        }
        let node = self.alloc(kind);
        let last = self.nodes[p].last_child;
        if last == NONE {
            self.nodes[p].first_child = node;
        } else {
            self.nodes[last as usize].next_sibling = node;
        }
        self.nodes[p].last_child = node;
        self.nodes[node as usize].parent = parent;
        self.taffy.add_child(self.tnodes[p], self.tnodes[node as usize]).expect("taffy child");
        Ok(node)
    }

    /// Removes a node and everything under it. The root cannot be removed.
    pub fn remove(&mut self, node: u32) -> Result<(), String> {
        let n = self.live(node)?;
        if node == 0 {
            return Err("the root cannot be removed".to_string());
        }
        let parent = self.nodes[n].parent;
        let p = parent as usize;
        let mut prev = NONE;
        let mut cur = self.nodes[p].first_child;
        while cur != node {
            prev = cur;
            cur = self.nodes[cur as usize].next_sibling;
        }
        let next = self.nodes[n].next_sibling;
        if prev == NONE {
            self.nodes[p].first_child = next;
        } else {
            self.nodes[prev as usize].next_sibling = next;
        }
        if self.nodes[p].last_child == node {
            self.nodes[p].last_child = prev;
        }
        let _ = self.taffy.remove_child(self.tnodes[p], self.tnodes[n]);
        let mut stack = vec![node];
        while let Some(i) = stack.pop() {
            let mut child = self.nodes[i as usize].first_child;
            while child != NONE {
                stack.push(child);
                child = self.nodes[child as usize].next_sibling;
            }
            let v = self.visible[i as usize];
            if !v.is_empty() {
                self.damage.push(v);
            }
            let _ = self.taffy.remove(self.tnodes[i as usize]);
            self.nodes[i as usize] = UiNode::FREE;
            self.state[i as usize] = 0;
            self.edits.remove(&i);
            self.history.remove(&i);
            self.texts[i as usize] = String::new();
            self.runs.remove(&i);
            self.free.push(i);
            self.removed.push(i);
            self.hover.retain(|&h| h != i);
            self.pressed.retain(|&p| p != i);
            if self.focus == i {
                self.focus = NONE;
            }
        }
        self.order_dirty = true;
        Ok(())
    }

    /// Moves a node to sit just before its sibling `before`, or last when `before` is `None`.
    pub fn move_before(&mut self, node: u32, before: Option<u32>) -> Result<(), String> {
        let n = self.live(node)?;
        if node == 0 {
            return Err("the root cannot be moved".to_string());
        }
        let parent = self.nodes[n].parent;
        let p = parent as usize;
        if let Some(b) = before {
            self.live(b)?;
            if b == node {
                return Ok(());
            }
            if self.nodes[b as usize].parent != parent {
                return Err("a node moves among its own siblings".to_string());
            }
        }
        let mut kids = self.children(parent);
        kids.retain(|&k| k != node);
        let at = match before {
            Some(b) => kids.iter().position(|&k| k == b).expect("a sibling"),
            None => kids.len(),
        };
        kids.insert(at, node);
        if self.children(parent) == kids {
            return Ok(());
        }
        for (i, &k) in kids.iter().enumerate() {
            self.nodes[k as usize].next_sibling = kids.get(i + 1).copied().unwrap_or(NONE);
        }
        self.nodes[p].first_child = kids[0];
        self.nodes[p].last_child = kids[kids.len() - 1];
        let _ = self.taffy.remove_child(self.tnodes[p], self.tnodes[n]);
        self.taffy.insert_child_at_index(self.tnodes[p], at, self.tnodes[n]).expect("taffy child");
        for &k in &kids {
            self.paint_dirty[k as usize] = true;
        }
        self.order_dirty = true;
        Ok(())
    }

    /// Sets a node's text, which drops its runs.
    pub fn set_text(&mut self, node: u32, text: &str) -> Result<(), String> {
        if self.put_text(node, text)? {
            self.forget_history(node);
        }
        Ok(())
    }

    /// Stores a node's text and drops its runs; true if the text changed.
    fn put_text(&mut self, node: u32, text: &str) -> Result<bool, String> {
        let n = self.live(node)?;
        let had_runs = self.runs.remove(&node).is_some();
        let changed = self.texts[n] != text;
        if changed {
            self.texts[n].clear();
            self.texts[n].push_str(text);
        }
        if changed || had_runs {
            self.paint_dirty[n] = true;
            let _ = self.taffy.mark_dirty(self.tnodes[n]);
        }
        Ok(changed)
    }

    /// Sets a text or button node's text to `text` split into `runs`, which must cover it exactly on character boundaries.
    pub fn set_runs(&mut self, node: u32, text: &str, runs: Vec<Run>) -> Result<(), String> {
        let n = self.live(node)?;
        if !matches!(self.nodes[n].kind, Kind::Text | Kind::Button) {
            return Err("only a text or button node holds runs".to_string());
        }
        let mut at = 0;
        for run in &runs {
            at += run.len;
            if !text.is_char_boundary(at.min(text.len())) || at > text.len() {
                return Err("the runs do not split the text at characters".to_string());
            }
        }
        if at != text.len() {
            return Err("the runs do not cover the text".to_string());
        }
        self.set_text(node, text)?;
        if !runs.is_empty() {
            self.runs.insert(node, runs);
        }
        self.paint_dirty[n] = true;
        let _ = self.taffy.mark_dirty(self.tnodes[n]);
        Ok(())
    }

    /// A node's runs; none for plain text.
    pub fn runs(&self, node: u32) -> &[Run] {
        self.runs.get(&node).map_or(&[], Vec::as_slice)
    }

    pub fn text(&self, node: u32) -> Result<&str, String> {
        let n = self.live(node)?;
        Ok(self.text_of(n))
    }

    fn text_of(&self, n: usize) -> &str {
        &self.texts[n]
    }

    /// The id of a style equal to `def`, adding it if it is new.
    pub fn intern_style(&mut self, def: StyleDef) -> u32 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{def:?}").hash(&mut hasher);
        let key = hasher.finish();
        if let Some(ids) = self.style_lookup.get(&key) {
            for &id in ids {
                if self.styles[id as usize] == def {
                    return id;
                }
            }
        }
        let id = self.styles.len() as u32;
        self.styles.push(def);
        self.style_lookup.entry(key).or_default().push(id);
        id
    }

    pub fn style_count(&self) -> usize {
        self.styles.len()
    }

    pub fn set_style(&mut self, node: u32, style_id: u32) -> Result<(), String> {
        let n = self.live(node)?;
        if style_id != NO_STYLE && style_id as usize >= self.styles.len() {
            return Err(format!("no style {style_id}"));
        }
        if self.nodes[n].style_id != style_id {
            self.nodes[n].style_id = style_id;
            self.paint_dirty[n] = true;
            self.sync_style(node);
        }
        Ok(())
    }

    /// The node's style; a node with none has the default.
    pub fn style(&self, node: u32) -> &StyleDef {
        match self.nodes.get(node as usize).map(|n| n.style_id) {
            Some(id) if id != NO_STYLE => &self.styles[id as usize],
            _ => &self.default_style,
        }
    }

    /// The style to paint: the node's own, with the paint fields of its pressed or hover style while it is held or under the pointer.
    pub fn painted_style(&self, node: u32) -> StyleDef {
        let base = self.style(node);
        let state = self.state.get(node as usize).copied().unwrap_or(0);
        let over = if state & (interact::PRESSED | interact::HOVER) == interact::PRESSED | interact::HOVER && base.pressed != NO_STYLE {
            base.pressed
        } else if state & interact::HOVER != 0 && base.hover != NO_STYLE {
            base.hover
        } else {
            return base.clone();
        };
        let mut style = base.clone();
        if let Some(o) = self.style_by_id(over) {
            style.background = o.background;
            style.border_color = o.border_color;
            style.radius = o.radius;
            style.opacity = o.opacity;
            style.color = o.color;
        }
        style
    }

    pub fn style_by_id(&self, id: u32) -> Option<&StyleDef> {
        self.styles.get(id as usize)
    }

    fn taffy_style(&self, node: u32) -> taffy::Style {
        let forced = (node == 0).then_some(self.viewport);
        let mut t = to_taffy(self.style(node), forced);
        if self.nodes[node as usize].kind == Kind::Scroll {
            t.overflow = taffy::Point { x: taffy::Overflow::Hidden, y: taffy::Overflow::Hidden };
        }
        t
    }

    fn sync_style(&mut self, node: u32) {
        let style = self.taffy_style(node);
        let _ = self.taffy.set_style(self.tnodes[node as usize], style);
    }

    /// Records the pixel size an image node is laid out at when its style gives none.
    pub fn set_image_size(&mut self, node: u32, width: f32, height: f32) -> Result<(), String> {
        let n = self.live(node)?;
        self.image_sizes[n] = [width, height];
        self.paint_dirty[n] = true;
        let _ = self.taffy.mark_dirty(self.tnodes[n]);
        Ok(())
    }

    pub fn kind(&self, node: u32) -> Option<Kind> {
        self.nodes.get(node as usize).map(|n| n.kind).filter(|k| *k != Kind::Free)
    }

    pub fn parent(&self, node: u32) -> Option<u32> {
        self.nodes.get(node as usize).map(|n| n.parent).filter(|p| *p != NONE)
    }

    /// The children of a node, in order.
    pub fn children(&self, node: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let Some(n) = self.nodes.get(node as usize) else { return out };
        let mut cur = n.first_child;
        while cur != NONE {
            out.push(cur);
            cur = self.nodes[cur as usize].next_sibling;
        }
        out
    }

    /// Live nodes in paint order: parents before children, earlier siblings before later ones.
    pub fn paint_order(&self) -> &[u32] {
        &self.order
    }

    /// The node's rectangle in window pixels, with every scroll offset applied.
    pub fn rect(&self, node: u32) -> Rect {
        self.rects.get(node as usize).copied().unwrap_or(Rect::ZERO)
    }

    /// The part of the node's rectangle that scroll and overflow clipping leave visible.
    pub fn visible(&self, node: u32) -> Rect {
        self.visible.get(node as usize).copied().unwrap_or(Rect::ZERO)
    }

    /// The region clipping the node's own painting.
    pub fn clip(&self, node: u32) -> Rect {
        self.clips.get(node as usize).copied().unwrap_or(Rect::ZERO)
    }

    pub fn scroll_offset(&self, node: u32) -> [f32; 2] {
        self.scroll.get(node as usize).copied().unwrap_or([0.0; 2])
    }

    /// The size of a node's children taken together, insets included.
    pub fn content_size(&self, node: u32) -> [f32; 2] {
        self.content.get(node as usize).copied().unwrap_or([0.0; 2])
    }

    /// Lays out every node that changed, then updates rectangles and damage.
    pub fn layout(&mut self, measure: &mut dyn Measure) {
        let space = Size { width: AvailableSpace::Definite(self.viewport.w), height: AvailableSpace::Definite(self.viewport.h) };
        let Tree { taffy, nodes, texts, styles, default_style, image_sizes, runs, .. } = self;
        let style_of = |n: usize| match nodes[n].style_id {
            NO_STYLE => &*default_style,
            id => &styles[id as usize],
        };
        let _ = taffy.compute_layout_with_measure(self.tnodes[0], space, |inputs, _, ctx, style| {
            taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |known, avail| {
                let Some(&mut idx) = ctx else { return Size::ZERO };
                let n = idx as usize;
                let kind = nodes[n].kind;
                if !kind.has_content() {
                    return Size::ZERO;
                }
                let max_width = known.width.or(match avail.width {
                    AvailableSpace::Definite(w) => Some(w),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                });
                let [w, h] = if kind == Kind::Image {
                    image_sizes[n]
                } else {
                    let text = texts[n].as_str();
                    let runs = runs.get(&idx).map_or(&[][..], Vec::as_slice);
                    measure.measure(&MeasureNode { kind, text, runs, style: style_of(n) }, max_width)
                };
                Size { width: known.width.unwrap_or(w), height: known.height.unwrap_or(h) }
            })
        });
        self.refresh();
    }

    fn rebuild_order(&mut self) {
        self.order.clear();
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            self.order.push(i);
            let mut kids = self.children(i);
            kids.reverse();
            stack.extend(kids);
        }
        self.order_dirty = false;
    }

    fn scrolls(&self, n: usize) -> [bool; 2] {
        let style = self.style(n as u32);
        let by_kind = self.nodes[n].kind == Kind::Scroll;
        [style.overflow[0] == Overflow::Scroll, by_kind || style.overflow[1] == Overflow::Scroll]
    }

    /// Whether the node scrolls vertically.
    pub fn is_scrollable(&self, node: u32) -> bool {
        self.live(node).is_ok_and(|n| self.scrolls(n)[1])
    }

    fn clips_children(&self, n: usize) -> bool {
        self.nodes[n].kind == Kind::Scroll || self.style(n as u32).clips()
    }

    /// Recomputes rectangles, clips and visible regions from taffy's layout and the scroll offsets, and records what changed as damage.
    fn refresh(&mut self) {
        if self.order_dirty {
            self.rebuild_order();
        }
        let old_visible = self.visible.clone();
        let mut stack: Vec<(u32, f32, f32, Rect)> = vec![(0, 0.0, 0.0, self.viewport)];
        while let Some((i, ox, oy, clip)) = stack.pop() {
            let n = i as usize;
            let layout = *self.taffy.layout(self.tnodes[n]).expect("taffy layout");
            let rect = Rect::new(ox + layout.location.x, oy + layout.location.y, layout.size.width, layout.size.height);
            self.rects[n] = rect;
            self.clips[n] = clip;
            self.visible[n] = rect.intersect(&clip);
            let kids = self.children(i);
            let insets = self.style(i).insets(rect.w);
            let (mut right, mut bottom) = (0.0f32, 0.0f32);
            for &k in &kids {
                let l = self.taffy.layout(self.tnodes[k as usize]).expect("taffy layout");
                right = right.max(l.location.x + l.size.width);
                bottom = bottom.max(l.location.y + l.size.height);
            }
            let content = [(right + insets[2]).max(rect.w), (bottom + insets[3]).max(rect.h)];
            self.content[n] = content;
            let axes = self.scrolls(n);
            let max = [(content[0] - rect.w).max(0.0), (content[1] - rect.h).max(0.0)];
            let offset = [if axes[0] { self.scroll[n][0].clamp(0.0, max[0]) } else { 0.0 }, if axes[1] { self.scroll[n][1].clamp(0.0, max[1]) } else { 0.0 }];
            self.scroll[n] = offset;
            let child_clip = if self.clips_children(n) { clip.intersect(&rect) } else { clip };
            for &k in kids.iter().rev() {
                stack.push((k, rect.x - offset[0], rect.y - offset[1], child_clip));
            }
        }
        for &i in &self.order {
            let n = i as usize;
            let (old, new) = (old_visible.get(n).copied().unwrap_or(Rect::ZERO), self.visible[n]);
            if old != new || self.paint_dirty[n] {
                for r in [old, new] {
                    if !r.is_empty() {
                        self.damage.push(r);
                    }
                }
            }
            self.paint_dirty[n] = false;
        }
    }

    /// Marks a node for repainting at the next layout, for a change that moves nothing (a colour, a hover).
    pub fn damage_node(&mut self, node: u32) {
        if let Some(d) = self.paint_dirty.get_mut(node as usize) {
            *d = true;
        }
    }

    /// Scrolls a node by `dx`, `dy` pixels within its content; true if it moved.
    pub fn scroll_by(&mut self, node: u32, dx: f32, dy: f32) -> bool {
        let Ok(n) = self.live(node) else { return false };
        let axes = self.scrolls(n);
        let rect = self.rects[n];
        let max = [(self.content[n][0] - rect.w).max(0.0), (self.content[n][1] - rect.h).max(0.0)];
        let before = self.scroll[n];
        let next = [if axes[0] { (before[0] + dx).clamp(0.0, max[0]) } else { 0.0 }, if axes[1] { (before[1] + dy).clamp(0.0, max[1]) } else { 0.0 }];
        if next == before {
            return false;
        }
        self.scroll[n] = next;
        self.refresh();
        true
    }

    /// The node under the point: the last one in paint order whose visible region holds it.
    pub fn hit(&self, x: f32, y: f32) -> Option<u32> {
        self.order.iter().rev().copied().find(|&i| self.visible[i as usize].contains(x, y))
    }

    /// Scrolls the nearest scrollable ancestor of the node under the point that can move; returns it.
    pub fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> Option<u32> {
        let mut cur = self.hit(x, y)?;
        loop {
            if self.scroll_by(cur, dx, dy) {
                return Some(cur);
            }
            cur = self.parent(cur)?;
        }
    }

    /// The regions to repaint since the last call, overlapping ones merged.
    pub fn take_damage(&mut self) -> Vec<Rect> {
        let view = self.viewport;
        let mut rects: Vec<Rect> = self.damage.drain(..).map(|r| r.intersect(&view)).filter(|r| !r.is_empty()).collect();
        loop {
            let mut merged = false;
            'outer: for a in 0..rects.len() {
                for b in a + 1..rects.len() {
                    if rects[a].touches(&rects[b]) {
                        rects[a] = rects[a].union(&rects[b]);
                        rects.swap_remove(b);
                        merged = true;
                        break 'outer;
                    }
                }
            }
            if !merged {
                break;
            }
        }
        if rects.len() > 8 {
            let all = rects.iter().fold(Rect::ZERO, |acc, r| acc.union(r));
            return vec![all];
        }
        rects
    }

    /// The ids removed since the last call, so per-node resources can be dropped.
    pub fn take_removed(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.removed)
    }

    pub fn live_nodes(&self) -> usize {
        self.nodes.len() - self.free.len()
    }
}
