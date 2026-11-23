//! Tell assistive technology (a screen reader) what is in a window.
//!
//! Each window has an AccessKit adapter. While something listens (Orca on
//! Linux, VoiceOver on macOS, Narrator or NVDA on Windows), the tree of
//! [`runtime::accessibility`](crate::runtime::accessibility) is built from
//! the widgets after every frame that is drawn and handed to it; while
//! nothing listens, nothing is built.
//!
//! What assistive technology asks for comes back as [`Request`]s: a press
//! of a widget (its default action) and the keyboard focus on a widget.
use crate::runtime::UserInterface;
use crate::runtime::accessibility::Node as TreeNode;
use crate::runtime::accessibility::{Role, Tree};

use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler,
    DeactivationHandler, Node, NodeId, Toggled, TreeId, TreeInfo, TreeUpdate,
};

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What assistive technology asks a window to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Press a widget, as a click of the pointer would; what is scrolled
    /// out of view is brought into view first.
    Press(Target),
    /// Give the keyboard focus to a widget.
    Focus(Target),
    /// Scroll what is around a widget so that it can be seen.
    Reveal(Target),
    /// Set the number of a widget that has one (a slider).
    SetNumber(Target, f64),
    /// Move the number of a widget by so many of its steps.
    Step(Target, i32),
    /// Scroll a scroll area by so many pages, to the right and down.
    ScrollPages(Target, f32, f32),
    /// Scroll a scroll area to an offset.
    ScrollTo(Target, f32, f32),
}

/// The widget a [`Request`] is about: a node of the tree assistive
/// technology was given last.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    tree: Arc<Tree>,
    index: usize,
}

#[derive(Default)]
struct Shared {
    /// What assistive technology asked for and the window did not do yet.
    requests: Vec<ActionRequest>,
    /// The last tree that was handed out: what the identifiers of a
    /// request mean.
    tree: Option<Arc<Tree>>,
    /// Whether a tree is wanted that was not built yet.
    is_tree_wanted: bool,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wakes the event loop; called from the threads of the platform.
type Wake = Arc<dyn Fn() + Send + Sync>;

struct Handler {
    shared: Arc<Mutex<Shared>>,
    wake: Wake,
}

impl ActivationHandler for Handler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // The tree is made of the widgets, on the thread of the window:
        // it is built at the next frame, which is asked for here
        lock(&self.shared).is_tree_wanted = true;
        (self.wake)();

        None
    }
}

impl ActionHandler for Handler {
    fn do_action(&mut self, request: ActionRequest) {
        lock(&self.shared).requests.push(request);
        (self.wake)();
    }
}

impl DeactivationHandler for Handler {
    fn deactivate_accessibility(&mut self) {}
}

/// The accessibility of a window.
pub struct Adapter {
    raw: accesskit_winit::Adapter,
    shared: Arc<Mutex<Shared>>,
}

impl std::fmt::Debug for Adapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adapter").finish_non_exhaustive()
    }
}

impl Adapter {
    /// Creates the [`Adapter`] of a window. The window must not have been
    /// shown yet.
    ///
    /// `wake` has to make the event loop turn (and redraw); it is called
    /// from other threads.
    pub fn new(
        event_loop: &winit::event_loop::ActiveEventLoop,
        window: &winit::window::Window,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let wake: Wake = Arc::new(wake);

        let handler = || Handler {
            shared: shared.clone(),
            wake: wake.clone(),
        };

        let raw = accesskit_winit::Adapter::with_direct_handlers(
            event_loop,
            window,
            handler(),
            handler(),
            handler(),
        );

        Self { raw, shared }
    }

