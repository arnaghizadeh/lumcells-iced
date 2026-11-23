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
                let index = usize::try_from(request.target_node.0).ok()?;

                let target = Target {
                    tree: tree.clone(),
                    index,
                };

                let _ = tree.nodes.get(index)?;

                match request.action {
                    Action::Click => Some(Request::Press(target)),
                    Action::Focus => Some(Request::Focus(target)),
                    Action::ScrollIntoView => Some(Request::Reveal(target)),
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
    }
}

/// The identifier of a node is its place in the [`Tree`].
fn node_id(index: usize) -> NodeId {
    NodeId(index as u64)
}

/// Turns a [`Tree`] into what AccessKit wants: every node, every time.
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

    let nodes = tree
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let role = match node.role {
                Role::Window => accesskit::Role::Window,
                Role::Text => accesskit::Role::Label,
                Role::TextInput => accesskit::Role::TextInput,
                Role::SecureTextInput => accesskit::Role::PasswordInput,
                Role::ScrollArea => accesskit::Role::ScrollView,
                Role::Button => accesskit::Role::Button,
                Role::CheckBox => accesskit::Role::CheckBox,
                Role::Switch => accesskit::Role::Switch,
                Role::RadioButton => accesskit::Role::RadioButton,
                Role::ComboBox => accesskit::Role::ComboBox,
                Role::ProgressBar => accesskit::Role::ProgressIndicator,
                Role::Slider => accesskit::Role::Slider,
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
            let is_named =
                node.label.is_some() && node.label != node.placeholder;

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

            // What is inside a scroll area is cut at its edges, and it
            // says how far it is scrolled
            if let Some(scroll) = node.scroll {
                raw.set_clips_children();

                let room = |content: f32, view: f32| {
                    f64::from((content - view).max(0.0))
                };

                raw.set_scroll_x(f64::from(scroll.offset.x));
                raw.set_scroll_x_min(0.0);
                raw.set_scroll_x_max(room(
                    scroll.content.width,
                    node.bounds.width,
                ));
                raw.set_scroll_y(f64::from(scroll.offset.y));
                raw.set_scroll_y_min(0.0);
                raw.set_scroll_y_max(room(
                    scroll.content.height,
                    node.bounds.height,
                ));
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
                )
            {
                raw.add_action(Action::Click);
            }

            raw.set_children(
                node.children
                    .iter()
                    .map(|child| node_id(*child))
                    .collect::<Vec<_>>(),
            );

            (node_id(index), raw)
        })
        .collect();

    let mut info = TreeInfo::new(node_id(Tree::ROOT));
    info.toolkit_name = Some("iced".to_owned());
    info.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());

    TreeUpdate {
        nodes,
        tree: Some(info),
        tree_id: TreeId::ROOT,
        focus: node_id(tree.focused().unwrap_or(Tree::ROOT)),
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

        let tree = Tree {
            nodes: vec![
                node(Role::Window, Some("Form"), &[1, 2, 3, 4]),
                node(Role::Text, Some("Hello"), &[]),
                check,
                disabled,
                progress,
            ],
        };

        let update = tree_update(&tree, 2.0);

        assert_eq!(update.focus, NodeId(2), "the focus is on the check box");
        assert_eq!(update.tree.as_ref().unwrap().root, NodeId(0));
        assert_eq!(update.nodes.len(), 5);

        let (_, window) = &update.nodes[0];
        assert_eq!(window.role(), accesskit::Role::Window);
        assert_eq!(window.label(), Some("Form"));
        assert_eq!(
            window.children(),
            [NodeId(1), NodeId(2), NodeId(3), NodeId(4)]
        );
        assert_eq!(window.transform(), Some(&accesskit::Affine::scale(2.0)));

        let (_, text) = &update.nodes[1];
        assert_eq!(text.role(), accesskit::Role::Label);
        assert_eq!(text.value(), Some("Hello"));

        let (_, check) = &update.nodes[2];
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

        let (_, disabled) = &update.nodes[3];
        assert!(disabled.is_disabled());
        assert!(!disabled.supports_action(Action::Click));

        let (_, progress) = &update.nodes[4];
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

        let tree = Tree {
            nodes: vec![
                node(Role::Window, Some("List"), &[1, 3]),
                area,
                node(Role::Button, Some("Send a report"), &[]),
                node(Role::Button, Some("Beside the list"), &[]),
            ],
        };

        let update = tree_update(&tree, 1.0);

        let (_, area) = &update.nodes[1];
        assert_eq!(area.role(), accesskit::Role::ScrollView);
        assert!(area.clips_children());
        assert_eq!(area.scroll_y(), Some(40.0));
        assert_eq!(area.scroll_y_max(), Some(400.0));
        assert_eq!(area.scroll_x_max(), Some(0.0));

        let (_, inside) = &update.nodes[2];
        assert!(inside.supports_action(Action::ScrollIntoView));

        let (_, beside) = &update.nodes[3];
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

        let tree = Tree {
            nodes: vec![
                node(Role::Window, Some("Form"), &[1, 2, 3, 4, 5]),
                named,
                unnamed,
                node(Role::SecureTextInput, Some("Password"), &[]),
                list,
                plain_list,
            ],
        };

        let update = tree_update(&tree, 1.0);

        let (_, named) = &update.nodes[1];
        assert_eq!(named.label(), Some("Email"));
        assert_eq!(named.placeholder(), Some("you@example.com"));

        let (_, unnamed) = &update.nodes[2];
        assert_eq!(unnamed.label(), Some("you@example.com"));
        assert_eq!(unnamed.placeholder(), None);

        let (_, secret) = &update.nodes[3];
        assert_eq!(secret.role(), accesskit::Role::PasswordInput);
        assert_eq!(secret.value(), None);

        let (_, list) = &update.nodes[4];
        assert_eq!(list.label(), Some("Language"));
        assert_eq!(list.value(), Some("Rust"));

        let (_, plain_list) = &update.nodes[5];
        assert_eq!(plain_list.label(), Some("Rust"));
    }
}
