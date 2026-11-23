//! Checkboxes can be used to let users make binary choices.
//!
//! # Example
//! ```no_run
//! # mod iced { pub mod widget { pub use iced_widget::*; } pub use iced_widget::Renderer; pub use iced_widget::core::*; }
//! # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
//! #
//! use iced::widget::checkbox;
//!
//! struct State {
//!    is_checked: bool,
//! }
//!
//! enum Message {
//!     CheckboxToggled(bool),
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     checkbox(state.is_checked)
//!         .label("Toggle me!")
//!         .on_toggle(Message::CheckboxToggled)
//!         .into()
//! }
//!
//! fn update(state: &mut State, message: Message) {
//!     match message {
//!         Message::CheckboxToggled(is_checked) => {
//!             state.is_checked = is_checked;
//!         }
//!     }
//! }
//! ```
//! ![Checkbox drawn by `iced_wgpu`](https://github.com/iced-rs/iced/blob/7760618fb112074bc40b148944521f312152012a/docs/images/checkbox.png?raw=true)
use crate::core::alignment;
use crate::core::border;
use crate::core::keyboard;
use crate::core::layout;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::text;
use crate::core::theme::palette;
use crate::core::touch;
use crate::core::widget;
use crate::core::widget::operation;
use crate::core::widget::tree::{self, Tree};
use crate::core::window;
use crate::core::{
    Background, Border, Clipboard, Color, Element, Event, Layout, Length,
    Pixels, Rectangle, Shell, Size, Theme, Widget,
};

/// A box that can be checked.
///
/// # Example
/// ```no_run
/// # mod iced { pub mod widget { pub use iced_widget::*; } pub use iced_widget::Renderer; pub use iced_widget::core::*; }
/// # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
/// #
/// use iced::widget::checkbox;
///
/// struct State {
///    is_checked: bool,
/// }
///
/// enum Message {
///     CheckboxToggled(bool),
/// }
///
/// fn view(state: &State) -> Element<'_, Message> {
///     checkbox(state.is_checked)
///         .label("Toggle me!")
///         .on_toggle(Message::CheckboxToggled)
///         .into()
/// }
///
/// fn update(state: &mut State, message: Message) {
///     match message {
///         Message::CheckboxToggled(is_checked) => {
///             state.is_checked = is_checked;
///         }
///     }
/// }
/// ```
/// ![Checkbox drawn by `iced_wgpu`](https://github.com/iced-rs/iced/blob/7760618fb112074bc40b148944521f312152012a/docs/images/checkbox.png?raw=true)
pub struct Checkbox<
    'a,
    Message,
    Theme = crate::Theme,
    Renderer = crate::Renderer,
> where
    Renderer: text::Renderer,
    Theme: Catalog,
{
    state: State,
    on_toggle: Option<Box<dyn Fn(bool) -> Message + 'a>>,
    label: Option<text::Fragment<'a>>,
    width: Length,
    size: f32,
    spacing: f32,
    text_size: Option<Pixels>,
    text_line_height: text::LineHeight,
    text_shaping: text::Shaping,
    text_wrapping: text::Wrapping,
    font: Option<Renderer::Font>,
    icon: Icon<Renderer::Font>,
    indeterminate_icon: Option<Icon<Renderer::Font>>,
    class: Theme::Class<'a>,
    last_status: Option<Status>,
    id: Option<widget::Id>,
}

