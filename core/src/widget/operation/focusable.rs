//! Operate on widgets that can be focused.
use crate::Rectangle;
use crate::widget::Id;
use crate::widget::operation::{self, Operation, Outcome};

use std::any::Any;

/// The internal state of a widget that can be focused.
pub trait Focusable {
    /// Returns whether the widget is focused or not.
    fn is_focused(&self) -> bool;

    /// Focuses the widget.
    fn focus(&mut self);

    /// Unfocuses the widget.
    fn unfocus(&mut self);

    /// Returns whether [`focus_next`] and [`focus_previous`] stop at the
    /// widget. A widget that is disabled, but can still hold the focus (a
    /// field that cannot be typed in and can be copied from), says `false`:
    /// Tab passes it, and takes the focus away from it.
    fn is_tab_stop(&self) -> bool {
        true
    }
}

/// A [`Focusable`] that Tab does not stop at; see
/// [`Focusable::is_tab_stop`]. For a widget whose state does not know
/// whether the widget is disabled.
#[allow(missing_debug_implementations)]
pub struct Passed<'a>(pub &'a mut dyn Focusable);

impl Focusable for Passed<'_> {
    fn is_focused(&self) -> bool {
        self.0.is_focused()
    }

    fn focus(&mut self) {
        self.0.focus();
    }

    fn unfocus(&mut self) {
        self.0.unfocus();
    }

    fn is_tab_stop(&self) -> bool {
        false
    }
}

/// A summary of the focusable widgets present on a widget tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Count {
    /// The index of the current focused widget, if any.
    pub focused: Option<usize>,

    /// The total amount of focusable widgets.
    pub total: usize,
}

/// Produces an [`Operation`] that focuses the widget with the given [`Id`].
pub fn focus<T>(target: Id) -> impl Operation<T> {
    struct Focus {
        target: Id,
    }

    impl<T> Operation<T> for Focus {
        fn focusable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            match id {
                Some(id) if id == &self.target => {
                    state.focus();
                }
                _ => {
                    state.unfocus();
                }
            }
        }

        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
            operate(self);
        }
    }

    Focus { target }
}

/// Produces an [`Operation`] that unfocuses the focused widget.
pub fn unfocus<T>() -> impl Operation<T> {
    struct Unfocus;

    impl<T> Operation<T> for Unfocus {
        fn focusable(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            state.unfocus();
        }

        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
            operate(self);
        }
    }

    Unfocus
}

/// The edge of a focus trap, announced by the widget that is one (the
/// `focus_trap` of `iced_widget`) through [`Operation::custom`] before and
/// after it lets an [`Operation`] traverse what is inside it.
///
/// While a trap is in the tree, [`focus_next`], [`focus_previous`] and
/// [`count`] only see what is inside it; with several, the one that comes
/// last (the one drawn on top in a `stack`) holds.
///
/// It travels through [`Operation::custom`] so that every [`Operation`]
/// that wraps another one hands it on as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    /// What follows is inside a trap.
    Enter,
    /// The trap entered last ends here.
    Leave,
}

impl Trap {
    /// Announces this edge to the given [`Operation`].
    pub fn announce<T>(
        mut self,
        operation: &mut dyn Operation<T>,
        bounds: Rectangle,
    ) {
        operation.custom(None, bounds, &mut self);
    }
}

/// Returns whether a widget with these bounds can be seen at all: one that
/// was laid out with no width or no height is skipped by [`focus_next`] and
/// [`focus_previous`].
pub fn is_visible(bounds: Rectangle) -> bool {
    bounds.width > 0.0 && bounds.height > 0.0
}

/// Which focusable widgets take part: all of them, or those inside the trap
/// that comes last.
#[derive(Debug, Clone, Default)]
struct Scope {
    /// The traps entered so far.
    seen: usize,
    /// The ordinals of the traps we are inside of.
    stack: Vec<usize>,
}

impl Scope {
    fn custom(&mut self, state: &mut dyn Any) {
        match state.downcast_ref::<Trap>() {
            Some(Trap::Enter) => {
                self.stack.push(self.seen);
                self.seen += 1;
            }
            Some(Trap::Leave) => {
                let _ = self.stack.pop();
            }
            None => {}
        }
    }

