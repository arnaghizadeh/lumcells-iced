//! Build the tree that assistive technology walks from the widgets of a
//! user interface.
//!
//! The [`Tree`] here knows nothing of a platform: a shell turns it into what
//! its accessibility toolkit wants (`iced_winit` into an AccessKit tree),
//! and a test can read it as it is.
//!
//! It is made by one widget [`Operation`]: what widgets tell about their
//! text, their text inputs, their focus, and what they are
//! ([`Operation::accessible`]) is put together with their bounds. Nothing
//! is kept between two trees.
//!
//! What can be scrolled is a node of its own ([`Role::ScrollArea`]), and
//! what is inside it is told where it is drawn, not where its layout put
//! it before any scrolling: the [`Tree`] follows scrolling. What is
//! scrolled out of view is still in the tree ([`Tree::visible_bounds`]
//! tells that it cannot be seen) and [`reveal`] scrolls it into view.
use crate::UserInterface;
use crate::core;
use crate::core::widget::Id;
use crate::core::widget::operation::accessible::{
    self, Accessible, Live, Selection, Toggled,
};
use crate::core::widget::operation::scrollable::AbsoluteOffset;
use crate::core::widget::operation::{
    Focusable, Operation, Scrollable, TextInput,
};
use crate::core::{Rectangle, Size, Vector};

/// What a [`Node`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// The window: the root of the [`Tree`].
    Window,
    /// A piece of text.
    Text,
    /// A field text is typed in.
    TextInput,
    /// A field a secret is typed in: its value is never told.
    SecureTextInput,
    /// Something that scrolls what is inside it.
    ScrollArea,
    /// Something that is pressed.
    Button,
    /// A box that is ticked or not (or in part).
    CheckBox,
    /// A switch that is on or off.
    Switch,
    /// One choice among a few.
    RadioButton,
    /// A list that opens to choose one of its options.
    ComboBox,
    /// A field text is typed in, with a list of options that follows what
    /// is typed.
    EditableComboBox,
    /// A bar that shows how far something is.
    ProgressBar,
    /// A handle on a rail that chooses a number between two ends.
    Slider,
    /// A field text is typed in over several lines.
    TextEditor,
    /// The open list of a [`Role::ComboBox`], or a menu.
    Menu,
    /// One option of a [`Role::Menu`].
    MenuItem,
}

/// A widget, as assistive technology is told about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// What assistive technology knows it by, from one [`Tree`] to the
    /// next: see [`Tree::of`] for what it is made of. Zero until the
    /// [`Tree`] is finished.
    pub id: u64,
    /// What it is.
    pub role: Role,
    /// Its name; for a [`Role::Text`], the text itself.
    pub label: Option<String>,
    /// What it shows while it is empty (a field) or nothing is chosen (a
    /// list). A widget that was given no name has this as its
    /// [`label`](Self::label) too.
    pub placeholder: Option<String>,
    /// Its value as text: what is typed in a field, the chosen option.
    pub value: Option<String>,
    /// Whether it is ticked, for what can be.
    pub toggled: Option<Toggled>,
    /// Its value as a number, with the least and the most it can be.
    pub range: Option<(f32, f32, f32)>,
    /// How far one step moves that number.
    pub step: Option<f32>,
    /// The caret and the selection in its text, in bytes: in
    /// [`value`](Self::value), or in [`label`](Self::label) for a
    /// [`Role::Text`].
    pub selection: Option<Selection>,
    /// Whether what it opens (its list, its menu) is open.
    pub expanded: Option<bool>,
    /// Whether it is the chosen one among others.
    pub selected: Option<bool>,
    /// Whether, and how, a change of its text is said without being asked
    /// for: a status line.
    pub live: Option<Live>,
    /// Whether it can be used.
    pub is_enabled: bool,
    /// Whether it takes part in keyboard focus.
    pub is_focusable: bool,
    /// Whether it has the keyboard focus.
    pub is_focused: bool,
    /// Where it is drawn in the window, in logical pixels: after the
    /// scrolling of everything it is inside of. It may lie outside of what
    /// can be seen; see [`Tree::visible_bounds`].
    pub bounds: Rectangle,
    /// Where its layout puts it, before any scrolling: the bounds widget
    /// operations are told.
    pub layout: Rectangle,
    /// For a [`Role::ScrollArea`]: how far it is scrolled.
    pub scroll: Option<Scroll>,
    /// The indices in [`Tree::nodes`] of what is inside it.
    pub children: Vec<usize>,
}