impl<'a, Message, Theme, Renderer> Checkbox<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer,
    Theme: Catalog,
{
    /// The default size of a [`Checkbox`].
    const DEFAULT_SIZE: f32 = 16.0;

    /// Creates a new [`Checkbox`].
    ///
    /// It expects:
    ///   * the [`State`] of the [`Checkbox`]; a boolean describing whether
    ///     it is checked or not converts into one
    pub fn new(state: impl Into<State>) -> Self {
        Checkbox {
            state: state.into(),
            on_toggle: None,
            label: None,
            width: Length::Shrink,
            size: Self::DEFAULT_SIZE,
            spacing: Self::DEFAULT_SIZE / 2.0,
            text_size: None,
            text_line_height: text::LineHeight::default(),
            text_shaping: text::Shaping::default(),
            text_wrapping: text::Wrapping::default(),
            font: None,
            icon: Icon {
                font: Renderer::ICON_FONT,
                code_point: Renderer::CHECKMARK_ICON,
                size: None,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
            },
            indeterminate_icon: None,
            class: Theme::default(),
            last_status: None,
            id: None,
        }
    }

    /// Sets the [`widget::Id`] of the [`Checkbox`].
    ///
    /// The identifier can be used to focus it with
    /// [`operation::focus`](crate::core::widget::operation::focusable::focus).
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the label of the [`Checkbox`].
    pub fn label(mut self, label: impl text::IntoFragment<'a>) -> Self {
        self.label = Some(label.into_fragment());
        self
    }

    /// Sets the function that will be called when the [`Checkbox`] is toggled.
    /// It will receive the new state of the [`Checkbox`] and must produce a
    /// `Message`.
    ///
    /// Unless `on_toggle` is called, the [`Checkbox`] will be disabled.
    pub fn on_toggle<F>(mut self, f: F) -> Self
    where
        F: 'a + Fn(bool) -> Message,
    {
        self.on_toggle = Some(Box::new(f));
        self
    }

    /// Sets the function that will be called when the [`Checkbox`] is toggled,
    /// if `Some`.
    ///
    /// If `None`, the checkbox will be disabled.
    pub fn on_toggle_maybe<F>(mut self, f: Option<F>) -> Self
    where
        F: Fn(bool) -> Message + 'a,
    {
        self.on_toggle = f.map(|f| Box::new(f) as _);
        self
    }

    /// Sets the size of the [`Checkbox`].
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = size.into().0;
        self
    }

    /// Sets the width of the [`Checkbox`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the spacing between the [`Checkbox`] and the text.
    pub fn spacing(mut self, spacing: impl Into<Pixels>) -> Self {
        self.spacing = spacing.into().0;
        self
    }

    /// Sets the text size of the [`Checkbox`].
    pub fn text_size(mut self, text_size: impl Into<Pixels>) -> Self {
        self.text_size = Some(text_size.into());
        self
    }

    /// Sets the text [`text::LineHeight`] of the [`Checkbox`].
    pub fn text_line_height(
        mut self,
        line_height: impl Into<text::LineHeight>,
    ) -> Self {
        self.text_line_height = line_height.into();
        self
    }

    /// Sets the [`text::Shaping`] strategy of the [`Checkbox`].
    pub fn text_shaping(mut self, shaping: text::Shaping) -> Self {
        self.text_shaping = shaping;
        self
    }

    /// Sets the [`text::Wrapping`] strategy of the [`Checkbox`].
    pub fn text_wrapping(mut self, wrapping: text::Wrapping) -> Self {
        self.text_wrapping = wrapping;
        self
    }

    /// Sets the [`Renderer::Font`] of the text of the [`Checkbox`].
    ///
    /// [`Renderer::Font`]: crate::core::text::Renderer
    pub fn font(mut self, font: impl Into<Renderer::Font>) -> Self {
        self.font = Some(font.into());
        self
    }

    /// Sets the [`Icon`] of the [`Checkbox`].
    pub fn icon(mut self, icon: Icon<Renderer::Font>) -> Self {
        self.icon = icon;
        self
    }

    /// Sets the [`Icon`] of a [`Checkbox`] that is
    /// [`State::Indeterminate`].
    ///
    /// By default, a dash is drawn.
    pub fn indeterminate_icon(mut self, icon: Icon<Renderer::Font>) -> Self {
        self.indeterminate_icon = Some(icon);
        self
    }

    /// Sets the style of the [`Checkbox`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style class of the [`Checkbox`].
    #[cfg(feature = "advanced")]
    #[must_use]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
}

/// What a [`Checkbox`] keeps between frames: its label and whether it has the
/// keyboard focus.
struct Internal<P: text::Paragraph> {
    is_focused: bool,
    text: widget::text::State<P>,
}

impl<P: text::Paragraph> Default for Internal<P> {
    fn default() -> Self {
        Self {
            is_focused: false,
            text: widget::text::State::default(),
        }
    }
}

impl<P: text::Paragraph> operation::Focusable for Internal<P> {
    fn is_focused(&self) -> bool {
        self.is_focused
    }

    fn focus(&mut self) {
        self.is_focused = true;
    }

