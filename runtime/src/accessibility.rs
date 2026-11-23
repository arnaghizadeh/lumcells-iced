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
use crate::core::widget::operation::accessible::{self, Accessible, Toggled};
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
    /// A bar that shows how far something is.
    ProgressBar,
}

/// A widget, as assistive technology is told about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
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
            role,
            label: None,
            placeholder: None,
            value: None,
            toggled: None,
            range: None,
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

    /// Builds the [`Tree`] of the given [`UserInterface`], under a window
    /// with the given title and bounds.
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
            parents: vec![Self::ROOT],
            entered_next: None,
            focus: None,
            offset: Vector::ZERO,
        };

        ui.operate(renderer, &mut collector);

        collector.finish()
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
    fn push(&mut self, mut node: Node) -> usize {
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

        let nodes = order
            .into_iter()
            .map(|index| {
                let mut node = self.nodes[index].clone();

                for child in &mut node.children {
                    *child = kept[*child];
                }

                node
            })
            .collect();

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
                self.offset = self.offset + scrolled;
                operate(self);
                self.offset = self.offset - scrolled;
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
        _id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.focus = None;

        let mut node = Node::new(Role::ScrollArea, bounds);
        node.scroll = Some(Scroll {
            offset: translation,
            content: content_bounds.size(),
        });

        let index = self.push(node);

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
        _id: Option<&Id>,
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

        let _ = self.push(node);
    }

    fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
        // The label of a widget that described itself at the same place
        // (a check box, a switch) is not told a second time: the widget
        // is called by it, or by the name it was given instead
        let parent = *self.parents.last().unwrap_or(&Tree::ROOT);

        if let Some(last) = self.nodes[parent].children.last().copied()
            && !matches!(
                self.nodes[last].role,
                Role::Text | Role::ScrollArea | Role::Button
            )
            && self.nodes[last].layout == bounds
            && self.nodes[last].label.is_some()
        {
            return;
        }

        self.focus = None;

        let mut node = Node::new(Role::Text, bounds);
        node.label = Some(text.to_owned());

        let _ = self.push(node);
    }

    fn accessible(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        accessible: Accessible<'_>,
    ) {
        let role = match accessible.role {
            accessible::Role::Button => Role::Button,
            accessible::Role::CheckBox => Role::CheckBox,
            accessible::Role::Switch => Role::Switch,
            accessible::Role::RadioButton => Role::RadioButton,
            accessible::Role::ComboBox => Role::ComboBox,
            accessible::Role::ProgressBar => Role::ProgressBar,
            accessible::Role::TextInput => Role::TextInput,
            accessible::Role::SecureTextInput => Role::SecureTextInput,
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
        node.is_enabled = accessible.is_enabled;

        let index = self.push(node);

        // Only a button has children that are widgets
        if role == Role::Button {
            self.entered_next = Some(index);
        }
    }
}

fn is_field(role: Role) -> bool {
    matches!(role, Role::TextInput | Role::SecureTextInput)
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

    let Some(node) = now.nodes.get(index) else {
        return false;
    };

    if node.role != asked.role || node.layout != asked.layout {
        return false;
    }

    let Some(seen) = now.visible_bounds(index) else {
        return false;
    };

    let _ = ui.update(
        &[
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