/// How far a [`Role::ScrollArea`] is scrolled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scroll {
    /// By how much what is inside is moved up and to the left.
    pub offset: Vector,
    /// The size of everything that is inside.
    pub content: Size,
}

impl Node {
    /// Creates a [`Node`] that is drawn where its layout puts it.
    pub fn new(role: Role, bounds: Rectangle) -> Self {
        Self {
            id: 0,
            role,
            label: None,
            placeholder: None,
            value: None,
            toggled: None,
            range: None,
            step: None,
            selection: None,
            expanded: None,
            selected: None,
            live: None,
            is_enabled: true,
            is_focusable: false,
            is_focused: false,
            bounds,
            layout: bounds,
            scroll: None,
            children: Vec::new(),
        }
    }
}

/// The widgets of a window, as assistive technology is told about them.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    /// The nodes; the first one is the window.
    pub nodes: Vec<Node>,
}

impl Tree {
    /// The index of the root: the window.
    pub const ROOT: usize = 0;

    /// The [`Node::id`] of the window.
    pub const ROOT_ID: u64 = 1;

    /// Builds the [`Tree`] of the given [`UserInterface`], under a window
    /// with the given title and bounds.
    ///
    /// Each [`Node`] has an [`id`](Node::id) that stays the same from one
    /// [`Tree`] to the next, so that a screen reader keeps its place when
    /// the interface is laid out again, scrolled, or changes around it:
    ///
    /// - a widget that was given an [`Id`] (`.id("save")`) is known by it,
    ///   wherever it goes (the first one, if several have the same);
    /// - any other is known by what it is inside of, what it is, what it
    ///   is called, and which one of those it is: the second button
    ///   "Remove" of the list. A piece of text is known by its words; a
    ///   field or a list by its name, not by what is typed or chosen;
    /// - a status line ([`Node::live`]) is known by being the first (the
    ///   second, ...) status line of what it is inside of, since its words
    ///   are what changes.
    ///
    /// So a button that changes its name is another button to a screen
    /// reader, and one that only moves is the same; give a widget an
    /// [`Id`] where that is not what is meant.
    pub fn of<Message, Theme, Renderer>(
        ui: &mut UserInterface<'_, Message, Theme, Renderer>,
        renderer: &Renderer,
        title: &str,
        bounds: Rectangle,
    ) -> Self
    where
        Renderer: core::Renderer,
    {
        let mut window = Node::new(Role::Window, bounds);
        window.label = Some(title.to_owned());

        let mut collector = Collector {
            nodes: vec![window],
            ids: vec![None],
            parents: vec![Self::ROOT],
            entered_next: None,
            focus: None,
            offset: Vector::ZERO,
        };

        ui.operate(renderer, &mut collector);

        collector.finish()
    }

    /// Makes a [`Tree`] of nodes put together by hand (the first one is
    /// the window, [`Node::children`] are indices, a parent stands before
    /// what is inside it) and gives each its [`Node::id`].
    pub fn from_nodes(mut nodes: Vec<Node>) -> Self {
        let ids = vec![None; nodes.len()];
        identify(&mut nodes, &ids);

        Self { nodes }
    }