    fn unfocus(&mut self) {
        self.is_focused = false;
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Checkbox<'_, Message, Theme, Renderer>
where
    Renderer: text::Renderer,
    Theme: Catalog,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Internal<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Internal::<Renderer::Paragraph>::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Shrink,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::next_to_each_other(
            &limits.width(self.width),
            if self.label.is_some() {
                self.spacing
            } else {
                0.0
            },
            |_| layout::Node::new(Size::new(self.size, self.size)),
            |limits| {
                if let Some(label) = self.label.as_deref() {
                    let state = tree
                        .state
                        .downcast_mut::<Internal<Renderer::Paragraph>>();

                    widget::text::layout(
                        &mut state.text,
                        renderer,
                        limits,
                        label,
                        widget::text::Format {
                            width: self.width,
                            height: Length::Shrink,
                            line_height: self.text_line_height,
                            size: self.text_size,
                            font: self.font,
                            align_x: text::Alignment::Default,
                            align_y: alignment::Vertical::Top,
                            shaping: self.text_shaping,
                            wrapping: self.text_wrapping,
                        },
                    )
                } else {
                    layout::Node::new(Size::ZERO)
                }
            },
        )
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        // A click elsewhere takes the keyboard focus away, whoever
        // captured it
        if let Event::Mouse(mouse::Event::ButtonPressed(_))
        | Event::Touch(touch::Event::FingerPressed { .. }) = event
        {
            let state =
                tree.state.downcast_mut::<Internal<Renderer::Paragraph>>();

            if state.is_focused && !cursor.is_over(layout.bounds()) {
                state.is_focused = false;
                shell.request_redraw();
            }
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                let mouse_over = cursor.is_over(layout.bounds());

                if mouse_over && let Some(on_toggle) = &self.on_toggle {
                    shell.publish((on_toggle)(!self.state.is_checked()));
                    shell.capture_event();
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key:
                    keyboard::Key::Named(
                        keyboard::key::Named::Enter
                        | keyboard::key::Named::Space,
                    ),
                repeat: false,
                ..
            }) => {
                let state =
                    tree.state.downcast_ref::<Internal<Renderer::Paragraph>>();

                if state.is_focused && !shell.is_event_captured() {
                    if let Some(on_toggle) = &self.on_toggle {
                        shell.publish((on_toggle)(!self.state.is_checked()));
                        shell.capture_event();
                    }
                }
            }
            _ => {}
        }

        let current_status = {
            let is_mouse_over = cursor.is_over(layout.bounds());
            let is_disabled = self.on_toggle.is_none();
            let state = self.state;
            let is_focused = tree
                .state
                .downcast_ref::<Internal<Renderer::Paragraph>>()
                .is_focused;

            if is_disabled {
                Status::Disabled { state }
            } else if is_focused {
                Status::Focused { state }
            } else if is_mouse_over {
                Status::Hovered { state }
            } else {
                Status::Active { state }
            }
        };

        if let Event::Window(window::Event::RedrawRequested(_now)) = event {
            self.last_status = Some(current_status);
        } else if self
            .last_status
            .is_some_and(|status| status != current_status)
        {
            shell.request_redraw();
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
        if cursor.is_over(layout.bounds()) && self.on_toggle.is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let mut children = layout.children();

        let style = theme.style(
            &self.class,
            self.last_status
                .unwrap_or(Status::Disabled { state: self.state }),
        );

        {
            let layout = children.next().unwrap();
            let bounds = layout.bounds();

            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style.border,
                    ..renderer::Quad::default()
                },
                style.background,
            );

            let icon = match self.state {
                State::Unchecked => None,
                State::Checked => Some(&self.icon),
                State::Indeterminate => self.indeterminate_icon.as_ref(),
            };

            if let Some(Icon {
                font,
                code_point,
                size,
                line_height,
                shaping,
            }) = icon
            {
                let size = size.unwrap_or(Pixels(bounds.height * 0.7));

                renderer.fill_text(
                    text::Text {
                        content: code_point.to_string(),
                        font: *font,
                        size,
                        line_height: *line_height,
                        bounds: bounds.size(),
                        align_x: text::Alignment::Center,
                        align_y: alignment::Vertical::Center,
                        shaping: *shaping,
                        wrapping: text::Wrapping::default(),
                    },
                    bounds.center(),
                    style.icon_color,
                    *viewport,
                );
            } else if self.state == State::Indeterminate {
                // The dash: a bar across the middle of the box
                let width = (bounds.width * 0.5).round();
                let height = (bounds.height * 0.125).round().max(1.0);

                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x
                                + ((bounds.width - width) / 2.0).round(),
                            y: bounds.y
                                + ((bounds.height - height) / 2.0).round(),
                            width,
                            height,
                        },
                        border: border::rounded(height / 2.0),
                        ..renderer::Quad::default()
                    },
                    style.icon_color,
                );
            }
        }

        if self.label.is_none() {
            return;
        }

        {
            let label_layout = children.next().unwrap();
            let state: &Internal<Renderer::Paragraph> =
                tree.state.downcast_ref();

            crate::text::draw(
                renderer,
                defaults,
                label_layout.bounds(),
                state.text.raw(),
                crate::text::Style {
                    color: style.text_color,
                },
                viewport,
            );
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<Internal<Renderer::Paragraph>>();

        if self.on_toggle.is_some() {
            operation.focusable(self.id.as_ref(), layout.bounds(), state);
        } else {
            // A disabled check box gives up its focus
            state.is_focused = false;
        }

        if let Some(label) = self.label.as_deref() {
            operation.text(None, layout.bounds(), label);
        }
    }
}

