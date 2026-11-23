//! Text widgets display information through writing.
//!
//! # Example
//! ```no_run
//! # mod iced { pub mod widget { pub fn text<T>(t: T) -> iced_core::widget::Text<'static, iced_core::Theme, ()> { unimplemented!() } }
//! #            pub use iced_core::color; }
//! # pub type State = ();
//! # pub type Element<'a, Message> = iced_core::Element<'a, Message, iced_core::Theme, ()>;
//! use iced::widget::text;
//! use iced::color;
//!
//! enum Message {
//!     // ...
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     text("Hello, this is iced!")
//!         .size(20)
//!         .color(color!(0x0000ff))
//!         .into()
//! }
//! ```
use crate::alignment;
use crate::clipboard::{self, Clipboard};
use crate::keyboard;
use crate::layout;
use crate::mouse;
use crate::renderer;
use crate::text;
use crate::text::paragraph::{self, Paragraph};
use crate::touch;
use crate::widget::operation::accessible::{self, Accessible};
use crate::widget::tree::{self, Tree};
use crate::{
    Color, Element, Event, Layout, Length, Pixels, Point, Rectangle, Shell,
    Size, Theme, Vector, Widget,
};

pub use text::{Alignment, LineHeight, Shaping, Wrapping};

pub use crate::widget::operation::accessible::Live;

/// A bunch of text.
///
/// # Example
/// ```no_run
/// # mod iced { pub mod widget { pub fn text<T>(t: T) -> iced_core::widget::Text<'static, iced_core::Theme, ()> { unimplemented!() } }
/// #            pub use iced_core::color; }
/// # pub type State = ();
/// # pub type Element<'a, Message> = iced_core::Element<'a, Message, iced_core::Theme, ()>;
/// use iced::widget::text;
/// use iced::color;
///
/// enum Message {
///     // ...
/// }
///
/// fn view(state: &State) -> Element<'_, Message> {
///     text("Hello, this is iced!")
///         .size(20)
///         .color(color!(0x0000ff))
///         .into()
/// }
/// ```
pub struct Text<'a, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fragment: text::Fragment<'a>,
    format: Format<Renderer::Font>,
    class: Theme::Class<'a>,
    selectable: bool,
    selection_color: Option<Color>,
    live: Option<Live>,
}