    /// Whether we are inside the trap with the given ordinal (`None`: no trap
    /// in the tree, everything takes part).
    fn is_inside(&self, trap: Option<usize>) -> bool {
        trap.is_none_or(|trap| self.stack.contains(&trap))
    }
}

/// What the first pass over the tree found: the focusable widgets that take
/// part, and the trap that holds, if any.
#[derive(Debug, Clone, Copy, Default)]
struct Order {
    count: Count,
    trap: Option<usize>,
}

fn order() -> impl Operation<Order> {
    struct FindOrder {
        scope: Scope,
        // Everything, then one count per trap
        everything: Count,
        traps: Vec<Count>,
    }

    impl Operation<Order> for FindOrder {
        fn focusable(
            &mut self,
            _id: Option<&Id>,
            bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            if !is_visible(bounds) || !state.is_tab_stop() {
                return;
            }

            let is_focused = state.is_focused();

            let counts = std::iter::once(&mut self.everything).chain(
                self.traps.iter_mut().enumerate().filter_map(|(i, count)| {
                    self.scope.stack.contains(&i).then_some(count)
                }),
            );

            for count in counts {
                if is_focused {
                    count.focused = Some(count.total);
                }

                count.total += 1;
            }
        }

        fn custom(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Any,
        ) {
            if let Some(Trap::Enter) = state.downcast_ref::<Trap>() {
                self.traps.push(Count::default());
            }

            self.scope.custom(state);
        }

        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn Operation<Order>),
        ) {
            operate(self);
        }

        fn finish(&self) -> Outcome<Order> {
            Outcome::Some(match self.traps.last() {
                Some(count) => Order {
                    count: *count,
                    trap: Some(self.traps.len() - 1),
                },
                None => Order {
                    count: self.everything,
                    trap: None,
                },
            })
        }
    }

    FindOrder {
        scope: Scope::default(),
        everything: Count::default(),
        traps: Vec::new(),
    }
}

/// Produces an [`Operation`] that generates a [`Count`] and chains it with the
/// provided function to build a new [`Operation`].
///
/// Only the widgets that [`focus_next`] would visit are counted: those that
/// can be seen and, while there is a [`Trap`], those inside it.
pub fn count() -> impl Operation<Count> {
    operation::map(order(), |order| order.count)
}

/// Whether to go forward or backward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Next,
    Previous,
}

struct Move {
    step: Step,
    order: Order,
    scope: Scope,
    current: usize,
}

impl<T> Operation<T> for Move {
    fn focusable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        state: &mut dyn Focusable,
    ) {
        // What does not take part does not keep the focus either: the keys
        // would go on reaching a widget under a modal layer.
        if !is_visible(bounds)
            || !state.is_tab_stop()
            || !self.scope.is_inside(self.order.trap)
        {
            if state.is_focused() {
                state.unfocus();
            }

            return;
        }

        let Count { focused, total } = self.order.count;

        // Inside a trap the focus goes round. Without one, a step past the
        // end leaves nothing focused and the next one starts over, as
        // upstream does.
        let goes_round = self.order.trap.is_some();

        let target = match (self.step, focused) {
            (Step::Next, None) => Some(0),
            (Step::Previous, None) => total.checked_sub(1),
            (Step::Next, Some(focused)) if focused + 1 < total => {
                Some(focused + 1)
            }
            (Step::Previous, Some(focused)) if focused > 0 => Some(focused - 1),
            (Step::Next, Some(_)) => goes_round.then_some(0),
            (Step::Previous, Some(_)) => {
                goes_round.then(|| total.saturating_sub(1))
            }
        };

        if target == Some(self.current) {
            if !state.is_focused() {
                state.focus();
            }
        } else if state.is_focused() {
            state.unfocus();
        }

        self.current += 1;
    }

    fn custom(
        &mut self,
        _id: Option<&Id>,
        _bounds: Rectangle,
        state: &mut dyn Any,
    ) {
        self.scope.custom(state);
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }
}

