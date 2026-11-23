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
//! Shift+Page Down scroll at any time. Ctrl+Shift+F (Cmd+F) is handed to
//! the application when it asked for it with [`Terminal::on_search`].
//!
//! # Search
//! [`Content::search`] looks for a text in everything the terminal holds;
//! every place is marked, the terminal scrolls to the one that is current,
//! and [`Content::search_next`] and [`Content::search_previous`] go from one
//! to the other. The field the text is typed into is the application's.
//!
//! # The mouse
//! A drag selects, a double click a word, a triple click a line; a drag
//! past the top or the bottom scrolls for as long as the button is held
//! there. When the program in the terminal asks to be told of the mouse
//! (`ESC [ ? 1000 h`, `1002`, `1003`; a full screen editor does), presses,
//! releases, drags and the wheel go to it as bytes of
//! [`Terminal::on_input`] instead, and holding Shift gives the mouse back
//! to the selection, as in xterm.
//!
//! # Links
//! With [`Terminal::on_link`] an address a program wrote (`https://...`,
//! see [`grid::SCHEMES`]) or put on some text (`OSC 8`) is underlined under
//! the pointer, and Ctrl+click (Cmd+click on macOS) produces the message
//! with the address; [`Terminal::plain_link_click`] makes a plain click do
//! so. The widget opens nothing itself.
pub mod grid;
pub mod keys;
pub mod report;

pub use grid::{Cell, Flags, Grid, Link, MouseEncoding, MouseMode, Row};

use crate::core::alignment;
use crate::core::border::{self, Border};
use crate::core::clipboard::{self, Clipboard};
use crate::core::keyboard;
use crate::core::keyboard::key::Named;
use crate::core::layout;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::text::{self, Paragraph as _};
use crate::core::time::{Duration, Instant};
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
    search: Search,
}

