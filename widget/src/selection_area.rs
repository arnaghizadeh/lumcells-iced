//! Select text over several widgets at once.
use crate::core::clipboard::{self, Clipboard};
use crate::core::keyboard;
use crate::core::layout;
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::touch;
use crate::core::widget::operation::{self, Operation};
use crate::core::widget::text::selection::{Command, Part};
use crate::core::widget::tree::{self, Tree};
use crate::core::{
    Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Vector,
    Widget,
};

/// A container in which a selection goes from one selectable text to the
/// next: a drag that begins in one text (`text(..).selectable(true)`,
/// `rich_text(..).selectable(true)`, a selectable `markdown`) and ends in
/// another selects the end of the first, everything between, and the start
/// of the last, in the order of the widgets.
///
/// - The copy key of the platform copies all of it, a line feed between
///   two texts; on X11 and Wayland it is also the primary selection.
/// - Ctrl+A (Cmd+A) after a click inside it selects every text in it.
/// - Inside one text everything is as without the container (a double
///   click selects a word, Shift with the arrows moves the end).
///
/// It looks like nothing and takes no space of its own. Texts that are not
/// selectable are not part of it.
#[allow(missing_debug_implementations)]
pub struct SelectionArea<
    'a,
    Message,
    Theme = crate::Theme,
    Renderer = crate::Renderer,
> {
    content: Element<'a, Message, Theme, Renderer>,
}