    /// Lets the [`Adapter`] see an event of its window (its focus, its
    /// moves).
    pub fn process_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) {
        self.raw.process_event(window, event);
    }

    /// Whether a first tree was asked for and not given yet: the window
    /// should be drawn (and [`update`](Self::update) called).
    pub fn wants_tree(&self) -> bool {
        lock(&self.shared).is_tree_wanted
    }

    /// Hands the tree that `build` makes to assistive technology, if any
    /// listens; otherwise `build` is not called.
    pub fn update(&mut self, scale_factor: f64, build: impl FnOnce() -> Tree) {
        let shared = &self.shared;

        self.raw.update_if_active(|| {
            let tree = build();

            let mut shared = lock(shared);
            let update = tree_update(&tree, scale_factor);

            shared.tree = Some(Arc::new(tree));
            shared.is_tree_wanted = false;

            update
        });
    }

    /// Takes what assistive technology asked for since the last time.
    pub fn requests(&mut self) -> Vec<Request> {
        let mut shared = lock(&self.shared);
        let requests = std::mem::take(&mut shared.requests);

        let Some(tree) = shared.tree.clone() else {
            return Vec::new();
        };

        requests
            .into_iter()
            .filter_map(|request| {
                // A request for a line of text is for the field it is in
                let index =
                    tree.index_of(request.target_node.0).or_else(|| {
                        tree.nodes.iter().position(|node| {
                            (0..lines(node).count()).any(|line| {
                                run_id(node, line) == request.target_node
                            })
                        })
                    })?;

                let target = Target {
                    tree: tree.clone(),
                    index,
                };

                match (request.action, request.data) {
                    (Action::Click, _) => Some(Request::Press(target)),
                    (Action::Focus, _) => Some(Request::Focus(target)),
                    (Action::ScrollIntoView, _) => {
                        Some(Request::Reveal(target))
                    }
                    (
                        Action::SetValue,
                        Some(accesskit::ActionData::NumericValue(number)),
                    ) => Some(Request::SetNumber(target, number)),
                    (Action::Increment, _) => Some(Request::Step(target, 1)),
                    (Action::Decrement, _) => Some(Request::Step(target, -1)),
                    (Action::ScrollUp, _) => {
                        Some(Request::ScrollPages(target, 0.0, -1.0))
                    }
                    (Action::ScrollDown, _) => {
                        Some(Request::ScrollPages(target, 0.0, 1.0))
                    }
                    (Action::ScrollLeft, _) => {
                        Some(Request::ScrollPages(target, -1.0, 0.0))
                    }
                    (Action::ScrollRight, _) => {
                        Some(Request::ScrollPages(target, 1.0, 0.0))
                    }
                    (
                        Action::SetScrollOffset,
                        Some(accesskit::ActionData::SetScrollOffset(offset)),
                    ) => Some(Request::ScrollTo(
                        target,
                        offset.x as f32,
                        offset.y as f32,
                    )),
                    _ => None,
                }
            })
            .collect()
    }
}

/// Does what assistive technology asked for in the given
/// [`UserInterface`]; the messages of the widgets are added to `messages`.
pub fn perform<Message, Theme, Renderer>(
    request: Request,
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &mut Renderer,
    clipboard: &mut dyn crate::core::Clipboard,
    messages: &mut Vec<Message>,
) where
    Renderer: crate::core::Renderer,
{
    use crate::runtime::accessibility;

    match request {
        Request::Press(Target { tree, index }) => {
            let _ = accessibility::press(
                &tree, index, ui, renderer, clipboard, messages,
            );
        }
        Request::Focus(Target { tree, index }) => {
            accessibility::focus(&tree, index, ui, renderer);
        }
        Request::Reveal(Target { tree, index }) => {
            let _ = accessibility::scroll_into_view(&tree, index, ui, renderer);
        }
        Request::SetNumber(Target { tree, index }, number) => {
            let _ = accessibility::set_number(
                &tree,
                index,
                number as f32,
                ui,
                renderer,
                clipboard,
                messages,
            );
        }
        Request::Step(Target { tree, index }, steps) => {
            let _ = accessibility::step(
                &tree, index, steps, ui, renderer, clipboard, messages,
            );
        }
        Request::ScrollPages(Target { tree, index }, x, y) => {
            if let Some(area) = tree.nodes.get(index) {
                let _ = accessibility::scroll_by(
                    &tree,
                    index,
                    crate::core::Vector::new(
                        x * area.bounds.width,
                        y * area.bounds.height,
                    ),
                    ui,
                    renderer,
                );
            }
        }
        Request::ScrollTo(Target { tree, index }, x, y) => {
            if let Some(scroll) =
                tree.nodes.get(index).and_then(|area| area.scroll)
            {
                let _ = accessibility::scroll_by(
                    &tree,
                    index,
                    crate::core::Vector::new(x, y) - scroll.offset,
                    ui,
                    renderer,
                );
            }
        }
    }
}