    /// The index of the node with the given [`Node::id`], if it is in the
    /// [`Tree`].
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
    }

    /// The index of the node that has the keyboard focus, if any.
    pub fn focused(&self) -> Option<usize> {
        self.nodes.iter().position(|node| node.is_focused)
    }

    /// The index of the node that the node with the given index is inside
    /// of, if it is not the window.
    pub fn parent(&self, index: usize) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.children.contains(&index))
    }

    /// The indices of the [`Role::ScrollArea`]s around the node with the
    /// given index, innermost first.
    pub fn scroll_areas(&self, index: usize) -> Vec<usize> {
        let mut areas = Vec::new();
        let mut current = index;

        while let Some(parent) = self.parent(current) {
            if self.nodes[parent].role == Role::ScrollArea {
                areas.push(parent);
            }

            current = parent;
        }

        areas
    }

    /// The part of the node with the given index that can be seen: its
    /// bounds without what the scroll areas around it cut away. `None`
    /// when it is scrolled out of view entirely (or is not in the tree).
    pub fn visible_bounds(&self, index: usize) -> Option<Rectangle> {
        let mut bounds = self.nodes.get(index)?.bounds;

        for area in self.scroll_areas(index) {
            bounds = bounds.intersection(&self.nodes[area].bounds)?;
        }

        (bounds.width > 0.0 && bounds.height > 0.0).then_some(bounds)
    }

    /// The nodes with the given [`Role`], in the order of the widgets.
    pub fn with_role(&self, role: Role) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(move |node| node.role == role)
    }

    /// A line per node, indented by depth: for tests and for looking at.
    pub fn describe(&self) -> String {
        fn line(tree: &Tree, index: usize, depth: usize, out: &mut String) {
            let node = &tree.nodes[index];

            out.push_str(&"  ".repeat(depth));
            out.push_str(&format!("{:?}", node.role));

            if let Some(label) = &node.label {
                out.push_str(&format!(" {label:?}"));
            }

            if let Some(value) = &node.value {
                out.push_str(&format!(" value={value:?}"));
            }

            if let Some(scroll) = &node.scroll {
                out.push_str(&format!(
                    " scrolled={},{}",
                    scroll.offset.x, scroll.offset.y
                ));
            }

            if node.role != Role::Window && tree.visible_bounds(index).is_none()
            {
                out.push_str(" out-of-view");
            }

            if let Some(toggled) = node.toggled {
                out.push_str(&format!(" toggled={toggled:?}"));
            }

            if let Some((value, min, max)) = node.range {
                out.push_str(&format!(" {value} in {min}..={max}"));
            }

            if let Some(selection) = node.selection {
                if selection.anchor == selection.focus {
                    out.push_str(&format!(" caret={}", selection.focus));
                } else {
                    out.push_str(&format!(
                        " selected={}..{}",
                        selection.anchor, selection.focus
                    ));
                }
            }

            match node.expanded {
                Some(true) => out.push_str(" open"),
                Some(false) => out.push_str(" closed"),
                None => {}
            }

            if node.selected == Some(true) {
                out.push_str(" chosen");
            }

            if let Some(live) = node.live {
                out.push_str(&format!(" live={live:?}"));
            }

            if !node.is_enabled {
                out.push_str(" disabled");
            }

            if node.is_focused {
                out.push_str(" focused");
            }

            out.push('\n');

            for child in &node.children {
                line(tree, *child, depth + 1, out);
            }
        }

        let mut out = String::new();
        line(self, Self::ROOT, 0, &mut out);
        out
    }
}

struct Collector {
    nodes: Vec<Node>,
    /// The [`Id`] each node was given as a widget, if any.
    ids: Vec<Option<Id>>,
    /// The node the widgets visited now are inside of, innermost last.
    parents: Vec<usize>,
    /// The node that the next `traverse` goes into: a widget that described
    /// itself and has children.
    entered_next: Option<usize>,
    /// The focus state a widget told, for the node it makes next at the
    /// same place.
    focus: Option<(Rectangle, bool)>,
    /// By how much what is visited now is scrolled: the sum of the
    /// scrolling of the scroll areas around it.
    offset: Vector,
}

impl Collector {
    fn push(&mut self, id: Option<&Id>, mut node: Node) -> usize {
        if let Some((bounds, is_focused)) = self.focus.take()
            && bounds == node.layout
        {
            node.is_focusable = true;
            node.is_focused = is_focused;
        }

        // It is drawn where the scroll areas around it moved it
        node.bounds = node.layout - self.offset;

        let index = self.nodes.len();
        let parent = *self.parents.last().unwrap_or(&Tree::ROOT);

        self.nodes.push(node);
        self.ids.push(id.cloned());
        self.nodes[parent].children.push(index);
        self.entered_next = None;

        index
    }