impl<'a, Message, Theme, Renderer> SelectionArea<'a, Message, Theme, Renderer> {
    /// Creates a [`SelectionArea`] with the given content.
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self {
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct State {
    /// Where the pointer was pressed, while it is held.
    drag: Option<Point>,
    /// Whether the last press was inside: Ctrl+A is for the area.
    is_active: bool,
}

/// Asks the selectable texts inside what they are, and tells them what to
/// select.
struct Ask {
    /// The parts in the order of the widgets, where they are drawn.
    parts: Vec<Part>,
    /// What each part is to select, if anything is asked.
    commands: Option<Vec<Command>>,
    /// The scrolling of what is visited now.
    offset: Vector,
    scrolled_next: Option<Vector>,
}

impl Ask {
    fn new(commands: Option<Vec<Command>>) -> Self {
        Self {
            parts: Vec::new(),
            commands,
            offset: Vector::ZERO,
            scrolled_next: None,
        }
    }

    /// What is selected in all the parts, a line feed between two, and in
    /// how many parts.
    fn selected(&self) -> (String, usize) {
        let selected: Vec<_> = self
            .parts
            .iter()
            .filter_map(|part| part.selected.as_deref())
            .collect();

        (selected.join("\n"), selected.len())
    }
}

impl Operation for Ask {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let scrolled = self.scrolled_next.take().unwrap_or(Vector::ZERO);

        self.offset += scrolled;
        operate(self);
        self.offset -= scrolled;
    }

    fn scrollable(
        &mut self,
        _id: Option<&crate::core::widget::Id>,
        _bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        // What it scrolls is visited in the `traverse` that follows
        self.scrolled_next = Some(translation);
    }

    fn custom(
        &mut self,
        _id: Option<&crate::core::widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn std::any::Any,
    ) {
        let Some(part) = state.downcast_mut::<Part>() else {
            return;
        };

        let offset = self.offset;

        if let Some(commands) = &self.commands
            && let Some(command) = commands.get(self.parts.len())
        {
            // From where it is drawn to where its layout puts it
            part.command = Some(match *command {
                Command::Between(from, to) => {
                    Command::Between(from + offset, to + offset)
                }
                Command::From(point) => Command::From(point + offset),
                Command::Until(point) => Command::Until(point + offset),
                command => command,
            });
        }

        self.parts.push(Part {
            bounds: part.bounds - offset,
            selected: part.selected.clone(),
            command: None,
        });
    }
}

/// The index of the part the point is in, or else of the nearest one: the
/// one on its line before any other.
fn nearest(parts: &[Part], point: Point) -> Option<usize> {
    let distance = |bounds: &Rectangle| {
        let x = (bounds.x - point.x)
            .max(point.x - bounds.x - bounds.width)
            .max(0.0);
        let y = (bounds.y - point.y)
            .max(point.y - bounds.y - bounds.height)
            .max(0.0);

        (y, x)
    };

    parts
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            distance(&a.bounds)
                .partial_cmp(&distance(&b.bounds))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
}

/// What each part selects when the pointer was pressed at `anchor` and is
/// at `head` now.
fn commands(parts: &[Part], anchor: Point, head: Point) -> Vec<Command> {
    let (Some(from), Some(to)) = (nearest(parts, anchor), nearest(parts, head))
    else {
        return Vec::new();
    };

    (0..parts.len())
        .map(|index| {
            if from == to && index == from {
                Command::Between(anchor, head)
            } else if index < from.min(to) || index > from.max(to) {
                Command::Clear
            } else if index == from {
                if from < to {
                    Command::From(anchor)
                } else {
                    Command::Until(anchor)
                }
            } else if index == to {
                if from < to {
                    Command::Until(head)
                } else {
                    Command::From(head)
                }
            } else {
                Command::All
            }
        })
        .collect()
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for SelectionArea<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            limits,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let Tree {
            state, children, ..
        } = tree;

        let state = state.downcast_mut::<State>();
        let content = &mut children[0];

        let ask = |widget: &mut Element<'_, Message, Theme, Renderer>,
                   content: &mut Tree,
                   commands: Option<Vec<Command>>| {
            let mut ask = Ask::new(commands);

            widget
                .as_widget_mut()
                .operate(content, layout, renderer, &mut ask);

            ask
        };

        // The keys that are for all the texts at once come before the
        // texts see them
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) = event
            && modifiers.command()
            && !shell.is_event_captured()
        {
            match key.to_latin(*physical_key) {
                Some('c') => {
                    let (selected, parts) =
                        ask(&mut self.content, content, None).selected();

                    // One text copies what is selected in it itself
                    if parts > 1 {
                        clipboard.write(clipboard::Kind::Standard, selected);
                        shell.capture_event();
                        return;
                    }
                }
                Some('a') if state.is_active => {
                    let parts = ask(&mut self.content, content, None).parts;

                    if !parts.is_empty() {
                        let _ = ask(
                            &mut self.content,
                            content,
                            Some(vec![Command::All; parts.len()]),
                        );

                        let (selected, _) =
                            ask(&mut self.content, content, None).selected();

                        clipboard.write(clipboard::Kind::Primary, selected);
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                }
                _ => {}
            }
        }

        self.content.as_widget_mut().update(
            content, event, layout, cursor, renderer, clipboard, shell,
            viewport,
        );

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                state.drag = cursor.position_over(layout.bounds());
                state.is_active = state.drag.is_some();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                if let Some(anchor) = state.drag
                    && let Some(head) = cursor.position()
                {
                    let parts = ask(&mut self.content, content, None).parts;

                    if !parts.is_empty() {
                        let _ = ask(
                            &mut self.content,
                            content,
                            Some(commands(&parts, anchor, head)),
                        );

                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(
                touch::Event::FingerLifted { .. }
                | touch::Event::FingerLost { .. },
            ) => {
                if state.drag.take().is_some() {
                    let (selected, parts) =
                        ask(&mut self.content, content, None).selected();

                    if parts > 1 {
                        clipboard.write(clipboard::Kind::Primary, selected);
                    }
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            renderer_style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer>
    From<SelectionArea<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(
        area: SelectionArea<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(y: f32) -> Part {
        Part {
            bounds: Rectangle::new(Point::new(10.0, y), Size::new(100.0, 20.0)),
            selected: None,
            command: None,
        }
    }

    #[test]
    fn a_drag_over_several_texts_selects_each_ones_share() {
        let parts = [part(0.0), part(30.0), part(60.0), part(90.0)];

        let inside = Point::new(50.0, 35.0);
        let lower = Point::new(20.0, 95.0);

        // Down: the end of the first, all between, the start of the last
        assert_eq!(
            commands(&parts, inside, lower),
            [
                Command::Clear,
                Command::From(inside),
                Command::All,
                Command::Until(lower),
            ]
        );

        // Up: the same texts, the other way round
        assert_eq!(
            commands(&parts, lower, inside),
            [
                Command::Clear,
                Command::From(inside),
                Command::All,
                Command::Until(lower),
            ]
        );

        // Inside one text
        let further = Point::new(90.0, 38.0);
        assert_eq!(
            commands(&parts, inside, further),
            [
                Command::Clear,
                Command::Between(inside, further),
                Command::Clear,
                Command::Clear,
            ]
        );

        assert_eq!(commands(&[], inside, lower), []);
    }

    #[test]
    fn a_point_between_texts_belongs_to_the_nearest_on_its_line() {
        let mut beside = part(30.0);
        beside.bounds.x = 200.0;

        let parts = [part(0.0), part(30.0), beside, part(60.0)];

        assert_eq!(nearest(&parts, Point::new(50.0, 35.0)), Some(1));
        // In the gap under the first text, nearer to it than to the second
        assert_eq!(nearest(&parts, Point::new(50.0, 22.0)), Some(0));
        // Right of everything, on the line of the second and third
        assert_eq!(nearest(&parts, Point::new(900.0, 40.0)), Some(2));
        // Left of everything on that line
        assert_eq!(nearest(&parts, Point::new(0.0, 40.0)), Some(1));
        // Under everything
        assert_eq!(nearest(&parts, Point::new(50.0, 500.0)), Some(3));
        assert_eq!(nearest(&[], Point::ORIGIN), None);
    }
}