/// Produces an [`Operation`] that searches for the current focused widget, and
/// - if found, focuses the previous focusable widget (the last one, from the
///   first).
/// - if not found, focuses the last focusable widget.
///
/// A widget that cannot be seen (see [`is_visible`]), that is disabled
/// ([`Focusable::is_tab_stop`]; most disabled widgets do not announce
/// themselves at all) or that is outside of the [`Trap`] that holds is
/// skipped, and loses the focus if it had it.
pub fn focus_previous<T>() -> impl Operation<T>
where
    T: Send + 'static,
{
    operation::then(order(), |order| Move {
        step: Step::Previous,
        order,
        scope: Scope::default(),
        current: 0,
    })
}

/// Produces an [`Operation`] that searches for the current focused widget, and
/// - if found, focuses the next focusable widget (the first one, from the
///   last).
/// - if not found, focuses the first focusable widget.
///
/// A widget that cannot be seen (see [`is_visible`]), that is disabled
/// ([`Focusable::is_tab_stop`]; most disabled widgets do not announce
/// themselves at all) or that is outside of the [`Trap`] that holds is
/// skipped, and loses the focus if it had it.
pub fn focus_next<T>() -> impl Operation<T>
where
    T: Send + 'static,
{
    operation::then(order(), |order| Move {
        step: Step::Next,
        order,
        scope: Scope::default(),
        current: 0,
    })
}

/// Produces an [`Operation`] that searches for the current focused widget
/// and stores its ID. This ignores widgets that do not have an ID.
pub fn find_focused() -> impl Operation<Id> {
    struct FindFocused {
        focused: Option<Id>,
    }

    impl Operation<Id> for FindFocused {
        fn focusable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            if state.is_focused() && id.is_some() {
                self.focused = id.cloned();
            }
        }

        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn Operation<Id>),
        ) {
            operate(self);
        }

        fn finish(&self) -> Outcome<Id> {
            if let Some(id) = &self.focused {
                Outcome::Some(id.clone())
            } else {
                Outcome::None
            }
        }
    }

    FindFocused { focused: None }
}