    fn finish(mut self) -> Tree {
        // A button without a name is named after the text inside it, which
        // is then not told a second time
        for index in 0..self.nodes.len() {
            if self.nodes[index].role != Role::Button {
                continue;
            }

            let mut texts = Vec::new();
            let mut pending = self.nodes[index].children.clone();
            pending.reverse();

            let mut only_text = true;

            while let Some(child) = pending.pop() {
                let node = &self.nodes[child];

                if node.role == Role::Text {
                    texts.extend(node.label.clone());
                } else {
                    only_text = false;
                }

                pending.extend(node.children.iter().rev());
            }

            if self.nodes[index].label.is_none() && !texts.is_empty() {
                self.nodes[index].label = Some(texts.join(" "));
            }

            if only_text {
                self.nodes[index].children.clear();
            }
        }

        // What is no longer the child of anything is left out
        let mut kept = vec![usize::MAX; self.nodes.len()];
        let mut order = Vec::new();
        let mut pending = vec![Tree::ROOT];

        while let Some(index) = pending.pop() {
            kept[index] = order.len();
            order.push(index);
            pending.extend(self.nodes[index].children.iter().rev());
        }

        let ids: Vec<_> =
            order.iter().map(|index| self.ids[*index].clone()).collect();

        let mut nodes: Vec<Node> = order
            .into_iter()
            .map(|index| {
                let mut node = self.nodes[index].clone();

                for child in &mut node.children {
                    *child = kept[*child];
                }

                node
            })
            .collect();

        identify(&mut nodes, &ids);

        Tree { nodes }
    }
}

impl Operation for Collector {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        match self.entered_next.take() {
            Some(parent) => {
                let scrolled = self.nodes[parent]
                    .scroll
                    .map(|scroll| scroll.offset)
                    .unwrap_or(Vector::ZERO);

                self.parents.push(parent);
                self.offset += scrolled;
                operate(self);
                self.offset -= scrolled;
                let _ = self.parents.pop();
            }
            None => operate(self),
        }

