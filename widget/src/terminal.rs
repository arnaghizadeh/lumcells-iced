//! Terminals show what a program writes to a pseudo-terminal: a grid of
//! cells with colors, a cursor, the lines that scrolled away, a selection
//! that can be copied; and they turn key presses into the bytes a terminal
//! sends.
//!
//! The widget does not start anything and does not read from anywhere: the
//! application feeds the bytes it has (from a pseudo-terminal of its own, a
//! recording, a network connection) into a [`Content`] and gets the bytes
//! of the keys in a message.
//!
//! # Example
//! ```no_run
//! # mod iced { pub mod widget { pub use iced_widget::*; } pub use iced_widget::Renderer; pub use iced_widget::core::*; }
//! # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
//! #
//! use iced::widget::terminal;
//!
//! struct State {
//!    content: terminal::Content,
//! }
//!
//! #[derive(Debug, Clone)]
//! enum Message {
//!     // What the program wrote
//!     Output(Vec<u8>),
//!     // What the person typed
//!     Typed(Vec<u8>),
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     terminal(&state.content)
//!         .on_input(Message::Typed)
//!         .into()
//! }
//!
//! fn update(state: &mut State, message: Message) {
//!     match message {
//!         Message::Output(bytes) => state.content.feed(&bytes),
//!         Message::Typed(_bytes) => {
//!             // Write them to the program's input
//!         }
//!     }
//! }
//! ```
//!
//! # Keys
//! A terminal that takes input (one with [`Terminal::on_input`]) is in one
//! of two states when it has the keyboard focus. Reached with Tab it is
//! *focused*: the arrows and Page Up and Page Down scroll, Tab moves on,
//! and Enter starts typing. While *typing* (after Enter or a click) every
//! key goes to the program, Tab included; Shift+Escape stops typing.
//!
//! Ctrl+Shift+C copies the selection and Ctrl+Shift+V pastes (on macOS
//! Cmd+C and Cmd+V); while not typing Ctrl+C copies too. Shift+Page Up and
//! Shift+Page Down scroll at any time.
pub mod grid;
pub mod keys;

pub use grid::{Cell, Flags, Grid, Row};

use crate::core::alignment;
use crate::core::border::{self, Border};
use crate::core::clipboard::{self, Clipboard};
use crate::core::keyboard;
use crate::core::keyboard::key::Named;
use crate::core::layout;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::text::{self, Paragraph as _};
use crate::core::touch;
use crate::core::widget;
use crate::core::widget::operation::{self, Operation};
use crate::core::widget::tree::{self, Tree};
use crate::core::window;
use crate::core::{
    Color, Element, Event, Font, Layout, Length, Padding, Pixels, Point,
    Rectangle, Shell, Size, Theme, Widget,
};

use vte::ansi::{Processor, Timeout};

/// The lines a [`Content`] remembers after they scrolled away, unless told
/// otherwise.
pub const DEFAULT_HISTORY: usize = 10_000;

/// Synchronized updates are not held back: the bytes that are fed are on
/// the grid when `feed` returns.
#[derive(Debug, Default)]
struct Never;

impl Timeout for Never {
    fn set_timeout(&mut self, _duration: std::time::Duration) {}

    fn clear_timeout(&mut self) {}

    fn pending_timeout(&self) -> bool {
        false
    }
}

/// What a [`Terminal`] shows: the grid and the parser that fills it. It
/// lives in the state of the application.
pub struct Content {
    grid: Grid,
    parser: Processor<Never>,
}

impl Content {
    /// A terminal of `cols` by `rows` cells with the default history.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_history(cols, rows, DEFAULT_HISTORY)
    }

    /// A terminal that remembers up to `history` lines that scrolled away.
    pub fn with_history(cols: u16, rows: u16, history: usize) -> Self {
        Self {
            grid: Grid::new(cols, rows, history),
            parser: Processor::new(),
        }
    }

    /// Feeds bytes the program wrote: text and escape sequences, in any
    /// pieces (a sequence may be cut anywhere).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.grid, bytes);
    }

    /// Gives the grid another size, as when the pseudo-terminal got one.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.grid.resize(cols, rows);
    }

    /// The grid.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// The number of columns.
    pub fn cols(&self) -> u16 {
        self.grid.cols() as u16
    }

    /// The number of lines of the screen.
    pub fn rows(&self) -> u16 {
        self.grid.rows() as u16
    }

    /// Everything it holds as text: the history and the screen, a line
    /// break after every line that did not wrap.
    pub fn text(&self) -> String {
        self.grid.text(0, self.grid.used_lines())
    }

    /// The title the program set.
    pub fn title(&self) -> Option<&str> {
        self.grid.title()
    }

    /// What the terminal answers to questions of the program (see
    /// [`Grid::take_answers`]): write them to the program's input, or drop
    /// them when this terminal only watches.
    pub fn take_answers(&mut self) -> Vec<u8> {
        self.grid.take_answers()
    }
}

impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Content")
            .field("cols", &self.grid.cols())
            .field("rows", &self.grid.rows())
            .field("history", &self.grid.history())
            .finish()
    }
}

/// A terminal.
///
/// See the [module](self) for an example and for the keys.
pub struct Terminal<'a, Message, Theme = crate::Theme>
where
    Theme: Catalog,
{
    content: &'a Content,
    id: Option<widget::Id>,
    font: Font,
    size: Option<Pixels>,
    line_height: text::LineHeight,
    width: Length,
    height: Length,
    padding: Padding,
    on_input: Option<Box<dyn Fn(Vec<u8>) -> Message + 'a>>,
    on_fit: Option<Box<dyn Fn(u16, u16) -> Message + 'a>>,
    class: Theme::Class<'a>,
    last_status: Option<Status>,
}

impl<'a, Message, Theme> Terminal<'a, Message, Theme>
where
    Theme: Catalog,
{
    /// Creates a new [`Terminal`] that shows the given [`Content`].
    pub fn new(content: &'a Content) -> Self {
        Self {
            content,
            id: None,
            font: Font::MONOSPACE,
            size: None,
            line_height: text::LineHeight::Relative(1.25),
            width: Length::Fill,
            height: Length::Fill,
            padding: Padding::new(6.0),
            on_input: None,
            on_fit: None,
            class: Theme::default(),
            last_status: None,
        }
    }

    /// Sets the [`widget::Id`] of the [`Terminal`].
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the message that carries the bytes of what is typed or pasted.
    /// Without it the [`Terminal`] only shows (and scrolls, and copies).
    pub fn on_input(
        mut self,
        on_input: impl Fn(Vec<u8>) -> Message + 'a,
    ) -> Self {
        self.on_input = Some(Box::new(on_input));
        self
    }

    /// Sets the message for input when there is one (see
    /// [`on_input`](Self::on_input)).
    pub fn on_input_maybe(
        mut self,
        on_input: Option<impl Fn(Vec<u8>) -> Message + 'a>,
    ) -> Self {
        self.on_input = on_input.map(|f| Box::new(f) as _);
        self
    }

    /// Sets the message that says how many columns and lines fit in the
    /// space the [`Terminal`] has: produced when that number changes, so
    /// that the application can give the pseudo-terminal that size.
    pub fn on_fit(mut self, on_fit: impl Fn(u16, u16) -> Message + 'a) -> Self {
        self.on_fit = Some(Box::new(on_fit));
        self
    }

    /// Sets the [`Font`]; it has to be one with a fixed width.
    pub fn font(mut self, font: impl Into<Font>) -> Self {
        self.font = font.into();
        self
    }

    /// Sets the text size.
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Sets the height of a line.
    pub fn line_height(
        mut self,
        line_height: impl Into<text::LineHeight>,
    ) -> Self {
        self.line_height = line_height.into();
        self
    }

    /// Sets the width. [`Length::Shrink`] is the width of the grid's
    /// columns.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height. [`Length::Shrink`] is the height of the grid's
    /// screen.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the space around the grid.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Sets the style of the [`Terminal`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style class of the [`Terminal`].
    #[cfg(feature = "advanced")]
    #[must_use]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
}

/// A cell of everything the terminal ever showed: its line (counted from
/// the first line ever, so that it stays the same while lines scroll) and
/// its column.
type Place = (u64, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Unit {
    #[default]
    Cell,
    Word,
    Line,
}

#[derive(Debug, Default)]
struct State {
    is_focused: bool,
    is_typing: bool,
    /// Lines the view is above the end.
    scroll: usize,
    /// What `scroll` was measured against.
    seen_lines: u64,
    selection: Option<(Place, Place)>,
    dragging: Option<(Place, Place, Unit)>,
    last_click: Option<mouse::Click>,
    wheel: f32,
    cell: Size,
    measured: Option<(Font, f32, f32)>,
    fit: Option<(u16, u16)>,
}

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.is_focused
    }

    fn focus(&mut self) {
        self.is_focused = true;
    }

    fn unfocus(&mut self) {
        self.is_focused = false;
        self.is_typing = false;
    }
}