/// What is searched for, and where it was found.
#[derive(Debug, Default)]
struct Search {
    needle: String,
    /// The first and the last cell of every place, from the first line on.
    places: Vec<(Place, Place)>,
    current: Option<usize>,
    /// Counts the times another place became the current one: a
    /// [`Terminal`] that has not seen this number yet scrolls to the place.
    shown: u64,
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
            search: Search::default(),
        }
    }

    /// Feeds bytes the program wrote: text and escape sequences, in any
    /// pieces (a sequence may be cut anywhere).
    ///
    /// While a [`search`](Self::search) is on, everything is searched again
    /// after each call.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.grid, bytes);
        self.search_again();
    }

    /// Gives the grid another size, as when the pseudo-terminal got one.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.grid.resize(cols, rows);
        self.search_again();
    }

    /// Looks for `needle` in the history and on the screen and returns in
    /// how many places it stands. A [`Terminal`] marks them all; the last
    /// one (the newest) becomes the current place and the terminal scrolls
    /// to it. Capitals and small letters are the same as long as `needle`
    /// has no capital in it; a line that was wrapped is searched as one. An
    /// empty `needle` ends the search, as [`search_clear`](Self::search_clear)
    /// does.
    pub fn search(&mut self, needle: &str) -> usize {
        if needle.is_empty() {
            self.search_clear();
            return 0;
        }

        self.search.needle = needle.to_owned();
        self.search.current = None;
        self.search_again();
        self.search.current = self.search.places.len().checked_sub(1);
        self.search.shown += 1;
        self.search.places.len()
    }

    /// Makes the next place (towards the end) the current one; after the
    /// last comes the first.
    pub fn search_next(&mut self) {
        self.search_step(1);
    }

    /// Makes the place before (towards the start) the current one; before
    /// the first comes the last.
    pub fn search_previous(&mut self) {
        self.search_step(-1);
    }

    /// Ends the search: nothing is marked any more.
    pub fn search_clear(&mut self) {
        self.search.needle.clear();
        self.search.places.clear();
        self.search.current = None;
    }

    /// What is searched for; empty when nothing is.
    pub fn search_needle(&self) -> &str {
        &self.search.needle
    }

    /// In how many places it stands.
    pub fn search_count(&self) -> usize {
        self.search.places.len()
    }

    /// Which of the places is the current one, counted from 1 at the first
    /// line: "3 of 17" is `search_position()` of `search_count()`.
    pub fn search_position(&self) -> Option<usize> {
        self.search.current.map(|current| current + 1)
    }

    fn search_step(&mut self, by: isize) {
        let count = self.search.places.len() as isize;

        if count == 0 {
            return;
        }

        let current = self.search.current.map_or(0, |current| {
            (current as isize + by).rem_euclid(count) as usize
        });

        self.search.current = Some(current);
        self.search.shown += 1;
    }

    fn search_again(&mut self) {
        if self.search.needle.is_empty() {
            return;
        }

        let forgotten = self.grid.forgotten();
        let case_sensitive = self.search.needle.chars().any(char::is_uppercase);
        let was = self
            .search
            .current
            .and_then(|current| self.search.places.get(current))
            .map(|place| place.0);

        self.search.places = self
            .grid
            .find(&self.search.needle, case_sensitive)
            .into_iter()
            .map(|(start, end)| {
                (
                    (forgotten + start.0 as u64, start.1),
                    (forgotten + end.0 as u64, end.1),
                )
            })
            .collect();

        // The current place stays the current one while it is there
        self.search.current = was.and_then(|was| {
            self.search
                .places
                .binary_search_by(|place| place.0.cmp(&was))
                .ok()
                .or(self.search.places.len().checked_sub(1))
        });
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
    on_link: Option<Box<dyn Fn(String) -> Message + 'a>>,
    plain_link_click: bool,
    on_search: Option<Box<dyn Fn() -> Message + 'a>>,
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
            on_link: None,
            plain_link_click: false,
            on_search: None,
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

    /// Sets the message for a link that was clicked, with its address: one
    /// a program wrote out (`https://example.com/a`; the schemes are
    /// [`grid::SCHEMES`]) or one it put on some text (`OSC 8`). The link
    /// under the pointer is underlined, and Ctrl+click (Cmd+click on macOS)
    /// produces the message; see [`plain_link_click`](Self::plain_link_click).
    /// What happens to the address is the application's: the widget opens
    /// nothing. While the program in the terminal is told of the mouse, a
    /// link needs Shift held as well, as a selection does.
    pub fn on_link(mut self, on_link: impl Fn(String) -> Message + 'a) -> Self {
        self.on_link = Some(Box::new(on_link));
        self
    }

    /// Sets whether a plain click on a link (pressed and released on it
    /// without a drag) produces the message of [`on_link`](Self::on_link)
    /// too, which suits a terminal that only shows. A double click on an
    /// address then both opens it and selects the word.
    pub fn plain_link_click(mut self, plain: bool) -> Self {
        self.plain_link_click = plain;
        self
    }

    /// Sets the message for Ctrl+Shift+F (Cmd+F on macOS) while the
    /// [`Terminal`] has the keyboard focus: the key of "find", which would
    /// otherwise go to the program. The application shows its search field
    /// and calls [`Content::search`].
    pub fn on_search(mut self, on_search: Message) -> Self
    where
        Message: Clone + 'a,
    {
        self.on_search = Some(Box::new(move || on_search.clone()));
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
    modifiers: keyboard::Modifiers,
    /// The search place of the [`Content`] it scrolled to last.
    shown: u64,
    /// When a drag past the top or the bottom scrolled last.
    followed: Option<Instant>,
    /// The link under the pointer: its first and its last cell.
    hovered: Option<(Place, Place)>,
    /// The link a plain click began on.
    pressed_link: Option<String>,
    /// The button the program was told is down.
    reported: Option<report::Button>,
    /// The cell of the screen the program was told of last.
    reported_cell: Option<(usize, usize)>,
}

/// How often a drag past the top or the bottom scrolls.
const FOLLOW: Duration = Duration::from_millis(50);

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

    /// The cell under a point, when there is one: a point outside the
    /// lines that are shown has none.
    fn cell_at(
        &self,
        state: &State,
        bounds: Rectangle,
        position: Point,
    ) -> Option<Place> {
        let grid = self.content.grid();
        let inner = self.inner(bounds);
        let (first, count) =
            window(grid, state.scroll, self.visible_rows(state, bounds));
        let row = ((position.y - inner.y) / state.cell.height.max(1.0)).floor();
        let col = ((position.x - inner.x) / state.cell.width.max(1.0)).floor();

        (row >= 0.0
            && col >= 0.0
            && (row as usize) < count
            && (col as usize) < grid.cols())
        .then(|| {
            (
                grid.forgotten() + (first + row as usize) as u64,
                col as usize,
            )
        })
    }

    /// The link under a point.
    fn link_at(
        &self,
        state: &State,
        bounds: Rectangle,
        position: Point,
    ) -> Option<Link> {
        let grid = self.content.grid();
        let place = self.cell_at(state, bounds, position)?;

        grid.link_at((place.0 - grid.forgotten()) as usize, place.1)
    }

    /// The cell of the screen (column and line, as the program counts
    /// them from 0) under a point; a point outside is the nearest cell.
    /// `None` while the view is scrolled so far that the point is over the
    /// history, which the program knows nothing of.
    fn screen_cell(
        &self,
        state: &State,
        bounds: Rectangle,
        position: Point,
    ) -> Option<(usize, usize)> {
        let grid = self.content.grid();
        let inner = self.inner(bounds);
        let visible = self.visible_rows(state, bounds);
        let (first, _) = window(grid, state.scroll, visible);
        let row = ((position.y - inner.y) / state.cell.height.max(1.0))
            .floor()
            .clamp(0.0, visible.saturating_sub(1) as f32)
            as usize;
        let col = ((position.x - inner.x) / state.cell.width.max(1.0))
            .floor()
            .clamp(0.0, grid.cols().saturating_sub(1) as f32)
            as usize;
        let line = (first + row).checked_sub(grid.history())?;

        Some((col, line.min(grid.rows().saturating_sub(1))))
    }

    /// Whether the mouse is the program's now: it asked to be told, it can
    /// be told, and Shift is not held.
    fn reports(&self, state: &State) -> bool {
        self.on_input.is_some()
            && self.content.grid().mouse_mode() != MouseMode::None
            && !state.modifiers.shift()
    }

    /// Tells the program what the mouse did, when that is asked for.
    fn report(
        &self,
        state: &mut State,
        action: report::Action,
        cell: (usize, usize),
        shell: &mut Shell<'_, Message>,
    ) {
        let grid = self.content.grid();

        if let Some(on_input) = &self.on_input
            && action.is_reported(grid.mouse_mode())
            && let Some(bytes) = report::encode(
                action,
                cell.0,
                cell.1,
                state.modifiers,
                grid.mouse_encoding(),
            )
        {
            shell.publish(on_input(bytes));
        }

        state.reported_cell = Some(cell);
    }

    /// The mouse of a program that asked to be told of it. Returns whether
    /// the event was the program's, and so nothing for the selection.
    fn report_mouse(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        let button_of = |button: &mouse::Button| match button {
            mouse::Button::Left => Some(report::Button::Left),
            mouse::Button::Middle => Some(report::Button::Middle),
            mouse::Button::Right => Some(report::Button::Right),
            _ => None,
        };

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(button))
                if self.reports(state)
                    && cursor.is_over(bounds)
                    && !shell.is_event_captured() =>
            {
                let Some(button) = button_of(button) else {
                    return false;
                };
                let Some(cell) = cursor
                    .position()
                    .and_then(|at| self.screen_cell(state, bounds, at))
                else {
                    return false;
                };

                state.is_focused = true;
                state.is_typing = true;
                state.dragging = None;
                state.reported = Some(button);

                if state.selection.take().is_some() {
                    shell.request_redraw();
                }

                self.report(state, report::Action::Press(button), cell, shell);
                shell.capture_event();
                true
            }
            Event::Mouse(mouse::Event::ButtonReleased(button))
                if state.reported.is_some()
                    && state.reported == button_of(button) =>
            {
                let held = state.reported.take();
                let cell = cursor
                    .position()
                    .and_then(|at| self.screen_cell(state, bounds, at))
                    .or(state.reported_cell);

                if let (Some(button), Some(cell)) = (held, cell) {
                    self.report(
                        state,
                        report::Action::Release(button),
                        cell,
                        shell,
                    );
                }

                true
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
                if state.reported.is_some()
                    || (self.reports(state) && cursor.is_over(bounds)) =>
            {
                let cell = cursor
                    .position()
                    .and_then(|at| self.screen_cell(state, bounds, at));

                // Told once for every cell the pointer comes to
                if let Some(cell) = cell
                    && state.reported_cell != Some(cell)
                {
                    self.report(
                        state,
                        report::Action::Move(state.reported),
                        cell,
                        shell,
                    );
                }

                state.reported.is_some() || self.reports(state)
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if self.reports(state)
                    && cursor.is_over(bounds)
                    && !shell.is_event_captured() =>
            {
                let Some(cell) = cursor
                    .position()
                    .and_then(|at| self.screen_cell(state, bounds, at))
                else {
                    return false;
                };
                let moved = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        y / state.cell.height.max(1.0)
                    }
                } + state.wheel;
                let whole = moved.trunc();

                state.wheel = moved - whole;

                let button = if whole > 0.0 {
                    report::Button::WheelUp
                } else {
                    report::Button::WheelDown
                };

                for _ in 0..(whole.abs() as usize).min(20) {
                    self.report(
                        state,
                        report::Action::Press(button),
                        cell,
                        shell,
                    );
                }

                shell.capture_event();
                true
            }
            _ => false,
        }
    }

    /// The scroll position once the place a search made current is shown.
    fn scroll_now(&self, state: &State, bounds: Rectangle) -> usize {
        let search = &self.content.search;
        let grid = self.content.grid();

        if search.shown == state.shown {
            return state.scroll;
        }

        let Some(index) = search
            .current
            .and_then(|current| search.places.get(current))
            .and_then(|place| place.0.0.checked_sub(grid.forgotten()))
        else {
            return state.scroll;
        };

        let visible = self.visible_rows(state, bounds);
        let (first, count) = window(grid, state.scroll, visible);
        let index = index as usize;

        if (first..first + count).contains(&index) {
            return state.scroll;
        }

        // In the middle of the view
        let max = lines(grid).saturating_sub(visible);
        let first = index.saturating_sub(visible / 2).min(max);

        max - first
    }

    /// Scrolls to the place a search made current, once.
    fn show_search(&self, state: &mut State, bounds: Rectangle) {
        state.scroll = self.scroll_now(state, bounds);
        state.shown = self.content.search.shown;
    }

    /// The selection of a drag reaches the cell under `position`. Returns
    /// whether the selection changed.
    fn drag_to(
        &self,
        state: &mut State,
        bounds: Rectangle,
        position: Point,
    ) -> bool {
        let Some((from, to, unit)) = state.dragging else {
            return false;
        };
        let grid = self.content.grid();
        let place = self.place_at(state, bounds, position);
        let index = (place.0 - grid.forgotten()) as usize;
        let head = match unit {
            Unit::Cell => (place, place),
            Unit::Word => {
                let (start, end) = grid
                    .line(index)
                    .map_or((place.1, place.1), |row| word_at(row, place.1));

                ((place.0, start), (place.0, end))
            }
            Unit::Line => ((place.0, 0), (place.0, usize::MAX)),
        };
        let selection = (from.min(head.0), to.max(head.1));

        if unit != Unit::Cell || place != from {
            if state.selection != Some(selection) {
                state.selection = Some(selection);
                return true;
            }

            false
        } else {
            state.selection.take().is_some()
        }
    }

    /// A drag that is past the top or the bottom scrolls the view, a step
    /// every [`FOLLOW`] for as long as the pointer is there (further away,
    /// larger steps), and the selection follows.
    fn follow(
        &self,
        state: &mut State,
        bounds: Rectangle,
        position: Point,
        now: Instant,
        shell: &mut Shell<'_, Message>,
    ) {
        if state.dragging.is_none() {
            return;
        }

        let beyond = if position.y < bounds.y {
            bounds.y - position.y
        } else if position.y > bounds.y + bounds.height {
            bounds.y + bounds.height - position.y
        } else {
            state.followed = None;
            return;
        };

        if state.followed.is_none_or(|last| now >= last + FOLLOW) {
            let step = 1
                + ((beyond.abs() / (2.0 * state.cell.height.max(1.0)))
                    as isize)
                    .min(4);

            self.scroll_by(
                state,
                bounds,
                if beyond > 0.0 { step } else { -step },
            );
            state.followed = Some(now);

            if self.drag_to(state, bounds, position) {
                shell.request_redraw();
            }
        }

        // Again, unless that end is reached
        let max = lines(self.content.grid())
            .saturating_sub(self.visible_rows(state, bounds));

        if (beyond > 0.0 && state.scroll < max)
            || (beyond < 0.0 && state.scroll > 0)
        {
            shell.request_redraw_at(now + FOLLOW);
        }
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
        self.show_search(state, bounds);

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
        self.show_search(state, bounds);

        if self.on_input.is_none() {
            state.is_typing = false;
        }

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;

                // The pointer over a link looks different with the key held
                if state.hovered.is_some() {
                    shell.request_redraw();
                }
            }
            Event::Keyboard(
                keyboard::Event::KeyPressed { modifiers, .. }
                | keyboard::Event::KeyReleased { modifiers, .. },
            ) => {
                state.modifiers = *modifiers;
            }
            _ => {}
        }

        // The link under the pointer
        if let Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
            let hovered = (self.on_link.is_some()
                && state.dragging.is_none()
                && state.reported.is_none()
                && !self.reports(state))
            .then(|| {
                cursor
                    .position_over(bounds)
                    .and_then(|at| self.link_at(state, bounds, at))
            })
            .flatten()
            .map(|link| {
                let forgotten = grid.forgotten();

                (
                    (forgotten + link.start.0 as u64, link.start.1),
                    (forgotten + link.end.0 as u64, link.end.1),
                )
            });

            if state.hovered != hovered {
                state.hovered = hovered;
                shell.request_redraw();
            }
        }

        let reported = self.report_mouse(state, event, bounds, cursor, shell);

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
            _ if reported => {}
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

                // A link, with the key that opens one
                state.pressed_link = None;

                if let Some(on_link) = &self.on_link
                    && matches!(
                        event,
                        Event::Mouse(mouse::Event::ButtonPressed(_))
                    )
                    && let Some(link) = self.link_at(state, bounds, position)
                {
                    if state.modifiers.command() {
                        state.is_focused = true;
                        shell.publish(on_link(link.url));
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }

                    if self.plain_link_click {
                        state.pressed_link = Some(link.url);
                    }
                }

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
                        state.pressed_link = None;

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
                        state.pressed_link = None;

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
                if state.dragging.is_none() {
                    return;
                }

                let Some(position) = cursor.position() else {
                    return;
                };

                // Past the top or the bottom the view follows, and goes on
                // following while the pointer rests there
                self.follow(state, bounds, position, Instant::now(), shell);

                if self.drag_to(state, bounds, position) {
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(now))
                if state.dragging.is_some() =>
            {
                if let Some(position) = cursor.position() {
                    self.follow(state, bounds, position, *now, shell);
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(
                touch::Event::FingerLifted { .. }
                | touch::Event::FingerLost { .. },
            ) => {
                state.followed = None;

                if state.dragging.take().is_some() {
                    // On X11 and Wayland what is selected is the primary
                    // selection (a middle click pastes it)
                    if let Some(selected) = self.selected_text(state) {
                        clipboard.write(clipboard::Kind::Primary, selected);
                    }

                    // A plain click on a link: released where it was
                    // pressed, and nothing was selected on the way
                    if let Some(pressed) = state.pressed_link.take()
                        && state.selection.is_none()
                        && let Some(on_link) = &self.on_link
                        && cursor
                            .position()
                            .and_then(|at| self.link_at(state, bounds, at))
                            .is_some_and(|link| link.url == pressed)
                    {
                        shell.publish(on_link(pressed));
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

                // Find: the application's
                if letter.as_deref() == Some("f")
                    && chord
                    && let Some(on_search) = &self.on_search
                {
                    shell.publish(on_search());
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
        let scroll = self.scroll_now(state, bounds);
        let (first, count) =
            window(grid, scroll, self.visible_rows(state, bounds));
        let forgotten = grid.forgotten();
        let search = &self.content.search;
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

        let cursor_at = (grid.cursor_visible() && scroll == 0)
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

                // The part of this line between two cells
                let line_number = forgotten + index as u64;
                let span = |start: Place, end: Place| {
                    if !(start.0..=end.0).contains(&line_number) {
                        return None;
                    }

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

                    (to > from).then_some((from, to))
                };

                // What a search found: every place, and the current one
                // stronger and framed
                let found_from = search
                    .places
                    .partition_point(|place| place.1.0 < line_number);

                for (number, place) in
                    search.places.iter().enumerate().skip(found_from)
                {
                    if place.0.0 > line_number {
                        break;
                    }

                    let Some((from, to)) = span(place.0, place.1) else {
                        continue;
                    };
                    let is_current = search.current == Some(number);
                    let mark = style.indexed(3);

                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle {
                                x: inner.x + from as f32 * cell.width,
                                y: inner.y + row as f32 * cell.height,
                                width: (to - from) as f32 * cell.width,
                                height: cell.height,
                            },
                            border: Border {
                                color: style.text,
                                width: if is_current { 1.0 } else { 0.0 },
                                radius: 0.0.into(),
                            },
                            ..renderer::Quad::default()
                        },
                        mix(
                            style.background,
                            mark,
                            if is_current { 0.6 } else { 0.3 },
                        ),
                    );
                }

                // The selection
                if let Some((start, end)) = state.selection
                    && let Some((from, to)) = span(start, end)
                {
                    fill(renderer, row, from, to, style.selection);
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

                // The link under the pointer is underlined
                if let Some((start, end)) = state.hovered
                    && let Some((from, to)) = span(start, end)
                {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle {
                                x: inner.x + from as f32 * cell.width,
                                y: (inner.y
                                    + row as f32 * cell.height
                                    + cell.height * 0.92)
                                    .round(),
                                width: (to - from) as f32 * cell.width,
                                height: 1.0,
                            },
                            ..renderer::Quad::default()
                        },
                        style.text,
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
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();

        if !cursor.is_over(layout.bounds()) {
            mouse::Interaction::default()
        } else if state.hovered.is_some()
            && (self.plain_link_click || state.modifiers.command())
        {
            // A click here opens the link
            mouse::Interaction::Pointer
        } else if self.reports(state) {
            // The mouse is the program's: nothing is selected with it
            mouse::Interaction::Idle
        } else {
            mouse::Interaction::Text
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