        self.entered_next = None;
        self.focus = None;
    }

    fn container(&mut self, _id: Option<&Id>, bounds: Rectangle) {
        // The container of a widget that just described itself is that
        // widget; any other one is not told about
        if let Some(index) = self.entered_next
            && self.nodes[index].layout != bounds
        {
            self.entered_next = None;
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        // A scrollable that takes the keyboard focus told so just before
        let focus = self.focus.take().filter(|(at, _)| *at == bounds);
        self.focus = focus;

        let mut node = Node::new(Role::ScrollArea, bounds);
        node.scroll = Some(Scroll {
            offset: translation,
            content: content_bounds.size(),
        });

        let index = self.push(id, node);

        // What it scrolls is visited in the `traverse` that follows
        self.entered_next = Some(index);
    }

    fn focusable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        state: &mut dyn Focusable,
    ) {
        self.entered_next = None;

        // A text input tells its focus after itself
        let parent = *self.parents.last().unwrap_or(&Tree::ROOT);

        if let Some(last) = self.nodes[parent].children.last().copied()
            && is_field(self.nodes[last].role)
            && self.nodes[last].layout == bounds
        {
            self.nodes[last].is_focusable = true;
            self.nodes[last].is_focused = state.is_focused();
        } else {
            self.focus = Some((bounds, state.is_focused()));
        }
    }

    fn text_input(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        state: &mut dyn TextInput,
    ) {
        // A text input that described itself is there already
        let parent = *self.parents.last().unwrap_or(&Tree::ROOT);

        if let Some(last) = self.nodes[parent].children.last().copied()
            && is_field(self.nodes[last].role)
            && self.nodes[last].layout == bounds
        {
            return;
        }

        let mut node = Node::new(Role::TextInput, bounds);
        node.value = Some(state.text().to_owned());

        let _ = self.push(id, node);
    }

    fn text(&mut self, id: Option<&Id>, bounds: Rectangle, text: &str) {
        // The label of a widget that described itself at the same place
        // (a check box, a switch) is not told a second time: the widget
        // is called by it, or by the name it was given instead
        let parent = *self.parents.last().unwrap_or(&Tree::ROOT);

        if let Some(last) = self.nodes[parent].children.last().copied()
            && !matches!(self.nodes[last].role, Role::ScrollArea | Role::Button)
            // A piece of text that described itself (its selection, that
            // it is a status) said these very words
            && (self.nodes[last].role != Role::Text
                || self.nodes[last].label.as_deref() == Some(text))
            && self.nodes[last].layout == bounds
            && self.nodes[last].label.is_some()
        {
            return;
        }

        self.focus = None;

        let mut node = Node::new(Role::Text, bounds);
        node.label = Some(text.to_owned());

        let _ = self.push(id, node);
    }

    fn accessible(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        accessible: Accessible<'_>,
    ) {
        let role = match accessible.role {
            accessible::Role::Button => Role::Button,
            accessible::Role::CheckBox => Role::CheckBox,
            accessible::Role::Switch => Role::Switch,
            accessible::Role::RadioButton => Role::RadioButton,
            accessible::Role::ComboBox => Role::ComboBox,
            accessible::Role::EditableComboBox => Role::EditableComboBox,
            accessible::Role::ProgressBar => Role::ProgressBar,
            accessible::Role::TextInput => Role::TextInput,
            accessible::Role::SecureTextInput => Role::SecureTextInput,
            accessible::Role::Slider => Role::Slider,
            accessible::Role::TextEditor => Role::TextEditor,
            accessible::Role::Text => Role::Text,
            accessible::Role::Menu => Role::Menu,
            accessible::Role::MenuItem => Role::MenuItem,
            // A role of a later version: told as nothing in particular
            _ => return,
        };

        let mut node = Node::new(role, bounds);
        // What was given no name is called by its placeholder
        node.label = accessible
            .label
            .or(accessible.placeholder)
            .map(str::to_owned);
        node.placeholder = accessible.placeholder.map(str::to_owned);
        // What is typed in a secure field is never told, whatever the
        // widget says
        node.value = accessible
            .value
            .filter(|_| role != Role::SecureTextInput)
            .map(str::to_owned);
        node.toggled = accessible.toggled;
        node.range = accessible.range;
        node.step = accessible.step;
        node.selection = accessible
            .selection
            .filter(|_| role != Role::SecureTextInput);
        node.expanded = accessible.expanded;
        node.selected = accessible.selected;
        node.live = accessible.live;
        node.is_enabled = accessible.is_enabled;

        let index = self.push(id, node);

        // Only a button and a menu have children that are widgets
        if matches!(role, Role::Button | Role::Menu) {
            self.entered_next = Some(index);
        }
    }
}

fn is_field(role: Role) -> bool {
    matches!(
        role,
        Role::TextInput
            | Role::SecureTextInput
            | Role::TextEditor
            | Role::EditableComboBox
    )
}

/// Gives every node its [`Node::id`]; see [`Tree::of`]. The nodes are in
/// the order of the tree, a parent before what is inside it.
fn identify(nodes: &mut [Node], ids: &[Option<Id>]) {
    use std::collections::{HashMap, HashSet};
    use std::hash::{Hash, Hasher};

    /// FNV-1a: the same number for the same widget in every run and on
    /// every system, which the hasher of the standard library does not
    /// promise.
    struct Fnv(u64);

    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }

        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }

    fn hash(value: impl Hash) -> u64 {
        let mut hasher = Fnv(0xcbf2_9ce4_8422_2325);
        value.hash(&mut hasher);
        hasher.finish()
    }

    let mut parents = vec![Tree::ROOT; nodes.len()];

    for (index, node) in nodes.iter().enumerate() {
        for child in &node.children {
            parents[*child] = index;
        }
    }

    let mut used = HashSet::new();
    // How many of a kind were seen inside a node
    let mut seen: HashMap<u64, u32> = HashMap::new();

    for index in 0..nodes.len() {
        let mut id = if index == Tree::ROOT {
            Tree::ROOT_ID
        } else if let Some(id) = &ids[index]
            && !used.contains(&hash(("widget", id)))
        {
            hash(("widget", id))
        } else {
            let node = &nodes[index];
            let parent = nodes[parents[index]].id;

            let kind = if node.live.is_some() {
                // The words of a status line are what changes
                hash((parent, node.role, "live"))
            } else {
                hash((parent, node.role, &node.label))
            };

            let nth = seen.entry(kind).or_default();
            *nth += 1;

            hash((kind, *nth))
        };

        // Never the same for two, and never zero
        while id == 0 || !used.insert(id) {
            id = hash((id, "again"));
        }

        nodes[index].id = id;
    }
}

