//! Keep the keyboard inside a layer while it is open.
use crate::core::keyboard;
use crate::core::keyboard::key;
use crate::core::layout;
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::widget::operation::focusable::Trap;
use crate::core::widget::{Operation, Tree};
use crate::core::{
    Clipboard, Element, Event, Layout, Length, Rectangle, Shell, Size, Vector,
    Widget,
};

/// A container that keeps the keyboard focus inside its contents while it is
/// part of the interface: what a modal layer (a dialog in a `stack`) needs.
///
/// - Tab and Shift+Tab (`focus_next` and `focus_previous`) only visit the
///   widgets inside it, and go round. A widget outside that had the focus
///   loses it at the first of them.
/// - A key that nothing inside it took goes no further: the layers below it
///   in a `stack` do not see it, and neither do subscriptions that listen
///   for ignored keys. Tab alone is let through, for the shell to move the
///   focus.
/// - [`on_escape`](Self::on_escape) produces a message when Escape is
///   pressed and nothing inside took it, since a subscription would not hear
///   of it.
///
/// It does nothing about the pointer: wrap the layer in `opaque` for that.
/// With more than one trap in the interface, the last one (the topmost
/// layer of a `stack`) holds.
#[allow(missing_debug_implementations)]
pub struct FocusTrap<
    'a,
    Message,
    Theme = crate::Theme,
    Renderer = crate::Renderer,
> {
    content: Element<'a, Message, Theme, Renderer>,
    on_escape: Option<Message>,
}

impl<'a, Message, Theme, Renderer> FocusTrap<'a, Message, Theme, Renderer> {
    /// Creates a [`FocusTrap`] with the given content.
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self {
            content: content.into(),
            on_escape: None,
        }
    }

    /// The message to produce when Escape is pressed and nothing inside the
    /// [`FocusTrap`] took the key (an open pick list closes first).
    pub fn on_escape(mut self, message: Message) -> Self {
        self.on_escape = Some(message);
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for FocusTrap<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
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
        let bounds = layout.bounds();

        Trap::Enter.announce(operation, bounds);

        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            renderer,
            operation,
        );

        Trap::Leave.announce(operation, bounds);
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        if shell.is_event_captured() {
            return;
        }

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key::Named::Tab),
                modifiers,
                ..
            }) if !(modifiers.control()
                || modifiers.alt()
                || modifiers.logo()) =>
            {
                // The shell moves the focus, inside of us
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                if *key == keyboard::Key::Named(key::Named::Escape)
                    && let Some(on_escape) = &self.on_escape
                {
                    shell.publish(on_escape.clone());
                }

                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyReleased { .. }) => {
                shell.capture_event();
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

impl<'a, Message, Theme, Renderer> From<FocusTrap<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(
        trap: FocusTrap<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(trap)
    }
}