/// The lines that are shown: the index of the first one and how many.
fn window(grid: &Grid, scroll: usize, visible: usize) -> (usize, usize) {
    let used = lines(grid);
    let visible = visible.max(1);
    let scroll = scroll.min(used.saturating_sub(visible));
    let first = used.saturating_sub(visible).saturating_sub(scroll);

    (first, visible.min(used - first))
}

/// The lines there are to show: all of a full screen program's screen,
/// otherwise up to the last one in use.
fn lines(grid: &Grid) -> usize {
    if grid.alternate_screen() {
        grid.total_lines()
    } else {
        grid.used_lines()
    }
}

fn word_at(row: &Row, col: usize) -> (usize, usize) {
    let is_word = |col: usize| {
        row.cells.get(col).is_some_and(|cell| {
            !cell.c.is_whitespace() || cell.flags.contains(Flags::WIDE_SPACER)
        })
    };

    if !is_word(col) {
        return (col, col);
    }

    let mut start = col;
    let mut end = col;

    while start > 0 && is_word(start - 1) {
        start -= 1;
    }

    while is_word(end + 1) {
        end += 1;
    }

    (start, end)
}

impl<Message, Theme> Terminal<'_, Message, Theme>
where
    Theme: Catalog,
{
    fn inner(&self, bounds: Rectangle) -> Rectangle {
        bounds.shrink(self.padding)
    }

    fn visible_rows(&self, state: &State, bounds: Rectangle) -> usize {
        if state.cell.height <= 0.0 {
            return 1;
        }

        // A hair of tolerance: a height made of whole lines shows them all
        ((self.inner(bounds).height + 0.01) / state.cell.height)
            .floor()
            .max(1.0) as usize
    }

    /// The cell under a point, which may be outside the grid.
    fn place_at(
        &self,
        state: &State,
        bounds: Rectangle,
        position: Point,
    ) -> Place {
        let grid = self.content.grid();
        let inner = self.inner(bounds);
        let (first, count) =
            window(grid, state.scroll, self.visible_rows(state, bounds));
        let row = ((position.y - inner.y) / state.cell.height.max(1.0))
            .floor()
            .clamp(0.0, count.saturating_sub(1) as f32)
            as usize;
        let col = ((position.x - inner.x) / state.cell.width.max(1.0))
            .floor()
            .clamp(0.0, grid.cols().saturating_sub(1) as f32)
            as usize;

        (grid.forgotten() + (first + row) as u64, col)
    }

    fn selected_text(&self, state: &State) -> Option<String> {
        let grid = self.content.grid();
        let (start, end) = state.selection?;
        let forgotten = grid.forgotten();

        // What was forgotten in the meantime is not in it any more
        let start = if start.0 < forgotten {
            (0, 0)
        } else {
            ((start.0 - forgotten) as usize, start.1)
        };
        let end = ((end.0.checked_sub(forgotten)?) as usize, end.1);
        let text = grid.selected(start, end);

        (!text.is_empty()).then_some(text)
    }

    fn scroll_by(&self, state: &mut State, bounds: Rectangle, lines_up: isize) {
        let grid = self.content.grid();
        let max = lines(grid).saturating_sub(self.visible_rows(state, bounds));
        let now = state.scroll.min(max) as isize + lines_up;

        state.scroll = now.clamp(0, max as isize) as usize;
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Terminal<'_, Message, Theme>
where
    Theme: Catalog,
    Renderer: text::Renderer<Font = Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let line_height = self.line_height.to_absolute(size);
        let key = (self.font, size.0, line_height.0);

        if state.measured != Some(key) {
            const SAMPLE: &str = "MMMMMMMMMMMMMMMMMMMM";

            let paragraph = Renderer::Paragraph::with_text(text::Text {
                content: SAMPLE,
                bounds: Size::INFINITE,
                size,
                line_height: self.line_height,
                font: self.font,
                align_x: text::Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping: text::Shaping::Basic,
                wrapping: text::Wrapping::None,
            });

            state.cell = Size::new(
                paragraph.min_bounds().width / SAMPLE.len() as f32,
                line_height.0,
            );
            state.measured = Some(key);
        }

        let grid = self.content.grid();
        let natural = Size::new(
            grid.cols() as f32 * state.cell.width,
            grid.rows() as f32 * state.cell.height,
        )
        .expand(self.padding);

        layout::Node::new(limits.resolve(self.width, self.height, natural))
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();

        operation.focusable(self.id.as_ref(), bounds, state);

        // The lines that are shown, as text: what a test or a tool looks for
        let grid = self.content.grid();
        let inner = self.inner(bounds);
        let (first, count) =
            window(grid, state.scroll, self.visible_rows(state, bounds));

        for row in 0..count {
            let Some(line) = grid.line(first + row) else {
                break;
            };
            let content = line.text(0, usize::MAX);

            if !content.trim().is_empty() {
                operation.text(
                    None,
                    Rectangle {
                        x: inner.x,
                        y: inner.y + row as f32 * state.cell.height,
                        width: inner.width,
                        height: state.cell.height,
                    },
                    &content,
                );
            }
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
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let grid = self.content.grid();

        // A view that was scrolled up stays on the lines it shows while
        // new ones arrive below
        let now_lines = grid.forgotten() + lines(grid) as u64;

        if state.scroll > 0 && now_lines > state.seen_lines {
            state.scroll += (now_lines - state.seen_lines) as usize;
        }

        state.seen_lines = now_lines;

        if self.on_input.is_none() {
            state.is_typing = false;
        }

        // How many cells fit
        if let Some(on_fit) = &self.on_fit
            && state.cell.width > 0.0
            && state.cell.height > 0.0
        {
            let inner = self.inner(bounds);
            let fit = (
                ((inner.width + 0.01) / state.cell.width).floor().max(2.0)
                    as u16,
                ((inner.height + 0.01) / state.cell.height).floor().max(1.0)
                    as u16,
            );

            if state.fit != Some(fit) {
                state.fit = Some(fit);
                shell.publish(on_fit(fit.0, fit.1));
            }
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(button))
                if !cursor.is_over(bounds) =>
            {
                let _ = button;

                // A click elsewhere takes the focus and the selection away
                if state.is_focused || state.selection.is_some() {
                    state.is_focused = false;
                    state.is_typing = false;
                    state.selection = None;
                    state.dragging = None;
                    shell.request_redraw();
                }
            }
            Event::Touch(touch::Event::FingerPressed { .. })
                if !cursor.is_over(bounds) =>
            {
                state.is_focused = false;
                state.is_typing = false;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if shell.is_event_captured() {
                    return;
                }

                let Some(position) = cursor.position() else {
                    return;
                };

                state.is_focused = true;
                state.is_typing = self.on_input.is_some();

                let click = mouse::Click::new(
                    position,
                    mouse::Button::Left,
                    state.last_click,
                );
                let place = self.place_at(state, bounds, position);
                let index = (place.0 - grid.forgotten()) as usize;

                match click.kind() {
                    mouse::click::Kind::Single => {
                        state.selection = None;
                        state.dragging = Some((place, place, Unit::Cell));
                    }
                    mouse::click::Kind::Double => {
                        let (start, end) = grid
                            .line(index)
                            .map_or((place.1, place.1), |row| {
                                word_at(row, place.1)
                            });
                        let word = ((place.0, start), (place.0, end));

                        state.selection = Some(word);
                        state.dragging = Some((word.0, word.1, Unit::Word));
                    }
                    mouse::click::Kind::Triple => {
                        let line = ((place.0, 0), (place.0, usize::MAX));

                        state.selection = Some(line);
                        state.dragging = Some((line.0, line.1, Unit::Line));
                    }
                }

                state.last_click = Some(click);
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                let Some((from, to, unit)) = state.dragging else {
                    return;
                };
                let Some(position) = cursor.position() else {
                    return;
                };

                // Past the top or the bottom the view follows
                if position.y < bounds.y {
                    self.scroll_by(state, bounds, 1);
                } else if position.y > bounds.y + bounds.height {
                    self.scroll_by(state, bounds, -1);
                }

                let place = self.place_at(state, bounds, position);
                let index = (place.0 - grid.forgotten()) as usize;
                let head = match unit {
                    Unit::Cell => (place, place),
                    Unit::Word => {
                        let (start, end) = grid
                            .line(index)
                            .map_or((place.1, place.1), |row| {
                                word_at(row, place.1)
                            });

                        ((place.0, start), (place.0, end))
                    }
                    Unit::Line => ((place.0, 0), (place.0, usize::MAX)),
                };
                let selection = (from.min(head.0), to.max(head.1));

                if unit != Unit::Cell || place != from {
                    if state.selection != Some(selection) {
                        state.selection = Some(selection);
                        shell.request_redraw();
                    }
                } else if state.selection.take().is_some() {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(
                touch::Event::FingerLifted { .. }
                | touch::Event::FingerLost { .. },
            ) => {
                if state.dragging.take().is_some() {
                    // On X11 and Wayland what is selected is the primary
                    // selection (a middle click pastes it)
                    if let Some(selected) = self.selected_text(state) {
                        clipboard.write(clipboard::Kind::Primary, selected);
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Middle,
            )) => {
                if shell.is_event_captured() {
                    return;
                }

                if let Some(on_input) = &self.on_input
                    && let Some(text) = clipboard.read(clipboard::Kind::Primary)
                {
                    state.is_focused = true;
                    state.is_typing = true;
                    state.scroll = 0;
                    shell.publish(on_input(keys::paste(
                        &text,
                        grid.bracketed_paste(),
                    )));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.is_over(bounds) =>
            {
                if shell.is_event_captured() {
                    return;
                }

                let moved = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        y / state.cell.height.max(1.0)
                    }
                } + state.wheel;
                let whole = moved.trunc();

                state.wheel = moved - whole;

                // At either end the wheel is the page's: a terminal inside
                // something that scrolls does not hold the pointer
                if whole != 0.0 {
                    let before = state.scroll;

                    self.scroll_by(state, bounds, whole as isize);

                    if state.scroll != before {
                        shell.request_redraw();
                        shell.capture_event();
                    }
                } else {
                    shell.capture_event();
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            }) if state.is_focused => {
                if shell.is_event_captured() {
                    return;
                }

                let page = self.visible_rows(state, bounds).max(2) as isize - 1;
                let letter = match key {
                    keyboard::Key::Character(c) => Some(c.to_ascii_lowercase()),
                    _ => None,
                };
                let chord = if cfg!(target_os = "macos") {
                    modifiers.logo()
                } else {
                    modifiers.control() && modifiers.shift()
                };

                // Copy
                if letter.as_deref() == Some("c")
                    && (chord || (!state.is_typing && modifiers.command()))
                {
                    if let Some(selected) = self.selected_text(state) {
                        clipboard.write(clipboard::Kind::Standard, selected);
                    }

                    shell.capture_event();
                    return;
                }

                // Paste
                if letter.as_deref() == Some("v") && chord {
                    if let Some(on_input) = &self.on_input
                        && state.is_typing
                        && let Some(text) =
                            clipboard.read(clipboard::Kind::Standard)
                    {
                        state.scroll = 0;
                        shell.publish(on_input(keys::paste(
                            &text,
                            grid.bracketed_paste(),
                        )));
                    }

                    shell.capture_event();
                    return;
                }

                // Scrolling
                let scrolls = modifiers.shift() || !state.is_typing;
                let by = match key {
                    keyboard::Key::Named(Named::PageUp) if scrolls => {
                        Some(page)
                    }
                    keyboard::Key::Named(Named::PageDown) if scrolls => {
                        Some(-page)
                    }
                    keyboard::Key::Named(Named::ArrowUp)
                        if !state.is_typing =>
                    {
                        Some(1)
                    }
                    keyboard::Key::Named(Named::ArrowDown)
                        if !state.is_typing =>
                    {
                        Some(-1)
                    }
                    keyboard::Key::Named(Named::Home) if !state.is_typing => {
                        Some(isize::MAX / 2)
                    }
                    keyboard::Key::Named(Named::End) if !state.is_typing => {
                        Some(isize::MIN / 2)
                    }
                    _ => None,
                };

                if let Some(by) = by {
                    self.scroll_by(state, bounds, by);
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }

                if !state.is_typing {
                    match key {
                        keyboard::Key::Named(Named::Enter)
                            if self.on_input.is_some() =>
                        {
                            state.is_typing = true;
                            state.scroll = 0;
                            shell.capture_event();
                            shell.request_redraw();
                        }
                        keyboard::Key::Named(Named::Escape)
                            if state.selection.is_some() =>
                        {
                            state.selection = None;
                            shell.capture_event();
                            shell.request_redraw();
                        }
                        // Tab and everything else is not this widget's
                        _ => {}
                    }

                    return;
                }

                // Typing: Shift+Escape gives the keys back
                if *key == keyboard::Key::Named(Named::Escape)
                    && modifiers.shift()
                {
                    state.is_typing = false;
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }

                if let Some(on_input) = &self.on_input {
                    if let Some(bytes) = keys::encode(
                        key,
                        *modifiers,
                        text.as_deref(),
                        grid.application_cursor(),
                    ) {
                        state.scroll = 0;
                        state.selection = None;
                        shell.publish(on_input(bytes));
                    }

                    // While typing no key moves the focus or reaches
                    // anything else
                    shell.capture_event();
                }
            }
            _ => {}
        }

        let status = if state.is_typing {
            Status::Typing
        } else if state.is_focused {
            Status::Focused
        } else if cursor.is_over(bounds) {
            Status::Hovered
        } else {
            Status::Active
        };

        if let Event::Window(window::Event::RedrawRequested(_now)) = event {
            self.last_status = Some(status);
        } else if self.last_status.is_some_and(|last| last != status) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let status = self.last_status.unwrap_or(Status::Active);
        let style = theme.style(&self.class, status);
        let grid = self.content.grid();

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: style.border,
                ..renderer::Quad::default()
            },
            style.background,
        );

        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };

        let inner = self.inner(bounds);
        let cell = state.cell;

        if cell.width <= 0.0 || cell.height <= 0.0 {
            return;
        }

        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let (first, count) =
            window(grid, state.scroll, self.visible_rows(state, bounds));
        let forgotten = grid.forgotten();
        let max_cols =
            ((inner.width / cell.width).ceil() as usize).min(grid.cols());

        let color_of = |color: grid::Color, is_fg: bool| match color {
            grid::Color::Default => {
                if is_fg {
                    style.text
                } else {
                    style.background
                }
            }
            grid::Color::Indexed(index) => style.indexed(index),
            grid::Color::Rgb(r, g, b) => Color::from_rgb8(r, g, b),
        };

        // What a cell looks like once inverse, hidden and faint are applied
        let colors = |cell: &Cell| {
            let mut fg = color_of(cell.fg, true);
            let mut bg = color_of(cell.bg, false);
            let mut has_bg = cell.bg != grid::Color::Default;

            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
                has_bg = true;
            }

            if cell.flags.contains(Flags::DIM) {
                fg = mix(fg, bg, 0.4);
            }

            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg;
            }

            (fg, has_bg.then_some(bg))
        };

        let cursor_at = (grid.cursor_visible() && state.scroll == 0)
            .then(|| (grid.history() + grid.cursor().0, grid.cursor().1));

        renderer.with_layer(clip, |renderer| {
            let fill = |renderer: &mut Renderer,
                        row: usize,
                        from: usize,
                        to: usize,
                        color: Color| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: inner.x + from as f32 * cell.width,
                            y: inner.y + row as f32 * cell.height,
                            width: (to - from) as f32 * cell.width,
                            height: cell.height,
                        },
                        ..renderer::Quad::default()
                    },
                    color,
                );
            };

            let write = |renderer: &mut Renderer,
                         row: usize,
                         col: usize,
                         cells: usize,
                         content: String,
                         flags: Flags,
                         color: Color| {
                let mut font = self.font;

                if flags.contains(Flags::BOLD) {
                    font.weight = crate::core::font::Weight::Bold;
                }

                if flags.contains(Flags::ITALIC) {
                    font.style = crate::core::font::Style::Italic;
                }

                let position = Point::new(
                    inner.x + col as f32 * cell.width,
                    inner.y + row as f32 * cell.height,
                );
                let shaping = if content.is_ascii() {
                    text::Shaping::Basic
                } else {
                    text::Shaping::Advanced
                };

                renderer.fill_text(
                    text::Text {
                        content,
                        // Room to spare: nothing is wrapped or cut here
                        bounds: Size::new(
                            (cells + 2) as f32 * cell.width,
                            cell.height,
                        ),
                        size,
                        line_height: self.line_height,
                        font,
                        align_x: text::Alignment::Left,
                        align_y: alignment::Vertical::Top,
                        shaping,
                        wrapping: text::Wrapping::None,
                    },
                    position,
                    color,
                    clip,
                );

                for (flag, at) in
                    [(Flags::UNDERLINE, 0.92), (Flags::STRIKE, 0.55)]
                {
                    if flags.contains(flag) {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: Rectangle {
                                    x: position.x,
                                    y: (position.y + cell.height * at).round(),
                                    width: cells as f32 * cell.width,
                                    height: 1.0,
                                },
                                ..renderer::Quad::default()
                            },
                            color,
                        );
                    }
                }
            };

            for row in 0..count {
                let index = first + row;
                let Some(line) = grid.line(index) else {
                    break;
                };
                let cells = &line.cells[..line.cells.len().min(max_cols)];

                // Backgrounds, in runs of one color
                let mut col = 0;

                while col < cells.len() {
                    let Some(bg) = colors(&cells[col]).1 else {
                        col += 1;
                        continue;
                    };
                    let from = col;

                    while col < cells.len() && colors(&cells[col]).1 == Some(bg)
                    {
                        col += 1;
                    }

                    fill(renderer, row, from, col, bg);
                }

                // The selection
                if let Some((start, end)) = state.selection {
                    let line_number = forgotten + index as u64;

                    if (start.0..=end.0).contains(&line_number) {
                        let from = if line_number == start.0 {
                            start.1.min(max_cols)
                        } else {
                            0
                        };
                        let to = if line_number == end.0 {
                            end.1.saturating_add(1).min(max_cols)
                        } else {
                            max_cols
                        };

                        if to > from {
                            fill(renderer, row, from, to, style.selection);
                        }
                    }
                }

                // The text: runs of plain characters of one look are
                // written together, anything else cell by cell so that it
                // stays in its column whatever font draws it
                let used = line.len_used().min(cells.len());
                let mut col = 0;

                while col < used {
                    let cell_here = &cells[col];

                    if cell_here.c == ' '
                        || cell_here.flags.contains(Flags::WIDE_SPACER)
                    {
                        col += 1;
                        continue;
                    }

                    let look = |cell: &Cell| {
                        (
                            colors(cell).0,
                            cell.flags.contains(Flags::BOLD),
                            cell.flags.contains(Flags::ITALIC),
                            cell.flags.contains(Flags::UNDERLINE),
                            cell.flags.contains(Flags::STRIKE),
                        )
                    };
                    let here = look(cell_here);
                    let from = col;
                    let mut content = String::new();

                    if cell_here.c.is_ascii() {
                        while col < used
                            && cells[col].c.is_ascii()
                            && look(&cells[col]) == here
                        {
                            content.push(cells[col].c);
                            col += 1;
                        }

                        let trimmed = content.trim_end().len();
                        content.truncate(trimmed);
                    } else {
                        content.push(cell_here.c);
                        col += 1;
                    }

                    let width = if cell_here.flags.contains(Flags::WIDE) {
                        2
                    } else {
                        content.chars().count()
                    };

                    write(
                        renderer,
                        row,
                        from,
                        width,
                        content,
                        cell_here.flags,
                        here.0,
                    );
                }

                // The cursor: a block while typing, a frame otherwise
                if let Some((cursor_line, cursor_col)) = cursor_at
                    && cursor_line == index
                    && cursor_col < max_cols
                {
                    let under =
                        line.cells.get(cursor_col).copied().unwrap_or_default();
                    let wide = if under.flags.contains(Flags::WIDE) {
                        2
                    } else {
                        1
                    };
                    let block = Rectangle {
                        x: inner.x + cursor_col as f32 * cell.width,
                        y: inner.y + row as f32 * cell.height,
                        width: wide as f32 * cell.width,
                        height: cell.height,
                    };

                    if status == Status::Typing {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: block,
                                ..renderer::Quad::default()
                            },
                            style.cursor,
                        );

                        if under.c != ' ' {
                            write(
                                renderer,
                                row,
                                cursor_col,
                                wide,
                                under.c.to_string(),
                                under.flags,
                                style.background,
                            );
                        }
                    } else {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: block,
                                border: Border {
                                    color: style.cursor,
                                    width: 1.0,
                                    radius: 0.0.into(),
                                },
                                ..renderer::Quad::default()
                            },
                            Color::TRANSPARENT,
                        );
                    }
                }
            }
        });
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, Message, Theme, Renderer> From<Terminal<'a, Message, Theme>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: Catalog + 'a,
    Renderer: text::Renderer<Font = Font>,
{
    fn from(terminal: Terminal<'a, Message, Theme>) -> Self {
        Element::new(terminal)
    }
}