/// How to bring a node of a [`Tree`] into view: by how much each scroll
/// area around it has to be scrolled.
///
/// It is a widget [`Operation`]: run on the user interface the [`Tree`]
/// was built from, it scrolls. The widgets are then drawn elsewhere, so a
/// new [`Tree`] has to be built to learn where.
#[derive(Debug, Clone, PartialEq)]
pub struct Reveal {
    /// The layout bounds of each scroll area and what it is scrolled by.
    steps: Vec<(Rectangle, AbsoluteOffset)>,
}

impl Reveal {
    /// Whether nothing has to be scrolled: the node is in view as much as
    /// it can be.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Plans how to bring the node with the given index into view; see
/// [`Reveal`].
///
/// Each scroll area around the node is scrolled by the least that shows
/// the node whole, or its start when it is larger than the area.
pub fn reveal(tree: &Tree, index: usize) -> Reveal {
    let Some(node) = tree.nodes.get(index) else {
        return Reveal { steps: Vec::new() };
    };

    fn least(start: f32, end: f32, from: f32, to: f32) -> f32 {
        if start < from {
            start - from
        } else if end > to {
            (end - to).min(start - from)
        } else {
            0.0
        }
    }

    let mut bounds = node.bounds;
    let mut steps = Vec::new();

    for area in tree.scroll_areas(index) {
        let area = &tree.nodes[area];
        let view = area.bounds;

        let by = AbsoluteOffset {
            x: least(
                bounds.x,
                bounds.x + bounds.width,
                view.x,
                view.x + view.width,
            ),
            y: least(
                bounds.y,
                bounds.y + bounds.height,
                view.y,
                view.y + view.height,
            ),
        };

        if by.x != 0.0 || by.y != 0.0 {
            steps.push((area.layout, by));
            bounds = bounds - Vector::new(by.x, by.y);
        }

        // The area around this one has to show what this one shows
        bounds = bounds.intersection(&view).unwrap_or(bounds);
    }

    Reveal { steps }
}

impl Operation for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        for (area, by) in &self.steps {
            if *area == bounds {
                state.scroll_by(*by, bounds, content_bounds);
            }
        }
    }
}

/// Brings the node with the given index of `tree` into view in the user
/// interface the [`Tree`] was built from. Returns whether anything was
/// scrolled.
pub fn scroll_into_view<Message, Theme, Renderer>(
    tree: &Tree,
    index: usize,
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &Renderer,
) -> bool
where
    Renderer: core::Renderer,
{
    let mut reveal = reveal(tree, index);

    if reveal.is_empty() {
        return false;
    }

    ui.operate(renderer, &mut reveal);

    true
}

/// Scrolls the [`Role::ScrollArea`] with the given index of `tree` by the
/// given amount, as far as it goes. Returns whether it is a scroll area.
pub fn scroll_by<Message, Theme, Renderer>(
    tree: &Tree,
    index: usize,
    by: Vector,
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &Renderer,
) -> bool
where
    Renderer: core::Renderer,
{
    let Some(area) = tree.nodes.get(index) else {
        return false;
    };

    if area.role != Role::ScrollArea {
        return false;
    }

    ui.operate(
        renderer,
        &mut Reveal {
            steps: vec![(area.layout, AbsoluteOffset { x: by.x, y: by.y })],
        },
    );

    true
}

