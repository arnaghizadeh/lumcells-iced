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
use crate::core::{Point, Rectangle};
use crate::runtime::UserInterface;
use crate::runtime::accessibility::{Role, Tree};

use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler,
    DeactivationHandler, Node, NodeId, Toggled, TreeId, TreeInfo, TreeUpdate,
};

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What assistive technology asks a window to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    /// Press what is at this point, as a click of the pointer would.
    Press(Point),
    /// Give the keyboard focus to the widget with these bounds.
    Focus(Rectangle),
}

#[derive(Default)]
struct Shared {
    /// What assistive technology asked for and the window did not do yet.
    requests: Vec<ActionRequest>,
    /// Where the nodes of the last tree are, by identifier.
    bounds: Vec<Rectangle>,
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
            shared.bounds = tree.nodes.iter().map(|node| node.bounds).collect();
            shared.is_tree_wanted = false;

            tree_update(&tree, scale_factor)
        });
    }

    /// Takes what assistive technology asked for since the last time.
    pub fn requests(&mut self) -> Vec<Request> {
        let mut shared = lock(&self.shared);
        let requests = std::mem::take(&mut shared.requests);

        requests
            .into_iter()
            .filter_map(|request| {
                let bounds =
                    *shared.bounds.get(request.target_node.0 as usize)?;

                match request.action {
                    Action::Click => Some(Request::Press(bounds.center())),
                    Action::Focus => Some(Request::Focus(bounds)),
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
    use crate::core::mouse;
    use crate::core::widget::operation::{Focusable, Operation};

    match request {
        Request::Press(point) => {
            let _ = ui.update(
                &[
                    crate::core::Event::Mouse(mouse::Event::ButtonPressed(
                        mouse::Button::Left,
                    )),
                    crate::core::Event::Mouse(mouse::Event::ButtonReleased(
                        mouse::Button::Left,
                    )),
                ],
                mouse::Cursor::Available(point),
                renderer,
                clipboard,
                messages,
            );
        }
        Request::Focus(bounds) => {
            struct Focus(Rectangle);

            impl Operation for Focus {
                fn traverse(
                    &mut self,
                    operate: &mut dyn FnMut(&mut dyn Operation),
                ) {
                    operate(self);
                }

                fn focusable(
                    &mut self,
                    _id: Option<&crate::core::widget::Id>,
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

            ui.operate(renderer, &mut Focus(bounds));
        }
    }
}

/// The identifier of a node is its place in the [`Tree`].
fn node_id(index: usize) -> NodeId {
    NodeId(index as u64)
}

/// Turns a [`Tree`] into what AccessKit wants: every node, every time.
pub fn tree_update(tree: &Tree, scale_factor: f64) -> TreeUpdate {
    let nodes = tree
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let role = match node.role {
                Role::Window => accesskit::Role::Window,
                Role::Text => accesskit::Role::Label,
                Role::TextInput => accesskit::Role::TextInput,
                Role::Button => accesskit::Role::Button,
                Role::CheckBox => accesskit::Role::CheckBox,
                Role::Switch => accesskit::Role::Switch,
                Role::RadioButton => accesskit::Role::RadioButton,
                Role::ComboBox => accesskit::Role::ComboBox,
                Role::ProgressBar => accesskit::Role::ProgressIndicator,
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

            match (&node.label, &node.value, node.role) {
                // The text of a piece of text is its value
                (Some(text), _, Role::Text) => raw.set_value(text.as_str()),
                // A list is called by the option that is chosen, and by its
                // placeholder while none is: screen readers do not read
                // the value of a combo box
                (_, Some(chosen), Role::ComboBox) => {
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
    use crate::runtime::accessibility::Node as TreeNode;

    fn node(role: Role, label: Option<&str>, children: &[usize]) -> TreeNode {
        TreeNode {
            role,
            label: label.map(str::to_owned),
            value: None,
            toggled: None,
            range: None,
            is_enabled: true,
            is_focusable: false,
            is_focused: false,
            bounds: Rectangle::new(
                Point::new(10.0, 20.0),
                crate::core::Size::new(30.0, 40.0),
            ),
            children: children.to_vec(),
        }
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
}