fn mix(a: Color, b: Color, amount: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * amount,
        g: a.g + (b.g - a.g) * amount,
        b: a.b + (b.b - a.b) * amount,
        a: a.a,
    }
}

/// The possible states of a [`Terminal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing is happening to it.
    Active,
    /// The pointer is over it.
    Hovered,
    /// It has the keyboard focus and scrolls with the keys; Tab moves on.
    Focused,
    /// It has the keyboard focus and the keys go to the program.
    Typing,
}

/// The appearance of a [`Terminal`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// What is behind the text.
    pub background: Color,
    /// The text that has no color of its own.
    pub text: Color,
    /// The cursor.
    pub cursor: Color,
    /// What is behind selected text.
    pub selection: Color,
    /// The sixteen colors programs name: black, red, green, yellow, blue,
    /// magenta, cyan, white, then the bright ones.
    pub palette: [Color; 16],
    /// The border around the terminal.
    pub border: Border,
}

impl Style {
    /// One of the 256 indexed colors: the palette, then the color cube of
    /// six steps a part, then 24 greys.
    pub fn indexed(&self, index: u8) -> Color {
        match index {
            0..=15 => self.palette[usize::from(index)],
            16..=231 => {
                let index = index - 16;
                let step = |n: u8| if n == 0 { 0 } else { 55 + 40 * n };

                Color::from_rgb8(
                    step(index / 36),
                    step(index / 6 % 6),
                    step(index % 6),
                )
            }
            232..=255 => {
                let grey = 8 + 10 * (index - 232);

                Color::from_rgb8(grey, grey, grey)
            }
        }
    }
}