/// Moves the number of the node with the given index of `tree` (a
/// [`Role::Slider`]) by so many of its steps, up or down: the node is given
/// the keyboard focus and the arrow keys a person would press. The messages
/// of the widget are added to `messages`. Returns whether it is a slider
/// that can be used.
pub fn step<Message, Theme, Renderer>(
    tree: &Tree,
    index: usize,
    steps: i32,
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &mut Renderer,
    clipboard: &mut dyn core::Clipboard,
    messages: &mut Vec<Message>,
) -> bool
where
    Renderer: core::Renderer,
{
    use crate::core::keyboard;
    use crate::core::keyboard::key;

    let Some(node) = tree.nodes.get(index) else {
        return false;
    };

    if node.role != Role::Slider || !node.is_enabled || !node.is_focusable {
        return false;
    }

    focus(tree, index, ui, renderer);

    let named = if steps < 0 {
        key::Named::ArrowDown
    } else {
        key::Named::ArrowUp
    };

    let press = core::Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(named),
        modified_key: keyboard::Key::Named(named),
        physical_key: key::Physical::Unidentified(
            key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        text: None,
        repeat: false,
    });

    for _ in 0..steps.unsigned_abs().min(10_000) {
        let _ = ui.update(
            std::slice::from_ref(&press),
            core::mouse::Cursor::Unavailable,
            renderer,
            clipboard,
            messages,
        );
    }

    true
}

/// How many steps of the node take its number to the given one (never past
/// its ends), for [`step`].
pub fn steps_to(node: &Node, number: f32) -> i32 {
    let (Some((value, min, max)), Some(step)) = (node.range, node.step) else {
        return 0;
    };

    if step <= 0.0 {
        return 0;
    }

    ((number.clamp(min, max) - value) / step).round() as i32
}

/// Takes the number of the node with the given index of `tree` (a
/// [`Role::Slider`]) to the given number, as near as its steps come; see
/// [`step`]. Returns whether it is a slider that can be used.
pub fn set_number<Message, Theme, Renderer>(
    tree: &Tree,
    index: usize,
    number: f32,
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &mut Renderer,
    clipboard: &mut dyn core::Clipboard,
    messages: &mut Vec<Message>,
) -> bool
where
    Renderer: core::Renderer,
{
    let Some(node) = tree.nodes.get(index) else {
        return false;
    };

    let steps = steps_to(node, number);

    step(tree, index, steps, ui, renderer, clipboard, messages)
}

/// Presses the node with the given index of `tree` in the user interface
/// the [`Tree`] was built from, as a click of the pointer in the middle of
/// what can be seen of it would; what is scrolled out of view is scrolled
/// into view first. The messages of the widgets are added to `messages`.
///
/// Returns whether it was pressed: not when the interface changed since
/// the [`Tree`] was built and the node is something else now, and not when
/// nothing of it can be brought into view.
pub fn press<Message, Theme, Renderer>(
    tree: &Tree,
    index: usize,
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &mut Renderer,
    clipboard: &mut dyn core::Clipboard,
    messages: &mut Vec<Message>,
) -> bool
where
    Renderer: core::Renderer,
{
    use crate::core::mouse;

    let Some(asked) = tree.nodes.get(index) else {
        return false;
    };

    let _ = scroll_into_view(tree, index, ui, renderer);

    // Where it is drawn now
    let now = Tree::of(ui, renderer, "", tree.nodes[Tree::ROOT].bounds);

    // The same widget, wherever it is in the tree now
    let Some(index) = now.index_of(asked.id) else {
        return false;
    };

    let node = &now.nodes[index];

    if node.role != asked.role {
        return false;
    }

    let Some(seen) = now.visible_bounds(index) else {
        return false;
    };

    let _ = ui.update(
        &[
            // What follows the pointer (the options of an open list) has
            // to see it arrive first
            core::Event::Mouse(mouse::Event::CursorMoved {
                position: seen.center(),
            }),
            core::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            )),
            core::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            )),
        ],
        mouse::Cursor::Available(seen.center()),
        renderer,
        clipboard,
        messages,
    );

    true
}

/// Gives the keyboard focus to the node with the given index of `tree` in
/// the user interface the [`Tree`] was built from (and takes it from every
/// other widget), and brings it into view.
pub fn focus<Message, Theme, Renderer>(
    tree: &Tree,
    index: usize,
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &Renderer,
) where
    Renderer: core::Renderer,
{
    struct Focus(Rectangle);

    impl Operation for Focus {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn focusable(
            &mut self,
            _id: Option<&Id>,
            bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            if bounds == self.0 {
                state.focus();
            } else {
                state.unfocus();
            }
        }
    }

    let Some(node) = tree.nodes.get(index) else {
        return;
    };

    let _ = scroll_into_view(tree, index, ui, renderer);

    ui.operate(renderer, &mut Focus(node.layout));
}