impl<'a, Theme, Renderer> Text<'a, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    /// Create a new fragment of [`Text`] with the given contents.
    pub fn new(fragment: impl text::IntoFragment<'a>) -> Self {
        Text {
            fragment: fragment.into_fragment(),
            format: Format::default(),
            class: Theme::default(),
            selectable: false,
            selection_color: None,
            live: None,
        }
    }

    /// Makes the [`Text`] a status line for assistive technology: a screen
    /// reader says its words when they change, without being asked
    /// ("Up to date", "Waiting for approval"). [`Live::Polite`] waits for
    /// the reader to finish what it is saying, [`Live::Assertive`] does
    /// not.
    ///
    /// By default a [`Text`] is said only when a person reads it.
    pub fn live(mut self, live: impl Into<Option<Live>>) -> Self {
        self.live = live.into();
        self
    }

    /// Sets whether the [`Text`] can be selected with the pointer and copied
    /// with the copy key of the platform (Ctrl+C, or Cmd+C on macOS).
    ///
    /// A drag selects, a double click selects a word, a triple click selects
    /// everything; a click elsewhere drops the selection. On X11 and Wayland
    /// the selection is also put in the primary clipboard.
    ///
    /// By default, it cannot.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Sets the [`Color`] drawn behind the selected part of a selectable
    /// [`Text`].
    ///
    /// By default, it is the color of the text at a quarter of its opacity.
    pub fn selection_color(mut self, color: impl Into<Color>) -> Self {
        self.selection_color = Some(color.into());
        self
    }

    /// Sets the size of the [`Text`].
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.format.size = Some(size.into());
        self
    }

    /// Sets the [`LineHeight`] of the [`Text`].
    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.format.line_height = line_height.into();
        self
    }

    /// Sets the [`Font`] of the [`Text`].
    ///
    /// [`Font`]: crate::text::Renderer::Font
    pub fn font(mut self, font: impl Into<Renderer::Font>) -> Self {
        self.format.font = Some(font.into());
        self
    }

    /// Sets the [`Font`] of the [`Text`], if `Some`.
    ///
    /// [`Font`]: crate::text::Renderer::Font
    pub fn font_maybe(
        mut self,
        font: Option<impl Into<Renderer::Font>>,
    ) -> Self {
        self.format.font = font.map(Into::into);
        self
    }

    /// Sets the width of the [`Text`] boundaries.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.format.width = width.into();
        self
    }

    /// Sets the height of the [`Text`] boundaries.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.format.height = height.into();
        self
    }

    /// Centers the [`Text`], both horizontally and vertically.
    pub fn center(self) -> Self {
        self.align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
    }

    /// Sets the [`alignment::Horizontal`] of the [`Text`].
    pub fn align_x(mut self, alignment: impl Into<text::Alignment>) -> Self {
        self.format.align_x = alignment.into();
        self
    }

    /// Sets the [`alignment::Vertical`] of the [`Text`].
    pub fn align_y(
        mut self,
        alignment: impl Into<alignment::Vertical>,
    ) -> Self {
        self.format.align_y = alignment.into();
        self
    }

    /// Sets the [`Shaping`] strategy of the [`Text`].
    pub fn shaping(mut self, shaping: Shaping) -> Self {
        self.format.shaping = shaping;
        self
    }

    /// Sets the [`Wrapping`] strategy of the [`Text`].
    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.format.wrapping = wrapping;
        self
    }

    /// Sets the style of the [`Text`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the [`Color`] of the [`Text`].
    pub fn color(self, color: impl Into<Color>) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.color_maybe(Some(color))
    }

    /// Sets the [`Color`] of the [`Text`], if `Some`.
    pub fn color_maybe(self, color: Option<impl Into<Color>>) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        let color = color.map(Into::into);

        self.style(move |_theme| Style { color })
    }

    /// Sets the style class of the [`Text`].
    #[cfg(feature = "advanced")]
    #[must_use]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
}