/// The identifier of a node is its [`id`](crate::runtime::accessibility::Node::id):
/// the same from one tree to the next.
fn node_id(tree: &Tree, index: usize) -> NodeId {
    NodeId(tree.nodes[index].id)
}

/// Whether the node holds text a caret moves in, which assistive technology
/// is given line by line.
fn has_lines(node: &TreeNode) -> bool {
    match node.role {
        Role::TextInput | Role::TextEditor | Role::EditableComboBox => true,
        // A piece of text with a selection in it
        Role::Text => node.selection.is_some(),
        _ => false,
    }
}

/// The text of a node that has lines: what is typed in a field, the words
/// of a piece of text.
fn content(node: &TreeNode) -> &str {
    match node.role {
        Role::Text => node.label.as_deref(),
        _ => node.value.as_deref(),
    }
    .unwrap_or_default()
}

/// The lines of a node that has them, each with the byte it starts at and
/// with its line feed. A field with nothing in it has one empty line, for
/// the caret to be in.
fn lines(node: &TreeNode) -> impl Iterator<Item = (usize, &str)> {
    let text = if has_lines(node) {
        Some(content(node))
    } else {
        None
    };

    let mut start = 0;

    text.into_iter().flat_map(move |text| {
        let mut lines: Vec<_> = text
            .split_inclusive('\n')
            .map(|line| {
                let at = start;
                start += line.len();
                (at, line)
            })
            .collect();

        // The caret can stand after the last line feed
        if text.is_empty() || text.ends_with('\n') {
            lines.push((text.len(), ""));
        }

        lines
    })
}

/// The identifier of a line of a node.
fn run_id(node: &TreeNode, line: usize) -> NodeId {
    NodeId(
        node.id
            .rotate_left(21)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(line as u64 + 1)
            | 1 << 63,
    )
}

/// Where a byte of the text of a node is, for AccessKit: the line, and the
/// character (as a person counts them) in it.
fn position(node: &TreeNode, byte: usize) -> Option<accesskit::TextPosition> {
    use unicode_segmentation::UnicodeSegmentation;

    let (line, (start, text)) = lines(node)
        .enumerate()
        .take_while(|(_, (start, _))| *start <= byte)
        .last()?;

    let inside = byte.saturating_sub(start).min(text.len());

    Some(accesskit::TextPosition {
        node: run_id(node, line),
        character_index: text
            .grapheme_indices(true)
            .take_while(|(at, _)| *at < inside)
            .count(),
    })
}

/// A line of text as AccessKit wants it: its characters and where its
/// words begin.
fn run(node: &TreeNode, text: &str) -> Node {
    use unicode_segmentation::UnicodeSegmentation;

    let mut raw = Node::new(accesskit::Role::TextRun);

    raw.set_bounds(accesskit::Rect {
        x0: f64::from(node.bounds.x),
        y0: f64::from(node.bounds.y),
        x1: f64::from(node.bounds.x + node.bounds.width),
        y1: f64::from(node.bounds.y + node.bounds.height),
    });
    raw.set_value(text);
    raw.set_text_direction(accesskit::TextDirection::LeftToRight);

    let starts: Vec<usize> =
        text.grapheme_indices(true).map(|(at, _)| at).collect();

    raw.set_character_lengths(
        text.graphemes(true)
            .map(|character| character.len().min(255) as u8)
            .collect::<Vec<_>>(),
    );

    // Where the characters are drawn is not known here: they are told as
    // sharing the width of the line evenly, so that a reader that asks for
    // the place of the caret gets one inside the field
    let width = node.bounds.width / starts.len().max(1) as f32;

    raw.set_character_positions(
        (0..starts.len())
            .map(|i| i as f32 * width)
            .collect::<Vec<_>>(),
    );
    raw.set_character_widths(vec![width; starts.len()]);

    raw.set_word_starts(
        text.unicode_word_indices()
            .filter_map(|(at, _)| {
                // The first word begins the line: that is implied
                (at > 0)
                    .then(|| starts.binary_search(&at).ok())
                    .flatten()
                    .and_then(|character| u8::try_from(character).ok())
            })
            .collect::<Vec<_>>(),
    );

    raw
}