/// The sixteen colors on a dark ground.
pub const DARK_PALETTE: [Color; 16] = [
    Color::from_rgb8(0x1d, 0x1f, 0x21),
    Color::from_rgb8(0xe0, 0x6c, 0x75),
    Color::from_rgb8(0x8c, 0xc5, 0x70),
    Color::from_rgb8(0xe5, 0xc0, 0x7b),
    Color::from_rgb8(0x61, 0xaf, 0xef),
    Color::from_rgb8(0xc6, 0x78, 0xdd),
    Color::from_rgb8(0x56, 0xb6, 0xc2),
    Color::from_rgb8(0xc8, 0xcc, 0xd4),
    Color::from_rgb8(0x6b, 0x72, 0x80),
    Color::from_rgb8(0xff, 0x8a, 0x93),
    Color::from_rgb8(0xa9, 0xe0, 0x8e),
    Color::from_rgb8(0xf5, 0xd7, 0x9a),
    Color::from_rgb8(0x8c, 0xc8, 0xff),
    Color::from_rgb8(0xdf, 0x9c, 0xf2),
    Color::from_rgb8(0x7f, 0xd4, 0xdf),
    Color::from_rgb8(0xff, 0xff, 0xff),
];

/// The sixteen colors on a light ground.
pub const LIGHT_PALETTE: [Color; 16] = [
    Color::from_rgb8(0x1d, 0x1f, 0x21),
    Color::from_rgb8(0xb4, 0x1e, 0x2e),
    Color::from_rgb8(0x1f, 0x7a, 0x3a),
    Color::from_rgb8(0x8a, 0x5a, 0x00),
    Color::from_rgb8(0x1f, 0x5f, 0xbf),
    Color::from_rgb8(0x8e, 0x35, 0xa8),
    Color::from_rgb8(0x0b, 0x72, 0x85),
    Color::from_rgb8(0x5f, 0x66, 0x73),
    Color::from_rgb8(0x4a, 0x50, 0x5c),
    Color::from_rgb8(0xd1, 0x2f, 0x42),
    Color::from_rgb8(0x25, 0x8f, 0x45),
    Color::from_rgb8(0xa1, 0x6a, 0x00),
    Color::from_rgb8(0x2a, 0x72, 0xde),
    Color::from_rgb8(0xa5, 0x45, 0xc2),
    Color::from_rgb8(0x0e, 0x86, 0x9c),
    Color::from_rgb8(0x2b, 0x2f, 0x36),
];

/// The theme catalog of a [`Terminal`].
pub trait Catalog {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

/// A styling function for a [`Terminal`].
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

/// The default style of a [`Terminal`]: the colors of the theme, the dark
/// or the light palette to go with them, and a ring of two pixels while it
/// has the keyboard focus (thicker in the primary color while typing).
pub fn default(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let background = palette.background.base.color;
    let text = palette.background.base.text;

    Style {
        background,
        text,
        cursor: text,
        selection: mix(background, palette.primary.base.color, 0.35),
        palette: if palette.is_dark {
            DARK_PALETTE
        } else {
            LIGHT_PALETTE
        },
        border: match status {
            Status::Active | Status::Hovered => Border {
                color: palette.background.strong.color,
                width: 1.0,
                radius: border::Radius::new(4.0),
            },
            Status::Focused => Border {
                color: text,
                width: 2.0,
                radius: border::Radius::new(4.0),
            },
            Status::Typing => Border {
                color: palette.primary.base.color,
                width: 2.0,
                radius: border::Radius::new(4.0),
            },
        },
    }
}
