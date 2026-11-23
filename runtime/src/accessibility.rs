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
use crate::UserInterface;
use crate::core;
use crate::core::Rectangle;
use crate::core::widget::Id;
use crate::core::widget::operation::accessible::{self, Accessible, Toggled};
use crate::core::widget::operation::{Focusable, Operation, TextInput};

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
    /// Where it is in the window, in logical pixels.
    pub bounds: Rectangle,
    /// The indices in [`Tree::nodes`] of what is inside it.
    pub children: Vec<usize>,
}

impl Node {
    fn new(role: Role, bounds: Rectangle) -> Self {
        Self {
            role,
            label: None,
            value: None,
            toggled: None,
            range: None,
            is_enabled: true,
            is_focusable: false,
            is_focused: false,
            bounds,
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
        };

        ui.operate(renderer, &mut collector);

        collector.finish()
    }

    /// The index of the node that has the keyboard focus, if any.
    pub fn focused(&self) -> Option<usize> {
        self.nodes.iter().position(|node| node.is_focused)
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
}

impl Collector {
    fn push(&mut self, mut node: Node) -> usize {
        if let Some((bounds, is_focused)) = self.focus.take()
            && bounds == node.bounds
        {
            node.is_focusable = true;
            node.is_focused = is_focused;
        }

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
                self.parents.push(parent);
                operate(self);
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
            && self.nodes[index].bounds != bounds
        {
            self.entered_next = None;
        }
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
            && self.nodes[last].role == Role::TextInput
            && self.nodes[last].bounds == bounds
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
            && self.nodes[last].role == Role::TextInput
            && self.nodes[last].bounds == bounds
        {
            return;
        }

        let mut node = Node::new(Role::TextInput, bounds);
        node.value = Some(state.text().to_owned());

        let _ = self.push(node);
    }

    fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
        // The label of a widget that described itself with it (a check
        // box) is not told a second time
        let parent = *self.parents.last().unwrap_or(&Tree::ROOT);

        if let Some(last) = self.nodes[parent].children.last().copied()
            && self.nodes[last].role != Role::Text
            && self.nodes[last].bounds == bounds
            && self.nodes[last].label.as_deref() == Some(text)
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
            // A role of a later version: told as nothing in particular
            _ => return,
        };

        let mut node = Node::new(role, bounds);
        node.label = accessible.label.map(str::to_owned);
        node.value = accessible.value.map(str::to_owned);
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