/// Turns a [`Tree`] into what AccessKit wants: every node, every time.
///
/// A node keeps its identifier from one tree to the next (see
/// [`Tree::of`]), so AccessKit tells assistive technology only what
/// changed, and a screen reader stays where it was.
pub fn tree_update(tree: &Tree, scale_factor: f64) -> TreeUpdate {
    // Which nodes are inside of something that scrolls
    let mut is_scrolled = vec![false; tree.nodes.len()];

    for (index, node) in tree.nodes.iter().enumerate() {
        if node.role == Role::ScrollArea || is_scrolled[index] {
            for child in &node.children {
                if let Some(inside) = is_scrolled.get_mut(*child) {
                    *inside = true;
                }
            }
        }
    }

    let mut nodes = Vec::with_capacity(tree.nodes.len());

    for (index, node) in tree.nodes.iter().enumerate() {
        let role = match node.role {
            Role::Window => accesskit::Role::Window,
            Role::Text => accesskit::Role::Label,
            Role::TextInput => accesskit::Role::TextInput,
            Role::SecureTextInput => accesskit::Role::PasswordInput,
            Role::TextEditor => accesskit::Role::MultilineTextInput,
            Role::ScrollArea => accesskit::Role::ScrollView,
            Role::Button => accesskit::Role::Button,
            Role::CheckBox => accesskit::Role::CheckBox,
            Role::Switch => accesskit::Role::Switch,
            Role::RadioButton => accesskit::Role::RadioButton,
            Role::ComboBox => accesskit::Role::ComboBox,
            Role::EditableComboBox => accesskit::Role::EditableComboBox,
            Role::ProgressBar => accesskit::Role::ProgressIndicator,
            Role::Slider => accesskit::Role::Slider,
            Role::Menu => accesskit::Role::ListBox,
            Role::MenuItem => accesskit::Role::ListBoxOption,
            _ => accesskit::Role::GenericContainer,
        };

        let mut raw = Node::new(role);

        raw.set_bounds(accesskit::Rect {
            x0: f64::from(node.bounds.x),
            y0: f64::from(node.bounds.y),
            x1: f64::from(node.bounds.x + node.bounds.width),
            y1: f64::from(node.bounds.y + node.bounds.height),
        });

        // The bounds are in logical pixels; the window scales them
        if index == Tree::ROOT {
            raw.set_transform(accesskit::Affine::scale(scale_factor));
        }

        // A widget that was given a name of its own tells its
        // placeholder beside it
        let is_named = node.label.is_some() && node.label != node.placeholder;

        if is_named && let Some(placeholder) = &node.placeholder {
            raw.set_placeholder(placeholder.as_str());
        }

        match (&node.label, &node.value, node.role) {
            // The text of a piece of text is its value
            (Some(text), _, Role::Text) => raw.set_value(text.as_str()),
            // A list without a name is called by the option that is
            // chosen, and by its placeholder while none is: screen
            // readers do not read the value of a combo box
            (_, Some(chosen), Role::ComboBox) if !is_named => {
                raw.set_label(chosen.as_str());
                raw.set_value(chosen.as_str());
            }
            (label, value, _) => {
                if let Some(label) = label {
                    raw.set_label(label.as_str());
                }

                if let Some(value) = value {
                    raw.set_value(value.as_str());
                }
            }
        }

        if let Some(toggled) = node.toggled {
            use crate::core::widget::operation::accessible::Toggled as T;

            raw.set_toggled(match toggled {
                T::No => Toggled::False,
                T::Yes => Toggled::True,
                T::Mixed => Toggled::Mixed,
            });
        }

        if let Some((value, min, max)) = node.range {
            raw.set_numeric_value(f64::from(value));
            raw.set_min_numeric_value(f64::from(min));
            raw.set_max_numeric_value(f64::from(max));
        }

        if let Some(step) = node.step {
            raw.set_numeric_value_step(f64::from(step));
        }

        // A slider is moved by assistive technology too
        if node.role == Role::Slider && node.is_enabled {
            raw.add_action(Action::Increment);
            raw.add_action(Action::Decrement);
            raw.add_action(Action::SetValue);
        }

        if let Some(is_expanded) = node.expanded {
            raw.set_expanded(is_expanded);
        }

        if let Some(is_selected) = node.selected {
            raw.set_selected(is_selected);
        }

        // A status line is said when its words change
        if let Some(live) = node.live {
            use crate::core::widget::operation::accessible::Live;

            raw.set_live(match live {
                Live::Polite => accesskit::Live::Polite,
                Live::Assertive => accesskit::Live::Assertive,
            });
        }

        // What is inside a scroll area is cut at its edges, and it
        // says how far it is scrolled
        if let Some(scroll) = node.scroll {
            raw.set_clips_children();

            let room =
                |content: f32, view: f32| f64::from((content - view).max(0.0));

            raw.set_scroll_x(f64::from(scroll.offset.x));
            raw.set_scroll_x_min(0.0);
            raw.set_scroll_x_max(room(scroll.content.width, node.bounds.width));
            raw.set_scroll_y(f64::from(scroll.offset.y));
            raw.set_scroll_y_min(0.0);
            raw.set_scroll_y_max(room(
                scroll.content.height,
                node.bounds.height,
            ));

            if scroll.content.height > node.bounds.height {
                raw.add_action(Action::ScrollUp);
                raw.add_action(Action::ScrollDown);
            }

            if scroll.content.width > node.bounds.width {
                raw.add_action(Action::ScrollLeft);
                raw.add_action(Action::ScrollRight);
            }

            raw.add_action(Action::SetScrollOffset);
        }

        if is_scrolled[index] {
            raw.add_action(Action::ScrollIntoView);
        }

        if !node.is_enabled {
            raw.set_disabled();
        }

        if node.is_focusable {
            raw.add_action(Action::Focus);
        }

        if node.is_enabled
            && matches!(
                node.role,
                Role::Button
                    | Role::CheckBox
                    | Role::Switch
                    | Role::RadioButton
                    | Role::ComboBox
                    | Role::MenuItem
            )
        {
            raw.add_action(Action::Click);
        }

        let mut children: Vec<_> = node
            .children
            .iter()
            .map(|child| node_id(tree, *child))
            .collect();

        // The text of a field, line by line, with the caret and the
        // selection in it
        let mut runs = Vec::new();

        for (line, (_, text)) in lines(node).enumerate() {
            let id = run_id(node, line);

            children.push(id);
            runs.push((id, run(node, text)));
        }

        if let Some(selection) = node.selection
            && let Some(anchor) = position(node, selection.anchor)
            && let Some(focus) = position(node, selection.focus)
        {
            raw.set_text_selection(accesskit::TextSelection { anchor, focus });
        }

        raw.set_children(children);

        nodes.push((node_id(tree, index), raw));
        nodes.extend(runs);
    }

    let mut info = TreeInfo::new(node_id(tree, Tree::ROOT));
    info.toolkit_name = Some("iced".to_owned());
    info.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());

    TreeUpdate {
        nodes,
        tree: Some(info),
        tree_id: TreeId::ROOT,
        focus: node_id(tree, tree.focused().unwrap_or(Tree::ROOT)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::widget::operation::accessible::Toggled as T;
    use crate::core::{Point, Rectangle, Size, Vector};
    use crate::runtime::accessibility::Node as TreeNode;
    use crate::runtime::accessibility::Scroll;

    fn node(role: Role, label: Option<&str>, children: &[usize]) -> TreeNode {
        let mut node = TreeNode::new(
            role,
            Rectangle::new(
                Point::new(10.0, 20.0),
                crate::core::Size::new(30.0, 40.0),
            ),
        );

        node.label = label.map(str::to_owned);
        node.children = children.to_vec();
        node
    }

    #[test]
    fn a_tree_becomes_an_accesskit_tree() {
        let mut check = node(Role::CheckBox, Some("I agree"), &[]);
        check.toggled = Some(T::Mixed);
        check.is_focusable = true;
        check.is_focused = true;

        let mut disabled = node(Role::Button, Some("Not yet"), &[]);
        disabled.is_enabled = false;

        let mut progress = node(Role::ProgressBar, None, &[]);
        progress.range = Some((30.0, 0.0, 100.0));

        let tree = Tree::from_nodes(vec![
            node(Role::Window, Some("Form"), &[1, 2, 3, 4]),
            node(Role::Text, Some("Hello"), &[]),
            check,
            disabled,
            progress,
        ]);

        let update = tree_update(&tree, 2.0);

        let id = |index: usize| NodeId(tree.nodes[index].id);

        assert_eq!(update.focus, id(2), "the focus is on the check box");
        assert_eq!(update.tree.as_ref().unwrap().root, id(0));
        assert_eq!(id(0), NodeId(Tree::ROOT_ID));
        assert_eq!(update.nodes.len(), 5);

        let window = raw(&update, &tree, 0);
        assert_eq!(window.role(), accesskit::Role::Window);
        assert_eq!(window.label(), Some("Form"));
        assert_eq!(window.children(), [id(1), id(2), id(3), id(4)]);
        assert_eq!(window.transform(), Some(&accesskit::Affine::scale(2.0)));

        let text = raw(&update, &tree, 1);
        assert_eq!(text.role(), accesskit::Role::Label);
        assert_eq!(text.value(), Some("Hello"));

        let check = raw(&update, &tree, 2);
        assert_eq!(check.role(), accesskit::Role::CheckBox);
        assert_eq!(check.label(), Some("I agree"));
        assert_eq!(check.toggled(), Some(Toggled::Mixed));
        assert!(check.supports_action(Action::Click));
        assert!(check.supports_action(Action::Focus));
        assert_eq!(
            check.bounds(),
            Some(accesskit::Rect {
                x0: 10.0,
                y0: 20.0,
                x1: 40.0,
                y1: 60.0
            })
        );

        let disabled = raw(&update, &tree, 3);
        assert!(disabled.is_disabled());
        assert!(!disabled.supports_action(Action::Click));

        let progress = raw(&update, &tree, 4);
        assert_eq!(progress.role(), accesskit::Role::ProgressIndicator);
        assert_eq!(progress.numeric_value(), Some(30.0));
        assert_eq!(progress.max_numeric_value(), Some(100.0));
    }

    #[test]
    fn a_scroll_area_cuts_what_is_inside_and_says_how_far_it_is() {
        let mut area = node(Role::ScrollArea, None, &[2]);
        area.bounds = Rectangle::new(Point::ORIGIN, Size::new(200.0, 100.0));
        area.scroll = Some(Scroll {
            offset: Vector::new(0.0, 40.0),
            content: Size::new(200.0, 500.0),
        });

        let tree = Tree::from_nodes(vec![
            node(Role::Window, Some("List"), &[1, 3]),
            area,
            node(Role::Button, Some("Send a report"), &[]),
            node(Role::Button, Some("Beside the list"), &[]),
        ]);

        let update = tree_update(&tree, 1.0);

        let area = raw(&update, &tree, 1);
        assert_eq!(area.role(), accesskit::Role::ScrollView);
        assert!(area.clips_children());
        assert_eq!(area.scroll_y(), Some(40.0));
        assert_eq!(area.scroll_y_max(), Some(400.0));
        assert_eq!(area.scroll_x_max(), Some(0.0));

        let inside = raw(&update, &tree, 2);
        assert!(inside.supports_action(Action::ScrollIntoView));

        let beside = raw(&update, &tree, 3);
        assert!(!beside.supports_action(Action::ScrollIntoView));
    }

    #[test]
    fn a_name_stands_before_a_placeholder_and_a_secret_is_a_password() {
        let mut named = node(Role::TextInput, Some("Email"), &[]);
        named.placeholder = Some("you@example.com".to_owned());

        let mut unnamed = node(Role::TextInput, Some("you@example.com"), &[]);
        unnamed.placeholder = Some("you@example.com".to_owned());

        let mut list = node(Role::ComboBox, Some("Language"), &[]);
        list.value = Some("Rust".to_owned());

        let mut plain_list = node(Role::ComboBox, Some("Choose"), &[]);
        plain_list.placeholder = Some("Choose".to_owned());
        plain_list.value = Some("Rust".to_owned());

        let tree = Tree::from_nodes(vec![
            node(Role::Window, Some("Form"), &[1, 2, 3, 4, 5]),
            named,
            unnamed,
            node(Role::SecureTextInput, Some("Password"), &[]),
            list,
            plain_list,
        ]);

        let update = tree_update(&tree, 1.0);

        let named = raw(&update, &tree, 1);
        assert_eq!(named.label(), Some("Email"));
        assert_eq!(named.placeholder(), Some("you@example.com"));

        let unnamed = raw(&update, &tree, 2);
        assert_eq!(unnamed.label(), Some("you@example.com"));
        assert_eq!(unnamed.placeholder(), None);

        let secret = raw(&update, &tree, 3);
        assert_eq!(secret.role(), accesskit::Role::PasswordInput);
        assert_eq!(secret.value(), None);

        let list = raw(&update, &tree, 4);
        assert_eq!(list.label(), Some("Language"));
        assert_eq!(list.value(), Some("Rust"));

        let plain_list = raw(&update, &tree, 5);
        assert_eq!(plain_list.label(), Some("Rust"));
    }

    /// The node of the update that the node of the tree became.
    fn raw<'a>(update: &'a TreeUpdate, tree: &Tree, index: usize) -> &'a Node {
        let id = NodeId(tree.nodes[index].id);

        update
            .nodes
            .iter()
            .find_map(|(node, raw)| (*node == id).then_some(raw))
            .expect("the node is in the update")
    }

    fn by_id(update: &TreeUpdate, id: NodeId) -> &Node {
        update
            .nodes
            .iter()
            .find_map(|(node, raw)| (*node == id).then_some(raw))
            .expect("the node is in the update")
    }

    #[test]
    fn a_node_keeps_its_identifier_when_the_tree_changes_around_it() {
        use crate::core::widget::operation::accessible::Selection;

        let before = Tree::from_nodes(vec![
            node(Role::Window, Some("Form"), &[1, 2]),
            node(Role::Text, Some("Name"), &[]),
            node(Role::Button, Some("Save"), &[]),
        ]);

        // A notice above, and the button somewhere else
        let mut moved = node(Role::Button, Some("Save"), &[]);
        moved.bounds =
            Rectangle::new(Point::new(300.0, 400.0), Size::new(5.0, 5.0));

        let mut field = node(Role::TextInput, Some("Name"), &[]);
        field.selection = Some(Selection::caret(0));

        let after = Tree::from_nodes(vec![
            node(Role::Window, Some("Form"), &[1, 2, 3, 4]),
            node(Role::Text, Some("Saved."), &[]),
            node(Role::Text, Some("Name"), &[]),
            field,
            moved,
        ]);

        assert_eq!(before.nodes[1].id, after.nodes[2].id);
        assert_eq!(before.nodes[2].id, after.nodes[4].id);

        let a = tree_update(&before, 1.0);
        let b = tree_update(&after, 1.0);

        assert_eq!(a.nodes[2].0, NodeId(before.nodes[2].id));
        assert_eq!(raw(&b, &after, 4).label(), Some("Save"));
        assert_eq!(a.tree.unwrap().root, b.tree.unwrap().root);

        // No identifier is used twice, the lines of the field included
        let mut ids: Vec<_> = b.nodes.iter().map(|(id, _)| id.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), b.nodes.len());
        assert_eq!(b.nodes.len(), 6, "five nodes and the line of the field");
    }

    #[test]
    fn the_text_of_a_field_is_told_line_by_line_with_its_caret() {
        use crate::core::widget::operation::accessible::Selection;

        let mut editor = node(Role::TextEditor, Some("Notes"), &[]);
        // "Zoë" is four bytes: the caret after it is at 4; the selection
        // begins in the second line, after "tw"
        editor.value = Some("Zoë\ntwo words\n".to_owned());
        editor.selection = Some(Selection {
            anchor: 7,
            focus: 4,
        });

        let mut empty = node(Role::TextInput, Some("Name"), &[]);
        empty.value = Some(String::new());
        empty.selection = Some(Selection::caret(0));

        let mut secret = node(Role::SecureTextInput, Some("Password"), &[]);
        secret.selection = Some(Selection::caret(2));

        let tree = Tree::from_nodes(vec![
            node(Role::Window, Some("Form"), &[1, 2, 3]),
            editor,
            empty,
            secret,
        ]);

        let update = tree_update(&tree, 1.0);

        let editor = raw(&update, &tree, 1);
        assert_eq!(editor.role(), accesskit::Role::MultilineTextInput);
        assert_eq!(editor.value(), Some("Zoë\ntwo words\n"));

        let lines: Vec<_> = editor
            .children()
            .iter()
            .map(|id| by_id(&update, *id))
            .collect();

        assert_eq!(lines.len(), 3, "two lines and the empty one after them");
        assert!(lines.iter().all(|l| l.role() == accesskit::Role::TextRun));
        assert_eq!(lines[0].value(), Some("Zoë\n"));
        assert_eq!(lines[0].character_lengths(), [1, 1, 2, 1]);
        assert_eq!(lines[1].value(), Some("two words\n"));
        assert_eq!(lines[1].word_starts(), [4]);
        assert_eq!(lines[1].character_lengths().len(), 10);
        assert_eq!(lines[2].value(), Some(""));

        let selection = editor.text_selection().expect("a selection");
        assert_eq!(selection.anchor.node, editor.children()[1]);
        assert_eq!(selection.anchor.character_index, 2);
        assert_eq!(selection.focus.node, editor.children()[0]);
        assert_eq!(
            selection.focus.character_index, 3,
            "characters as a person counts them, not bytes"
        );

        // A field with nothing in it has a line for the caret to be in
        let empty = raw(&update, &tree, 2);
        assert_eq!(empty.children().len(), 1);
        let caret = empty.text_selection().expect("a caret");
        assert_eq!(caret.anchor, caret.focus);
        assert_eq!(caret.focus.node, empty.children()[0]);
        assert_eq!(caret.focus.character_index, 0);

        // Nothing of a secret
        let secret = raw(&update, &tree, 3);
        assert!(secret.children().is_empty());
        assert!(secret.text_selection().is_none());
    }

    #[test]
    fn open_lists_status_lines_and_sliders_say_what_they_are() {
        use crate::core::widget::operation::accessible::Live;

        let mut list = node(Role::ComboBox, Some("How often"), &[]);
        list.expanded = Some(true);

        let mut chosen = node(Role::MenuItem, Some("Daily"), &[]);
        chosen.selected = Some(true);

        let mut other = node(Role::MenuItem, Some("Hourly"), &[]);
        other.selected = Some(false);

        let mut status = node(Role::Text, Some("Up to date"), &[]);
        status.live = Some(Live::Polite);

        let mut slider = node(Role::Slider, Some("Volume"), &[]);
        slider.range = Some((40.0, 0.0, 100.0));
        slider.step = Some(5.0);
        slider.is_focusable = true;

        let mut combo = node(Role::EditableComboBox, Some("Language"), &[]);
        combo.value = Some("Du".to_owned());
        combo.expanded = Some(false);

        let tree = Tree::from_nodes(vec![
            node(Role::Window, Some("Form"), &[1, 2, 5, 6, 7]),
            list,
            node(Role::Menu, None, &[3, 4]),
            chosen,
            other,
            status,
            slider,
            combo,
        ]);

        let update = tree_update(&tree, 1.0);

        let list = raw(&update, &tree, 1);
        assert_eq!(list.is_expanded(), Some(true));

        let menu = raw(&update, &tree, 2);
        assert_eq!(menu.role(), accesskit::Role::ListBox);
        assert_eq!(menu.children().len(), 2);

        let chosen = raw(&update, &tree, 3);
        assert_eq!(chosen.role(), accesskit::Role::ListBoxOption);
        assert_eq!(chosen.is_selected(), Some(true));
        assert!(chosen.supports_action(Action::Click));
        assert_eq!(raw(&update, &tree, 4).is_selected(), Some(false));

        let status = raw(&update, &tree, 5);
        assert_eq!(status.role(), accesskit::Role::Label);
        assert_eq!(status.live(), Some(accesskit::Live::Polite));
        assert_eq!(status.value(), Some("Up to date"));
        assert!(
            status.children().is_empty(),
            "no lines: nothing is selected"
        );

        let slider = raw(&update, &tree, 6);
        assert_eq!(slider.role(), accesskit::Role::Slider);
        assert_eq!(slider.label(), Some("Volume"));
        assert_eq!(slider.numeric_value(), Some(40.0));
        assert_eq!(slider.numeric_value_step(), Some(5.0));
        assert!(slider.supports_action(Action::Increment));
        assert!(slider.supports_action(Action::SetValue));

        let combo = raw(&update, &tree, 7);
        assert_eq!(combo.role(), accesskit::Role::EditableComboBox);
        assert_eq!(combo.is_expanded(), Some(false));
        assert_eq!(combo.children().len(), 1, "what is typed, as a line");
    }
}