impl<'a, Message, Theme, Renderer> From<Checkbox<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a + Catalog,
    Renderer: 'a + text::Renderer,
{
    fn from(
        checkbox: Checkbox<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(checkbox)
    }
}

/// The icon in a [`Checkbox`].
#[derive(Debug, Clone, PartialEq)]
pub struct Icon<Font> {
    /// Font that will be used to display the `code_point`,
    pub font: Font,
    /// The unicode code point that will be used as the icon.
    pub code_point: char,
    /// Font size of the content.
    pub size: Option<Pixels>,
    /// The line height of the icon.
    pub line_height: text::LineHeight,
    /// The shaping strategy of the icon.
    pub shaping: text::Shaping,
}

/// The state of a [`Checkbox`].
///
/// A `bool` converts into it: `true` is [`State::Checked`], `false` is
/// [`State::Unchecked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum State {
    /// The [`Checkbox`] is not ticked.
    #[default]
    Unchecked,
    /// The [`Checkbox`] is ticked.
    Checked,
    /// The [`Checkbox`] is ticked "in part": it stands for several things
    /// of which some are ticked (a folder, "select all"). A dash is drawn.
    ///
    /// A click on it asks for `true`.
    Indeterminate,
}

impl State {
    /// Returns whether the [`State`] is [`State::Checked`].
    pub fn is_checked(self) -> bool {
        self == State::Checked
    }

    /// Returns whether the [`State`] is [`State::Indeterminate`].
    pub fn is_indeterminate(self) -> bool {
        self == State::Indeterminate
    }

    /// Returns the [`State`] of something made of parts: all of them ticked
    /// is [`State::Checked`], none is [`State::Unchecked`], some is
    /// [`State::Indeterminate`].
    pub fn of_parts(ticked: usize, total: usize) -> Self {
        if ticked == 0 {
            State::Unchecked
        } else if ticked >= total {
            State::Checked
        } else {
            State::Indeterminate
        }
    }
}

impl From<bool> for State {
    fn from(is_checked: bool) -> Self {
        if is_checked {
            State::Checked
        } else {
            State::Unchecked
        }
    }
}

impl Status {
    /// Returns the [`State`] of the [`Checkbox`] with this [`Status`].
    pub fn state(self) -> State {
        match self {
            Status::Active { state }
            | Status::Hovered { state }
            | Status::Disabled { state }
            | Status::Focused { state } => state,
        }
    }

    /// Returns whether the [`Checkbox`] with this [`Status`] is checked.
    pub fn is_checked(self) -> bool {
        self.state().is_checked()
    }
}

/// The possible status of a [`Checkbox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The [`Checkbox`] can be interacted with.
    Active {
        /// The [`State`] of the [`Checkbox`].
        state: State,
    },
    /// The [`Checkbox`] can be interacted with and it is being hovered.
    Hovered {
        /// The [`State`] of the [`Checkbox`].
        state: State,
    },
    /// The [`Checkbox`] cannot be interacted with.
    Disabled {
        /// The [`State`] of the [`Checkbox`].
        state: State,
    },
    /// The [`Checkbox`] has the keyboard focus: Space and Enter toggle it.
    ///
    /// A style is expected to draw a focus ring; see [`focus_ring`].
    Focused {
        /// The [`State`] of the [`Checkbox`].
        state: State,
    },
}

/// The style of a checkbox.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The [`Background`] of the checkbox.
    pub background: Background,
    /// The icon [`Color`] of the checkbox.
    pub icon_color: Color,
    /// The [`Border`] of the checkbox.
    pub border: Border,
    /// The text [`Color`] of the checkbox.
    pub text_color: Option<Color>,
}