/// The internal state of a [`Text`] widget.
pub type State<P> = paragraph::Plain<P>;

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Text<'_, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(paragraph::Plain::<Renderer::Paragraph>::default())
    }

    // The selection of a selectable text lives in a child of its tree, so
    // that the state of every text stays the plain paragraph it was.
    fn children(&self) -> Vec<Tree> {
        if self.selectable {
            vec![Selection::tree()]
        } else {
            Vec::new()
        }
    }

    fn diff(&self, tree: &mut Tree) {
        if !self.selectable {
            tree.children.clear();
        } else if tree.children.len() != 1
            || tree.children[0].tag != tree::Tag::of::<Selection>()
        {
            tree.children = vec![Selection::tree()];
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if !self.selectable {
            return;
        }

        let Some(selection) = tree.children.first_mut() else {
            return;
        };

        let paragraph = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let selection = selection.state.downcast_mut::<Selection>();
        let content: &str = &self.fragment;
        let bounds = layout.bounds();

        // Where the paragraph is drawn
        let origin = bounds.anchor(
            paragraph.min_bounds(),
            paragraph.align_x(),
            paragraph.align_y(),
        );

        let offset_at = |position: Point| {
            paragraph
                .raw()
                .hit_offset(position - Vector::new(origin.x, origin.y))
                .map(|offset| floor_char_boundary(content, offset))
        };

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                let before = selection.range();

                if let Some(position) = cursor.position_over(bounds)
                    && let Some(offset) = offset_at(position)
                {
                    let click = mouse::Click::new(
                        position,
                        mouse::Button::Left,
                        selection.last_click,
                    );

                    match click.kind() {
                        mouse::click::Kind::Single => {
                            selection.anchor = offset;
                            selection.head = offset;
                            selection.is_dragging = true;
                        }
                        mouse::click::Kind::Double => {
                            let (start, end) = word_around(content, offset);

                            selection.anchor = start;
                            selection.head = end;
                            selection.is_dragging = false;
                        }
                        mouse::click::Kind::Triple => {
                            selection.anchor = 0;
                            selection.head = content.len();
                            selection.is_dragging = false;
                        }
                    }

                    selection.last_click = Some(click);
                } else {
                    // A click elsewhere drops the selection
                    selection.anchor = 0;
                    selection.head = 0;
                    selection.is_dragging = false;
                    selection.last_click = None;
                }

                if selection.range() != before {
                    if !selection.is_dragging
                        && let Some(selected) = selection.selected(content)
                    {
                        clipboard.write(
                            clipboard::Kind::Primary,
                            selected.to_owned(),
                        );
                    }

                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. })
                if selection.is_dragging =>
            {
                if let Some(position) = cursor.position()
                    && let Some(offset) = offset_at(position)
                    && offset != selection.head
                {
                    selection.head = offset;
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(
                touch::Event::FingerLifted { .. }
                | touch::Event::FingerLost { .. },
            ) if selection.is_dragging => {
                selection.is_dragging = false;

                if let Some(selected) = selection.selected(content) {
                    clipboard
                        .write(clipboard::Kind::Primary, selected.to_owned());
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) if modifiers.command()
                && !shell.is_event_captured()
                && key.to_latin(*physical_key) == Some('c') =>
            {
                if let Some(selected) = selection.selected(content) {
                    clipboard
                        .write(clipboard::Kind::Standard, selected.to_owned());

                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.selectable && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.format.width,
            height: self.format.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout(
            tree.state.downcast_mut::<State<Renderer::Paragraph>>(),
            renderer,
            limits,
            &self.fragment,
            self.format,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor_position: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let style = theme.style(&self.class);

        if self.selectable
            && let Some(selection) = tree.children.first()
            && let Some((start, end)) =
                selection.state.downcast_ref::<Selection>().range()
        {
            let bounds = layout.bounds();
            let paragraph = state.raw();

            let origin = bounds.anchor(
                paragraph.min_bounds(),
                paragraph.align_x(),
                paragraph.align_y(),
            );

            let color = self.selection_color.unwrap_or_else(|| {
                style.color.unwrap_or(defaults.text_color).scale_alpha(0.25)
            });

            for line in paragraph.selection_bounds(start, end) {
                let line = line + Vector::new(origin.x, origin.y);

                if let Some(visible) = line.intersection(viewport) {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: visible,
                            ..renderer::Quad::default()
                        },
                        color,
                    );
                }
            }
        }

        draw(
            renderer,
            defaults,
            layout.bounds(),
            state.raw(),
            style,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn super::Operation,
    ) {
        // What is selected in it, if anything is
        let selection = tree
            .children
            .first()
            .filter(|_| self.selectable)
            .map(|selection| selection.state.downcast_ref::<Selection>())
            .filter(|selection| selection.anchor != selection.head)
            .map(|selection| accessible::Selection {
                anchor: selection.anchor.min(self.fragment.len()),
                focus: selection.head.min(self.fragment.len()),
            });

        // A text that has more to tell than its words describes itself
        if self.live.is_some() || selection.is_some() {
            operation.accessible(
                None,
                layout.bounds(),
                Accessible::new(accessible::Role::Text)
                    .label(&*self.fragment)
                    .selection(selection)
                    .live(self.live),
            );
        }

        operation.text(None, layout.bounds(), &self.fragment);
    }
}

/// The format of some [`Text`].
///
/// Check out the methods of the [`Text`] widget
/// to learn more about each field.
/// The selection of a selectable [`Text`]: two offsets in bytes of its
/// content, where the drag began and where it is now.
#[derive(Debug, Clone, Copy, Default)]
struct Selection {
    anchor: usize,
    head: usize,
    is_dragging: bool,
    last_click: Option<mouse::Click>,
}

impl Selection {
    fn tree() -> Tree {
        Tree {
            tag: tree::Tag::of::<Selection>(),
            state: tree::State::new(Selection::default()),
            children: Vec::new(),
        }
    }

    /// The selected range, in order; `None` when nothing is selected.
    fn range(&self) -> Option<(usize, usize)> {
        let start = self.anchor.min(self.head);
        let end = self.anchor.max(self.head);

        (start < end).then_some((start, end))
    }

    /// The selected part of the given content, if any. A selection that no
    /// longer fits the content (it changed since) selects nothing.
    fn selected<'a>(&self, content: &'a str) -> Option<&'a str> {
        let (start, end) = self.range()?;

        content.get(start..end.min(content.len()))
    }
}

/// The nearest boundary between two characters at or before the offset.
fn floor_char_boundary(content: &str, offset: usize) -> usize {
    let mut offset = offset.min(content.len());

    while !content.is_char_boundary(offset) {
        offset -= 1;
    }

    offset
}

/// The word (a run of letters, digits and `_`), or else the single character,
/// around the given offset of the content.
fn word_around(content: &str, offset: usize) -> (usize, usize) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';

    let after = content[offset..].chars().next();
    let before = content[..offset].chars().next_back();

    match (before, after) {
        (_, Some(c)) if is_word(c) => {}
        (Some(c), _) if is_word(c) => {}
        (_, Some(c)) => return (offset, offset + c.len_utf8()),
        (Some(c), None) => return (offset - c.len_utf8(), offset),
        (None, None) => return (offset, offset),
    }

    let start = content[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(offset, |(i, _)| i);

    let end = content[offset..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(content.len(), |(i, _)| offset + i);

    (start, end)
}

#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct Format<Font> {
    pub width: Length,
    pub height: Length,
    pub size: Option<Pixels>,
    pub font: Option<Font>,
    pub line_height: LineHeight,
    pub align_x: text::Alignment,
    pub align_y: alignment::Vertical,
    pub shaping: Shaping,
    pub wrapping: Wrapping,
}

impl<Font> Default for Format<Font> {
    fn default() -> Self {
        Self {
            size: None,
            line_height: LineHeight::default(),
            font: None,
            width: Length::Shrink,
            height: Length::Shrink,
            align_x: text::Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: Shaping::default(),
            wrapping: Wrapping::default(),
        }
    }
}

/// Produces the [`layout::Node`] of a [`Text`] widget.
pub fn layout<Renderer>(
    paragraph: &mut paragraph::Plain<Renderer::Paragraph>,
    renderer: &Renderer,
    limits: &layout::Limits,
    content: &str,
    format: Format<Renderer::Font>,
) -> layout::Node
where
    Renderer: text::Renderer,
{
    layout::sized(limits, format.width, format.height, |limits| {
        let bounds = limits.max();

        let size = format.size.unwrap_or_else(|| renderer.default_size());
        let font = format.font.unwrap_or_else(|| renderer.default_font());

        let _ = paragraph.update(text::Text {
            content,
            bounds,
            size,
            line_height: format.line_height,
            font,
            align_x: format.align_x,
            align_y: format.align_y,
            shaping: format.shaping,
            wrapping: format.wrapping,
        });

        paragraph.min_bounds()
    })
}

/// Draws text using the same logic as the [`Text`] widget.
pub fn draw<Renderer>(
    renderer: &mut Renderer,
    style: &renderer::Style,
    bounds: Rectangle,
    paragraph: &Renderer::Paragraph,
    appearance: Style,
    viewport: &Rectangle,
) where
    Renderer: text::Renderer,
{
    let anchor = bounds.anchor(
        paragraph.min_bounds(),
        paragraph.align_x(),
        paragraph.align_y(),
    );

    renderer.fill_paragraph(
        paragraph,
        anchor,
        appearance.color.unwrap_or(style.text_color),
        *viewport,
    );
}

impl<'a, Message, Theme, Renderer> From<Text<'a, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(
        text: Text<'a, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(text)
    }
}

impl<'a, Theme, Renderer> From<&'a str> for Text<'a, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: text::Renderer,
{
    fn from(content: &'a str) -> Self {
        Self::new(content)
    }
}

impl<'a, Message, Theme, Renderer> From<&'a str>
    for Element<'a, Message, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(content: &'a str) -> Self {
        Text::from(content).into()
    }
}

