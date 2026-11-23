//! Move the keyboard focus with Tab and Shift+Tab.
//!
//! This is what a shell does by default when no widget captured the key;
//! see [`shell::Settings::tab_focus`](crate::shell::Settings::tab_focus).
use crate::UserInterface;
use crate::core;
use crate::core::keyboard;
use crate::core::widget::operation::{self, Operation, Outcome};

/// The direction of a focus move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// To the next focusable widget.
    Next,
    /// To the previous focusable widget.
    Previous,
}

/// Returns the [`Direction`] the given event asks for, if it is a press of
/// Tab (with or without Shift, and with no other modifier).
pub fn direction(event: &core::Event) -> Option<Direction> {
    let core::Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Tab),
        modifiers,
        ..
    }) = event
    else {
        return None;
    };

    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }

    Some(if modifiers.shift() {
        Direction::Previous
    } else {
        Direction::Next
    })
}

/// Moves the keyboard focus of the given [`UserInterface`].
///
/// Returns `false`, and does nothing, when it has no focusable widget.
pub fn shift<Message, Theme, Renderer>(
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &Renderer,
    direction: Direction,
) -> bool
where
    Renderer: core::Renderer,
{
    let mut count = operation::focusable::count();
    ui.operate(renderer, &mut operation::black_box(&mut count));

    let Outcome::Some(count) = count.finish() else {
        return false;
    };

    if count.total == 0 {
        return false;
    }

    let mut current: Box<dyn Operation> = match direction {
        Direction::Next => Box::new(operation::focusable::focus_next()),
        Direction::Previous => Box::new(operation::focusable::focus_previous()),
    };

    loop {
        ui.operate(renderer, current.as_mut());

        match current.finish() {
            Outcome::Chain(next) => {
                current = next;
            }
            Outcome::None | Outcome::Some(()) => break,
        }
    }

    let _ = reveal(ui, renderer);

    true
}

/// Scrolls what is around the widget that has the keyboard focus so that it
/// can be seen: the focus that Tab moved under the fold of a `scrollable`
/// would be out of sight. Returns whether anything was scrolled.
pub fn reveal<Message, Theme, Renderer>(
    ui: &mut UserInterface<'_, Message, Theme, Renderer>,
    renderer: &Renderer,
) -> bool
where
    Renderer: core::Renderer,
{
    use crate::accessibility::{self, Tree};

    // The tree knows where everything is drawn after scrolling
    let tree = Tree::of(ui, renderer, "", core::Rectangle::INFINITE);

    tree.focused().is_some_and(|focused| {
        accessibility::scroll_into_view(&tree, focused, ui, renderer)
    })
}