/// The theme catalog of a [`Checkbox`].
pub trait Catalog: Sized {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

/// A styling function for a [`Checkbox`].
///
/// This is just a boxed closure: `Fn(&Theme, Status) -> Style`.
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(primary)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

/// A primary checkbox; denoting a main toggle.
pub fn primary(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();

    match status {
        Status::Active { state } => styled(
            palette.background.strong.color,
            palette.background.base,
            palette.primary.base.text,
            palette.primary.base,
            state,
        ),
        Status::Hovered { state } => styled(
            palette.background.strong.color,
            palette.background.weak,
            palette.primary.base.text,
            palette.primary.strong,
            state,
        ),
        Status::Focused { state } => focus_ring(
            theme,
            styled(
                palette.background.strong.color,
                palette.background.weak,
                palette.primary.base.text,
                palette.primary.strong,
                state,
            ),
        ),
        Status::Disabled { state } => styled(
            palette.background.weak.color,
            palette.background.weaker,
            palette.primary.base.text,
            palette.background.strong,
            state,
        ),
    }
}

/// A secondary checkbox; denoting a complementary toggle.
pub fn secondary(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();

    match status {
        Status::Active { state } => styled(
            palette.background.strong.color,
            palette.background.base,
            palette.background.base.text,
            palette.background.strong,
            state,
        ),
        Status::Hovered { state } => styled(
            palette.background.strong.color,
            palette.background.weak,
            palette.background.base.text,
            palette.background.strong,
            state,
        ),
        Status::Focused { state } => focus_ring(
            theme,
            styled(
                palette.background.strong.color,
                palette.background.weak,
                palette.background.base.text,
                palette.background.strong,
                state,
            ),
        ),
        Status::Disabled { state } => styled(
            palette.background.weak.color,
            palette.background.weak,
            palette.background.base.text,
            palette.background.weak,
            state,
        ),
    }
}

/// A success checkbox; denoting a positive toggle.
pub fn success(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();

    match status {
        Status::Active { state } => styled(
            palette.background.weak.color,
            palette.background.base,
            palette.success.base.text,
            palette.success.base,
            state,
        ),
        Status::Hovered { state } => styled(
            palette.background.strong.color,
            palette.background.weak,
            palette.success.base.text,
            palette.success.strong,
            state,
        ),
        Status::Focused { state } => focus_ring(
            theme,
            styled(
                palette.background.strong.color,
                palette.background.weak,
                palette.success.base.text,
                palette.success.strong,
                state,
            ),
        ),
        Status::Disabled { state } => styled(
            palette.background.weak.color,
            palette.background.weak,
            palette.success.base.text,
            palette.success.weak,
            state,
        ),
    }
}

/// A danger checkbox; denoting a negative toggle.
pub fn danger(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();

    match status {
        Status::Active { state } => styled(
            palette.background.strong.color,
            palette.background.base,
            palette.danger.base.text,
            palette.danger.base,
            state,
        ),
        Status::Hovered { state } => styled(
            palette.background.strong.color,
            palette.background.weak,
            palette.danger.base.text,
            palette.danger.strong,
            state,
        ),
        Status::Focused { state } => focus_ring(
            theme,
            styled(
                palette.background.strong.color,
                palette.background.weak,
                palette.danger.base.text,
                palette.danger.strong,
                state,
            ),
        ),
        Status::Disabled { state } => styled(
            palette.background.weak.color,
            palette.background.weak,
            palette.danger.base.text,
            palette.danger.weak,
            state,
        ),
    }
}

/// Draws the focus ring of the built-in [`Theme`] on a check box [`Style`]:
/// a border of two pixels in the color of the text of the page.
///
/// Custom styles can call this for [`Status::Focused`].
pub fn focus_ring(theme: &Theme, style: Style) -> Style {
    let palette = theme.extended_palette();

    Style {
        border: Border {
            color: palette.background.base.text,
            width: 2.0,
            radius: style.border.radius,
        },
        ..style
    }
}

fn styled(
    border_color: Color,
    base: palette::Pair,
    icon_color: Color,
    accent: palette::Pair,
    state: State,
) -> Style {
    // A box that is ticked in part is filled like one that is ticked
    let (background, border) = if state != State::Unchecked {
        (accent, accent.color)
    } else {
        (base, border_color)
    };

    Style {
        background: Background::Color(background.color),
        icon_color,
        border: Border {
            radius: 2.0.into(),
            width: 1.0,
            color: border,
        },
        text_color: None,
    }
}