/// The appearance of some text.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    /// The [`Color`] of the text.
    ///
    /// The default, `None`, means using the inherited color.
    pub color: Option<Color>,
}

/// The theme catalog of a [`Text`].
pub trait Catalog: Sized {
    /// The item class of this [`Catalog`].
    type Class<'a>;

    /// The default class produced by this [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, item: &Self::Class<'_>) -> Style;
}

/// A styling function for a [`Text`].
///
/// This is just a boxed closure: `Fn(&Theme, Status) -> Style`.
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(|_theme| Style::default())
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

/// The default text styling; color is inherited.
pub fn default(_theme: &Theme) -> Style {
    Style { color: None }
}

/// Text with the default base color.
pub fn base(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().text),
    }
}

/// Text conveying some important information, like an action.
pub fn primary(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().primary),
    }
}

/// Text conveying some secondary information, like a footnote.
pub fn secondary(theme: &Theme) -> Style {
    Style {
        color: Some(theme.extended_palette().secondary.base.color),
    }
}

/// Text conveying some positive information, like a successful event.
pub fn success(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().success),
    }
}

/// Text conveying some mildly negative information, like a warning.
pub fn warning(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().warning),
    }
}

/// Text conveying some negative information, like an error.
pub fn danger(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().danger),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_found_around_an_offset() {
        let content = "The server_1 cannot, alas, be reached";

        assert_eq!(word_around(content, 0), (0, 3));
        assert_eq!(word_around(content, 2), (0, 3));
        // At the end of a word, the word before
        assert_eq!(word_around(content, 3), (0, 3));
        assert_eq!(&content[4..12], "server_1");
        assert_eq!(word_around(content, 7), (4, 12));
        // On a character that is not of a word, with none before: itself
        assert_eq!(word_around(", ,", 0), (0, 1));
        assert_eq!(word_around(content, content.len()), (30, 37));
        assert_eq!(word_around("", 0), (0, 0));
    }

    #[test]
    fn a_word_is_made_of_whole_characters() {
        let content = "naïve café";

        assert_eq!(word_around(content, 1), (0, 6));
        assert_eq!(&content[7..], "café");
        assert_eq!(word_around(content, 8), (7, content.len()));
    }

    #[test]
    fn an_offset_is_moved_back_to_a_character() {
        let content = "aé";

        assert_eq!(floor_char_boundary(content, 2), 1);
        assert_eq!(floor_char_boundary(content, 3), 3);
        assert_eq!(floor_char_boundary(content, 99), 3);
    }

    #[test]
    fn a_selection_that_no_longer_fits_selects_nothing_wrong() {
        let selection = Selection {
            anchor: 9,
            head: 2,
            ..Selection::default()
        };

        assert_eq!(selection.range(), Some((2, 9)));
        assert_eq!(selection.selected("0123456789"), Some("2345678"));
        // The content became shorter: what is left of the range
        assert_eq!(selection.selected("0123"), Some("23"));
        // The range would cut a character in two: nothing
        assert_eq!(selection.selected("0é"), None);
        assert_eq!(Selection::default().selected("0123"), None);
    }
}