/// Produces an [`Operation`] that searches for the focusable widget
/// and stores whether it is focused or not. This ignores widgets that
/// do not have an ID.
pub fn is_focused(target: Id) -> impl Operation<bool> {
    struct IsFocused {
        target: Id,
        is_focused: Option<bool>,
    }

    impl Operation<bool> for IsFocused {
        fn focusable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            if id.is_some_and(|id| *id == self.target) {
                self.is_focused = Some(state.is_focused());
            }
        }

        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn Operation<bool>),
        ) {
            if self.is_focused.is_some() {
                return;
            }

            operate(self);
        }

        fn finish(&self) -> Outcome<bool> {
            self.is_focused.map_or(Outcome::None, Outcome::Some)
        }
    }

    IsFocused {
        target,
        is_focused: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Size;

    /// What a widget tree tells an operation, flattened.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Item {
        Widget {
            is_focused: bool,
            size: f32,
            is_tab_stop: bool,
        },
        Enter,
        Leave,
    }

    const fn widget() -> Item {
        Item::Widget {
            is_focused: false,
            size: 10.0,
            is_tab_stop: true,
        }
    }

    const HIDDEN: Item = Item::Widget {
        is_focused: false,
        size: 0.0,
        is_tab_stop: true,
    };

    const DISABLED: Item = Item::Widget {
        is_focused: false,
        size: 10.0,
        is_tab_stop: false,
    };

    struct State<'a>(&'a mut bool, bool);

    impl Focusable for State<'_> {
        fn is_focused(&self) -> bool {
            *self.0
        }

        fn focus(&mut self) {
            *self.0 = true;
        }

        fn unfocus(&mut self) {
            *self.0 = false;
        }

        fn is_tab_stop(&self) -> bool {
            self.1
        }
    }

    fn visit<T>(items: &mut [Item], operation: &mut dyn Operation<T>) {
        operation.traverse(&mut |operation| {
            for item in items.iter_mut() {
                match item {
                    Item::Widget {
                        is_focused,
                        size,
                        is_tab_stop,
                    } => operation.focusable(
                        None,
                        Rectangle::with_size(Size::new(*size, *size)),
                        &mut State(is_focused, *is_tab_stop),
                    ),
                    Item::Enter => {
                        Trap::Enter.announce(operation, Rectangle::INFINITE);
                    }
                    Item::Leave => {
                        Trap::Leave.announce(operation, Rectangle::INFINITE);
                    }
                }
            }
        });
    }

    fn run<T: 'static>(
        items: &mut [Item],
        operation: impl Operation<T> + 'static,
    ) -> Option<T> {
        let mut current: Box<dyn Operation<T>> = Box::new(operation);

        loop {
            visit(items, current.as_mut());

            match current.finish() {
                Outcome::Chain(next) => current = next,
                Outcome::Some(output) => return Some(output),
                Outcome::None => return None,
            }
        }
    }

    /// The indices of the widgets that have the focus.
    fn focused(items: &[Item]) -> Vec<usize> {
        items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                matches!(
                    item,
                    Item::Widget {
                        is_focused: true,
                        ..
                    }
                )
                .then_some(i)
            })
            .collect()
    }

    fn next(items: &mut [Item]) -> Vec<usize> {
        let _ = run(items, focus_next::<()>());
        focused(items)
    }

    fn previous(items: &mut [Item]) -> Vec<usize> {
        let _ = run(items, focus_previous::<()>());
        focused(items)
    }

    #[test]
    fn without_a_trap_the_end_leaves_nothing_focused_and_then_starts_over() {
        let mut items = [widget(), widget(), widget()];

        assert_eq!(next(&mut items), [0]);
        assert_eq!(next(&mut items), [1]);
        assert_eq!(next(&mut items), [2]);
        assert_eq!(next(&mut items), [] as [usize; 0]);
        assert_eq!(next(&mut items), [0]);

        assert_eq!(previous(&mut items), [] as [usize; 0]);
        assert_eq!(previous(&mut items), [2]);
        assert_eq!(previous(&mut items), [1]);
    }

    #[test]
    fn what_is_hidden_or_disabled_is_skipped() {
        let mut items = [HIDDEN, widget(), DISABLED, HIDDEN, widget(), HIDDEN];

        assert_eq!(run(&mut items, count()).map(|count| count.total), Some(2));

        assert_eq!(next(&mut items), [1]);
        assert_eq!(next(&mut items), [4]);
        assert_eq!(previous(&mut items), [1]);
        assert_eq!(previous(&mut items), [] as [usize; 0]);
        assert_eq!(previous(&mut items), [4]);
    }

    #[test]
    fn a_disabled_widget_that_holds_the_focus_gives_it_up_to_tab() {
        // A field that cannot be typed in was clicked
        let mut items = [
            widget(),
            Item::Widget {
                is_focused: true,
                size: 10.0,
                is_tab_stop: false,
            },
            widget(),
        ];

        assert_eq!(
            run(&mut items, count()),
            Some(Count {
                focused: None,
                total: 2
            })
        );
        assert_eq!(next(&mut items), [0]);
        assert_eq!(next(&mut items), [2]);
    }

    #[test]
    fn inside_a_trap_the_focus_goes_round_and_what_is_outside_loses_it() {
        let mut items = [
            Item::Widget {
                is_focused: true,
                size: 10.0,
                is_tab_stop: true,
            },
            widget(),
            Item::Enter,
            widget(),
            widget(),
            Item::Leave,
            widget(),
        ];

        assert_eq!(
            run(&mut items, count()),
            Some(Count {
                focused: None,
                total: 2
            })
        );

        assert_eq!(next(&mut items), [3]);
        assert_eq!(next(&mut items), [4]);
        assert_eq!(next(&mut items), [3]);
        assert_eq!(previous(&mut items), [4]);
        assert_eq!(previous(&mut items), [3]);
    }

    #[test]
    fn the_trap_that_comes_last_holds() {
        // Two layers of a stack, and a trap inside a trap
        let mut items = [
            Item::Enter,
            widget(),
            Item::Leave,
            Item::Enter,
            widget(),
            Item::Enter,
            widget(),
            widget(),
            Item::Leave,
            Item::Leave,
        ];

        assert_eq!(next(&mut items), [6]);
        assert_eq!(next(&mut items), [7]);
        assert_eq!(next(&mut items), [6]);
    }

    #[test]
    fn a_trap_with_one_widget_keeps_the_focus_on_it() {
        let mut items = [widget(), Item::Enter, widget(), Item::Leave];

        assert_eq!(next(&mut items), [2]);
        assert_eq!(next(&mut items), [2]);
        assert_eq!(previous(&mut items), [2]);
    }
}
