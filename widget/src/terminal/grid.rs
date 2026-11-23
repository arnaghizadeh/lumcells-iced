//! The grid of a terminal: cells in rows, a cursor, a scrolling region, the
//! alternate screen and the lines that scrolled away. It implements
//! [`vte::ansi::Handler`], so the parser of `vte` drives it.
use std::collections::VecDeque;

use unicode_width::UnicodeWidthChar;
use vte::ansi::{
    Attr, CharsetIndex, ClearMode, Color as AnsiColor, Handler, LineClearMode,
    Mode, NamedColor, NamedMode, NamedPrivateMode, PrivateMode,
    StandardCharset, TabulationClearMode,
};

/// The color of a cell, as the program in the terminal named it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// The foreground or the background of the terminal.
    #[default]
    Default,
    /// One of the 256 indexed colors; the first 16 are the palette of the
    /// style.
    Indexed(u8),
    /// A color given by its parts.
    Rgb(u8, u8, u8),
}

/// How a cell is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Flags(u16);

impl Flags {
    /// Bold.
    pub const BOLD: Flags = Flags(1);
    /// Faint.
    pub const DIM: Flags = Flags(1 << 1);
    /// Italic.
    pub const ITALIC: Flags = Flags(1 << 2);
    /// Underlined.
    pub const UNDERLINE: Flags = Flags(1 << 3);
    /// Foreground and background swapped.
    pub const INVERSE: Flags = Flags(1 << 4);
    /// Not shown.
    pub const HIDDEN: Flags = Flags(1 << 5);
    /// Struck through.
    pub const STRIKE: Flags = Flags(1 << 6);
    /// The first half of a character that is two cells wide.
    pub const WIDE: Flags = Flags(1 << 7);
    /// The second half of a character that is two cells wide.
    pub const WIDE_SPACER: Flags = Flags(1 << 8);

    /// Whether all of `other` is set.
    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    fn insert(&mut self, other: Flags) {
        self.0 |= other.0;
    }

    fn remove(&mut self, other: Flags) {
        self.0 &= !other.0;
    }
}

/// One cell of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The character; a space when the cell is empty.
    pub c: char,
    /// Its color.
    pub fg: Color,
    /// The color behind it.
    pub bg: Color,
    /// How it is drawn.
    pub flags: Flags,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            c: ' ',
            fg: Color::Default,
            bg: Color::Default,
            flags: Flags::default(),
        }
    }
}

impl Cell {
    /// Nothing is in it and nothing is drawn behind it.
    pub fn is_blank(&self) -> bool {
        self.c == ' '
            && self.bg == Color::Default
            && !self.flags.contains(Flags::INVERSE)
            && !self.flags.contains(Flags::WIDE_SPACER)
    }
}

/// One line of the grid.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    /// The cells, as many as the grid had columns when the line was written.
    pub cells: Vec<Cell>,
    /// The line went on in the next one because it was full (no line break
    /// between the two when text is copied).
    pub wrapped: bool,
}

impl Row {
    fn new(cols: usize) -> Row {
        Row {
            cells: vec![Cell::default(); cols],
            wrapped: false,
        }
    }

    fn filled(cols: usize, cell: Cell) -> Row {
        Row {
            cells: vec![cell; cols],
            wrapped: false,
        }
    }

    /// How many cells are in use: up to the last one that is not blank.
    pub fn len_used(&self) -> usize {
        self.cells
            .iter()
            .rposition(|cell| !cell.is_blank())
            .map_or(0, |last| last + 1)
    }

    /// The text of the cells from `from` up to `to` (not included), without
    /// the blanks at the end of the line.
    pub fn text(&self, from: usize, to: usize) -> String {
        let to = to.min(self.len_used());

        self.cells
            .get(from.min(to)..to)
            .unwrap_or(&[])
            .iter()
            .filter(|cell| !cell.flags.contains(Flags::WIDE_SPACER))
            .map(|cell| cell.c)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Cursor {
    row: usize,
    col: usize,
    /// The last column was written: the next character goes to the next
    /// line.
    pending_wrap: bool,
    /// The colors and flags of what is written next.
    pen: Cell,
    charsets: [StandardCharset; 4],
    charset: usize,
    origin: bool,
}

#[derive(Debug, Clone)]
struct Primary {
    screen: Vec<Row>,
    cursor: Cursor,
}

/// The grid of a terminal.
#[derive(Debug, Clone)]
pub struct Grid {
    cols: usize,
    rows: usize,
    screen: Vec<Row>,
    history: VecDeque<Row>,
    history_limit: usize,
    dropped: u64,
    /// The primary screen, while the alternate one is shown.
    primary: Option<Primary>,
    cursor: Cursor,
    saved: Cursor,
    top: usize,
    /// One past the last line of the scrolling region.
    bottom: usize,
    tabs: Vec<bool>,
    autowrap: bool,
    insert: bool,
    linefeed_newline: bool,
    cursor_visible: bool,
    application_cursor: bool,
    application_keypad: bool,
    bracketed_paste: bool,
    title: Option<String>,
    answers: Vec<u8>,
    bells: u64,
}

fn tab_stops(cols: usize) -> Vec<bool> {
    (0..cols).map(|col| col % 8 == 0 && col > 0).collect()
}

impl Grid {
    /// A grid of `cols` by `rows` cells (at least 2 by 1) that remembers up
    /// to `history_limit` lines that scrolled away.
    pub fn new(cols: u16, rows: u16, history_limit: usize) -> Grid {
        let cols = usize::from(cols.max(2));
        let rows = usize::from(rows.max(1));

        Grid {
            cols,
            rows,
            screen: (0..rows).map(|_| Row::new(cols)).collect(),
            history: VecDeque::new(),
            history_limit,
            dropped: 0,
            primary: None,
            cursor: Cursor::default(),
            saved: Cursor::default(),
            top: 0,
            bottom: rows,
            tabs: tab_stops(cols),
            autowrap: true,
            insert: false,
            linefeed_newline: false,
            cursor_visible: true,
            application_cursor: false,
            application_keypad: false,
            bracketed_paste: false,
            title: None,
            answers: Vec::new(),
            bells: 0,
        }
    }

    /// The number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The number of lines of the screen.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The lines that scrolled away and are remembered.
    pub fn history(&self) -> usize {
        self.history.len()
    }

    /// The lines that scrolled away and were forgotten since the start: line
    /// `n` of [`line`](Self::line) is line `forgotten() + n` of everything
    /// the terminal ever showed.
    pub fn forgotten(&self) -> u64 {
        self.dropped
    }

    /// History and screen together.
    pub fn total_lines(&self) -> usize {
        self.history.len() + self.rows
    }

    /// History and the part of the screen that is in use: up to the line of
    /// the cursor or the last line with something in it.
    pub fn used_lines(&self) -> usize {
        let used = self
            .screen
            .iter()
            .rposition(|row| row.len_used() > 0)
            .map_or(0, |last| last + 1)
            .max(self.cursor.row + 1);

        self.history.len() + used
    }

    /// A line of the history (from 0) or, after it, of the screen.
    pub fn line(&self, index: usize) -> Option<&Row> {
        if index < self.history.len() {
            self.history.get(index)
        } else {
            self.screen.get(index - self.history.len())
        }
    }

    /// Where the cursor is on the screen: line and column from 0.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor.row, self.cursor.col)
    }

    /// Whether the program wants the cursor shown.
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Whether the arrow keys are to be sent in their "application" form.
    pub fn application_cursor(&self) -> bool {
        self.application_cursor
    }

    /// Whether the keypad is in its "application" mode.
    pub fn application_keypad(&self) -> bool {
        self.application_keypad
    }

    /// Whether pasted text is to be put between the paste marks.
    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste
    }

    /// Whether the alternate screen is shown (a full screen program runs).
    pub fn alternate_screen(&self) -> bool {
        self.primary.is_some()
    }

    /// The title the program set.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// How often the bell rang.
    pub fn bells(&self) -> u64 {
        self.bells
    }

    /// What the terminal answers to the questions the program asked (where
    /// the cursor is, what kind of terminal this is): bytes for the program's
    /// input. They are handed out once.
    pub fn take_answers(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.answers)
    }

    /// The text of the lines from `from` up to `to` (not included), counted
    /// as [`line`](Self::line) counts. Lines that were wrapped are joined.
    pub fn text(&self, from: usize, to: usize) -> String {
        self.selected((from, 0), (to.saturating_sub(1), usize::MAX))
    }

    /// The text between two cells (line and column, both included), with a
    /// line break after every line that did not wrap into the next.
    pub fn selected(
        &self,
        start: (usize, usize),
        end: (usize, usize),
    ) -> String {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let last = end.0.min(self.total_lines().saturating_sub(1));
        let mut text = String::new();

        for index in start.0..=last {
            let Some(row) = self.line(index) else { break };
            let from = if index == start.0 { start.1 } else { 0 };
            let to = if index == end.0 {
                end.1.saturating_add(1)
            } else {
                usize::MAX
            };

            text.push_str(&row.text(from, to));

            if index != last && !row.wrapped {
                text.push('\n');
            }
        }

        text
    }

    /// Gives the grid another size. Lines keep what they have (there is no
    /// new wrapping); when the screen gets shorter, lines above the cursor
    /// go to the history.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = usize::from(cols.max(2));
        let rows = usize::from(rows.max(1));

        if cols == self.cols && rows == self.rows {
            return;
        }

        if rows < self.rows {
            // The lines below the cursor go first, then lines from the top.
            let mut spare = self.rows - rows;

            while spare > 0 && self.screen.len() > self.cursor.row + 1 {
                let _ = self.screen.pop();
                spare -= 1;
            }

            for _ in 0..spare {
                let row = self.screen.remove(0);
                self.remember(row);
                self.cursor.row = self.cursor.row.saturating_sub(1);
            }
        } else {
            for _ in self.rows..rows {
                self.screen.push(Row::new(cols));
            }
        }

        for row in &mut self.screen {
            row.cells.resize(cols, Cell::default());
        }

        if let Some(primary) = &mut self.primary {
            primary.screen.resize(rows, Row::new(cols));

            for row in &mut primary.screen {
                row.cells.resize(cols, Cell::default());
            }

            primary.cursor.row = primary.cursor.row.min(rows - 1);
            primary.cursor.col = primary.cursor.col.min(cols - 1);
        }

        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows;
        self.tabs = tab_stops(cols);
        self.cursor.row = self.cursor.row.min(rows - 1);
        self.cursor.col = self.cursor.col.min(cols - 1);
        self.cursor.pending_wrap = false;
        self.saved.row = self.saved.row.min(rows - 1);
        self.saved.col = self.saved.col.min(cols - 1);
    }

    fn remember(&mut self, row: Row) {
        if self.primary.is_some() || self.history_limit == 0 {
            return;
        }

        self.history.push_back(row);

        while self.history.len() > self.history_limit {
            let _ = self.history.pop_front();
            self.dropped += 1;
        }
    }

    fn blank(&self) -> Cell {
        Cell {
            c: ' ',
            fg: Color::Default,
            bg: self.cursor.pen.bg,
            flags: Flags::default(),
        }
    }

    /// The content of the region moves up by `count` lines; what leaves a
    /// region that begins at the top of the screen goes to the history.
    fn scroll_region_up(&mut self, top: usize, count: usize) {
        let count = count.min(self.bottom.saturating_sub(top));
        let blank = self.blank();

        for _ in 0..count {
            let row = self.screen.remove(top);

            if top == 0 {
                self.remember(row);
            }

            self.screen
                .insert(self.bottom - 1, Row::filled(self.cols, blank));
        }
    }

    fn scroll_region_down(&mut self, top: usize, count: usize) {
        let count = count.min(self.bottom.saturating_sub(top));
        let blank = self.blank();

        for _ in 0..count {
            let _ = self.screen.remove(self.bottom - 1);
            self.screen.insert(top, Row::filled(self.cols, blank));
        }
    }

    /// A wide character of which one half is overwritten loses the other.
    fn unhalve(&mut self, row: usize, col: usize) {
        let cols = self.cols;
        let cells = &mut self.screen[row].cells;

        if cells[col].flags.contains(Flags::WIDE) && col + 1 < cols {
            cells[col + 1] = Cell {
                bg: cells[col + 1].bg,
                ..Cell::default()
            };
        }

        if cells[col].flags.contains(Flags::WIDE_SPACER) && col > 0 {
            cells[col - 1] = Cell {
                bg: cells[col - 1].bg,
                ..Cell::default()
            };
        }
    }

    fn put(&mut self, col: usize, cell: Cell) {
        let row = self.cursor.row;

        self.unhalve(row, col);
        self.screen[row].cells[col] = cell;
    }

    fn wrap_now(&mut self) {
        self.screen[self.cursor.row].wrapped = true;
        self.cursor.pending_wrap = false;
        self.cursor.col = 0;
        self.index();
    }

    /// The cursor goes down a line; at the bottom of the region the content
    /// scrolls.
    fn index(&mut self) {
        if self.cursor.row + 1 == self.bottom {
            self.scroll_region_up(self.top, 1);
        } else if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        }
    }

    fn enter_alternate(&mut self) {
        if self.primary.is_some() {
            return;
        }

        let blank: Vec<Row> =
            (0..self.rows).map(|_| Row::new(self.cols)).collect();

        self.primary = Some(Primary {
            screen: std::mem::replace(&mut self.screen, blank),
            cursor: self.cursor,
        });
        self.top = 0;
        self.bottom = self.rows;
    }

    fn leave_alternate(&mut self) {
        if let Some(primary) = self.primary.take() {
            self.screen = primary.screen;
            self.cursor = primary.cursor;
            self.cursor.row = self.cursor.row.min(self.rows - 1);
            self.cursor.col = self.cursor.col.min(self.cols - 1);
            self.top = 0;
            self.bottom = self.rows;
        }
    }

    fn private_mode(&mut self, mode: PrivateMode, set: bool) {
        match mode {
            PrivateMode::Named(NamedPrivateMode::CursorKeys) => {
                self.application_cursor = set;
            }
            PrivateMode::Named(NamedPrivateMode::Origin) => {
                self.cursor.origin = set;
                self.goto(0, 0);
            }
            PrivateMode::Named(NamedPrivateMode::LineWrap) => {
                self.autowrap = set;
            }
            PrivateMode::Named(NamedPrivateMode::ShowCursor) => {
                self.cursor_visible = set;
            }
            PrivateMode::Named(NamedPrivateMode::BracketedPaste) => {
                self.bracketed_paste = set;
            }
            PrivateMode::Named(
                NamedPrivateMode::SwapScreenAndSetRestoreCursor,
            ) => {
                if set {
                    self.saved = self.cursor;
                    self.enter_alternate();
                } else {
                    self.leave_alternate();
                    self.cursor = self.saved;
                    self.cursor.row = self.cursor.row.min(self.rows - 1);
                    self.cursor.col = self.cursor.col.min(self.cols - 1);
                }
            }
            PrivateMode::Unknown(47 | 1047) => {
                if set {
                    self.enter_alternate();
                } else {
                    self.leave_alternate();
                }
            }
            PrivateMode::Unknown(1048) => {
                if set {
                    self.saved = self.cursor;
                } else {
                    self.cursor = self.saved;
                }
            }
            // The mouse, the focus, blinking, synchronized updates: nothing
            // the grid keeps
            _ => {}
        }
    }

    fn color(color: AnsiColor) -> Color {
        match color {
            AnsiColor::Named(named) => match named {
                NamedColor::Foreground
                | NamedColor::Background
                | NamedColor::Cursor
                | NamedColor::BrightForeground
                | NamedColor::DimForeground => Color::Default,
                NamedColor::DimBlack => Color::Indexed(0),
                NamedColor::DimRed => Color::Indexed(1),
                NamedColor::DimGreen => Color::Indexed(2),
                NamedColor::DimYellow => Color::Indexed(3),
                NamedColor::DimBlue => Color::Indexed(4),
                NamedColor::DimMagenta => Color::Indexed(5),
                NamedColor::DimCyan => Color::Indexed(6),
                NamedColor::DimWhite => Color::Indexed(7),
                other => Color::Indexed(other as u8),
            },
            AnsiColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
            AnsiColor::Indexed(index) => Color::Indexed(index),
        }
    }
}

impl Handler for Grid {
    fn set_title(&mut self, title: Option<String>) {
        self.title = title;
    }

    fn input(&mut self, c: char) {
        let c = self.cursor.charsets[self.cursor.charset].map(c);
        let width = match c.width() {
            // What combines with the character before it has no cell of its
            // own, and a cell holds one character: it is left out
            Some(0) | None => return,
            Some(width) => width,
        };

        if self.cursor.pending_wrap {
            if self.autowrap {
                self.wrap_now();
            } else {
                self.cursor.pending_wrap = false;
            }
        }

        // A wide character does not begin in the last column
        if width == 2 && self.cursor.col + 1 >= self.cols {
            if self.autowrap {
                self.wrap_now();
            } else {
                return;
            }
        }

        if self.insert {
            let (row, col) = (self.cursor.row, self.cursor.col);
            let cells = &mut self.screen[row].cells;

            for _ in 0..width {
                let _ = cells.pop();
                cells.insert(col, Cell::default());
            }
        }

        let pen = self.cursor.pen;
        let col = self.cursor.col;
        let mut cell = Cell { c, ..pen };

        if width == 2 {
            cell.flags.insert(Flags::WIDE);
            self.put(col, cell);

            let mut spacer = Cell { c: ' ', ..pen };
            spacer.flags.insert(Flags::WIDE_SPACER);
            self.put(col + 1, spacer);
        } else {
            self.put(col, cell);
        }

        if col + width >= self.cols {
            self.cursor.col = self.cols - 1;
            self.cursor.pending_wrap = true;
        } else {
            self.cursor.col = col + width;
        }
    }

    fn goto(&mut self, line: i32, col: usize) {
        let (first, last) = if self.cursor.origin {
            (self.top, self.bottom - 1)
        } else {
            (0, self.rows - 1)
        };
        let line = usize::try_from(line).unwrap_or(0);

        self.cursor.row = (first + line).min(last);
        self.cursor.col = col.min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn goto_line(&mut self, line: i32) {
        self.goto(line, self.cursor.col);
    }

    fn goto_col(&mut self, col: usize) {
        self.cursor.col = col.min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn insert_blank(&mut self, count: usize) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let blank = self.blank();
        let count = count.min(self.cols - col);
        let cells = &mut self.screen[row].cells;

        for _ in 0..count {
            let _ = cells.pop();
            cells.insert(col, blank);
        }

        self.cursor.pending_wrap = false;
    }

    fn move_up(&mut self, count: usize) {
        let first = if self.cursor.row >= self.top {
            self.top
        } else {
            0
        };

        self.cursor.row = self.cursor.row.saturating_sub(count).max(first);
        self.cursor.pending_wrap = false;
    }

    fn move_down(&mut self, count: usize) {
        let last = if self.cursor.row < self.bottom {
            self.bottom - 1
        } else {
            self.rows - 1
        };

        self.cursor.row = self.cursor.row.saturating_add(count).min(last);
        self.cursor.pending_wrap = false;
    }

    fn identify_terminal(&mut self, intermediate: Option<char>) {
        match intermediate {
            // A VT102, as plain as can be
            None => self.answers.extend_from_slice(b"\x1b[?6c"),
            Some('>') => self.answers.extend_from_slice(b"\x1b[>0;0;0c"),
            _ => {}
        }
    }

    fn device_status(&mut self, what: usize) {
        match what {
            5 => self.answers.extend_from_slice(b"\x1b[0n"),
            6 => {
                let row = self.cursor.row + 1
                    - if self.cursor.origin { self.top } else { 0 };
                let answer = format!("\x1b[{};{}R", row, self.cursor.col + 1);

                self.answers.extend_from_slice(answer.as_bytes());
            }
            _ => {}
        }
    }

    fn move_forward(&mut self, count: usize) {
        self.cursor.col =
            self.cursor.col.saturating_add(count).min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn move_backward(&mut self, count: usize) {
        self.cursor.col = self.cursor.col.saturating_sub(count);
        self.cursor.pending_wrap = false;
    }

    fn move_down_and_cr(&mut self, count: usize) {
        self.move_down(count);
        self.cursor.col = 0;
    }

    fn move_up_and_cr(&mut self, count: usize) {
        self.move_up(count);
        self.cursor.col = 0;
    }

    fn put_tab(&mut self, count: u16) {
        for _ in 0..count {
            let mut col = self.cursor.col + 1;

            while col < self.cols - 1 && !self.tabs[col] {
                col += 1;
            }

            self.cursor.col = col.min(self.cols - 1);
        }

        self.cursor.pending_wrap = false;
    }

    fn backspace(&mut self) {
        self.cursor.col = self.cursor.col.saturating_sub(1);
        self.cursor.pending_wrap = false;
    }

    fn carriage_return(&mut self) {
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    fn linefeed(&mut self) {
        self.cursor.pending_wrap = false;
        self.index();

        if self.linefeed_newline {
            self.cursor.col = 0;
        }
    }

    fn bell(&mut self) {
        self.bells += 1;
    }

    fn newline(&mut self) {
        self.cursor.pending_wrap = false;
        self.index();
        self.cursor.col = 0;
    }

    fn set_horizontal_tabstop(&mut self) {
        let col = self.cursor.col;
        self.tabs[col] = true;
    }

    fn scroll_up(&mut self, count: usize) {
        self.scroll_region_up(self.top, count);
    }

    fn scroll_down(&mut self, count: usize) {
        self.scroll_region_down(self.top, count);
    }

    fn insert_blank_lines(&mut self, count: usize) {
        if (self.top..self.bottom).contains(&self.cursor.row) {
            self.scroll_region_down(self.cursor.row, count);
            self.cursor.col = 0;
            self.cursor.pending_wrap = false;
        }
    }

    fn delete_lines(&mut self, count: usize) {
        if (self.top..self.bottom).contains(&self.cursor.row) {
            // What leaves in the middle of the screen is not history
            let row = self.cursor.row;
            let count = count.min(self.bottom - row);
            let blank = self.blank();

            for _ in 0..count {
                let _ = self.screen.remove(row);
                self.screen
                    .insert(self.bottom - 1, Row::filled(self.cols, blank));
            }

            self.cursor.col = 0;
            self.cursor.pending_wrap = false;
        }
    }

    fn erase_chars(&mut self, count: usize) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let blank = self.blank();
        let to = col.saturating_add(count.max(1)).min(self.cols);

        for at in col..to {
            self.unhalve(row, at);
            self.screen[row].cells[at] = blank;
        }

        self.cursor.pending_wrap = false;
    }

    fn delete_chars(&mut self, count: usize) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let blank = self.blank();
        let count = count.max(1).min(self.cols - col);

        self.unhalve(row, col);

        let cells = &mut self.screen[row].cells;

        for _ in 0..count {
            let _ = cells.remove(col);
            cells.push(blank);
        }

        self.cursor.pending_wrap = false;
    }

    fn move_backward_tabs(&mut self, count: u16) {
        for _ in 0..count {
            let mut col = self.cursor.col.saturating_sub(1);

            while col > 0 && !self.tabs[col] {
                col -= 1;
            }

            self.cursor.col = col;
        }

        self.cursor.pending_wrap = false;
    }

    fn move_forward_tabs(&mut self, count: u16) {
        self.put_tab(count);
    }

    fn save_cursor_position(&mut self) {
        self.saved = self.cursor;
    }

    fn restore_cursor_position(&mut self) {
        self.cursor = self.saved;
        self.cursor.row = self.cursor.row.min(self.rows - 1);
        self.cursor.col = self.cursor.col.min(self.cols - 1);
    }

    fn clear_line(&mut self, mode: LineClearMode) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let blank = self.blank();
        let range = match mode {
            LineClearMode::Right => col..self.cols,
            LineClearMode::Left => 0..col + 1,
            LineClearMode::All => 0..self.cols,
        };

        for at in range {
            self.unhalve(row, at);
            self.screen[row].cells[at] = blank;
        }

        if !matches!(mode, LineClearMode::Left) {
            self.screen[row].wrapped = false;
        }

        self.cursor.pending_wrap = false;
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        let row = self.cursor.row;
        let blank = self.blank();

        match mode {
            ClearMode::Below => {
                self.clear_line(LineClearMode::Right);

                for line in &mut self.screen[row + 1..] {
                    *line = Row::filled(self.cols, blank);
                }
            }
            ClearMode::Above => {
                self.clear_line(LineClearMode::Left);

                for line in &mut self.screen[..row] {
                    *line = Row::filled(self.cols, blank);
                }
            }
            ClearMode::All => {
                for line in &mut self.screen {
                    *line = Row::filled(self.cols, blank);
                }
            }
            ClearMode::Saved => {
                self.dropped += self.history.len() as u64;
                self.history.clear();
            }
        }

        self.cursor.pending_wrap = false;
    }

    fn clear_tabs(&mut self, mode: TabulationClearMode) {
        match mode {
            TabulationClearMode::Current => {
                let col = self.cursor.col;
                self.tabs[col] = false;
            }
            TabulationClearMode::All => self.tabs.fill(false),
        }
    }

    fn set_tabs(&mut self, interval: u16) {
        let interval = usize::from(interval.max(1));

        self.tabs = (0..self.cols)
            .map(|col| col % interval == 0 && col > 0)
            .collect();
    }

    fn reset_state(&mut self) {
        let history = std::mem::take(&mut self.history);
        let (dropped, bells) = (self.dropped, self.bells);
        let answers = std::mem::take(&mut self.answers);

        *self =
            Grid::new(self.cols as u16, self.rows as u16, self.history_limit);
        self.history = history;
        self.dropped = dropped;
        self.bells = bells;
        self.answers = answers;
    }

    fn reverse_index(&mut self) {
        if self.cursor.row == self.top {
            self.scroll_region_down(self.top, 1);
        } else {
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }

        self.cursor.pending_wrap = false;
    }

    fn terminal_attribute(&mut self, attr: Attr) {
        let pen = &mut self.cursor.pen;

        match attr {
            Attr::Reset => {
                *pen = Cell::default();
            }
            Attr::Bold => pen.flags.insert(Flags::BOLD),
            Attr::Dim => pen.flags.insert(Flags::DIM),
            Attr::Italic => pen.flags.insert(Flags::ITALIC),
            Attr::Underline
            | Attr::DoubleUnderline
            | Attr::Undercurl
            | Attr::DottedUnderline
            | Attr::DashedUnderline => pen.flags.insert(Flags::UNDERLINE),
            Attr::Reverse => pen.flags.insert(Flags::INVERSE),
            Attr::Hidden => pen.flags.insert(Flags::HIDDEN),
            Attr::Strike => pen.flags.insert(Flags::STRIKE),
            Attr::CancelBold => pen.flags.remove(Flags::BOLD),
            Attr::CancelBoldDim => {
                pen.flags.remove(Flags::BOLD);
                pen.flags.remove(Flags::DIM);
            }
            Attr::CancelItalic => pen.flags.remove(Flags::ITALIC),
            Attr::CancelUnderline => pen.flags.remove(Flags::UNDERLINE),
            Attr::CancelReverse => pen.flags.remove(Flags::INVERSE),
            Attr::CancelHidden => pen.flags.remove(Flags::HIDDEN),
            Attr::CancelStrike => pen.flags.remove(Flags::STRIKE),
            Attr::Foreground(color) => pen.fg = Grid::color(color),
            Attr::Background(color) => pen.bg = Grid::color(color),
            // Blinking and the color of the underline are not drawn
            _ => {}
        }
    }

    fn set_mode(&mut self, mode: Mode) {
        match mode {
            Mode::Named(NamedMode::Insert) => self.insert = true,
            Mode::Named(NamedMode::LineFeedNewLine) => {
                self.linefeed_newline = true;
            }
            Mode::Unknown(_) => {}
        }
    }

    fn unset_mode(&mut self, mode: Mode) {
        match mode {
            Mode::Named(NamedMode::Insert) => self.insert = false,
            Mode::Named(NamedMode::LineFeedNewLine) => {
                self.linefeed_newline = false;
            }
            Mode::Unknown(_) => {}
        }
    }

    fn set_private_mode(&mut self, mode: PrivateMode) {
        self.private_mode(mode, true);
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        self.private_mode(mode, false);
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        let bottom = bottom.unwrap_or(self.rows).min(self.rows);
        let top = top.max(1) - 1;

        // A region is at least two lines
        if top + 1 < bottom {
            self.top = top;
            self.bottom = bottom;
        } else {
            self.top = 0;
            self.bottom = self.rows;
        }

        self.goto(0, 0);
    }

    fn set_keypad_application_mode(&mut self) {
        self.application_keypad = true;
    }

    fn unset_keypad_application_mode(&mut self) {
        self.application_keypad = false;
    }

    fn set_active_charset(&mut self, index: CharsetIndex) {
        self.cursor.charset = match index {
            CharsetIndex::G0 => 0,
            CharsetIndex::G1 => 1,
            CharsetIndex::G2 => 2,
            CharsetIndex::G3 => 3,
        };
    }

    fn configure_charset(
        &mut self,
        index: CharsetIndex,
        charset: StandardCharset,
    ) {
        let index = match index {
            CharsetIndex::G0 => 0,
            CharsetIndex::G1 => 1,
            CharsetIndex::G2 => 2,
            CharsetIndex::G3 => 3,
        };

        self.cursor.charsets[index] = charset;
    }

    fn decaln(&mut self) {
        let cell = Cell {
            c: 'E',
            ..Cell::default()
        };

        for row in &mut self.screen {
            *row = Row::filled(self.cols, cell);
        }
    }

    fn text_area_size_chars(&mut self) {
        let answer = format!("\x1b[8;{};{}t", self.rows, self.cols);

        self.answers.extend_from_slice(answer.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vte::ansi::{Processor, Timeout};

    /// No synchronized updates in these tests: what is fed is on the grid.
    #[derive(Default)]
    struct Never;

    impl Timeout for Never {
        fn set_timeout(&mut self, _: std::time::Duration) {}
        fn clear_timeout(&mut self) {}
        fn pending_timeout(&self) -> bool {
            false
        }
    }

    fn fed(cols: u16, rows: u16, bytes: &[u8]) -> Grid {
        let mut grid = Grid::new(cols, rows, 100);
        feed(&mut grid, bytes);
        grid
    }

    fn feed(grid: &mut Grid, bytes: &[u8]) {
        let mut parser: Processor<Never> = Processor::new();
        parser.advance(grid, bytes);
    }

    fn screen(grid: &Grid) -> Vec<String> {
        (grid.history()..grid.total_lines())
            .map(|index| grid.line(index).unwrap().text(0, usize::MAX))
            .collect()
    }

    #[test]
    fn text_goes_where_the_cursor_is_and_a_line_feed_keeps_the_column() {
        let grid = fed(20, 4, b"one\r\ntwo\nthree");

        assert_eq!(screen(&grid), ["one", "two", "   three", ""]);
        assert_eq!(grid.cursor(), (2, 8));
    }

    #[test]
    fn a_full_line_wraps_and_copies_without_a_break() {
        let grid = fed(10, 4, b"0123456789abc\r\nnext");

        assert_eq!(screen(&grid), ["0123456789", "abc", "next", ""]);
        assert!(grid.line(0).unwrap().wrapped);
        assert_eq!(grid.text(0, 3), "0123456789abc\nnext");
    }

    #[test]
    fn the_last_column_waits_for_the_next_character_before_it_wraps() {
        let mut grid = fed(5, 3, b"abcde");

        assert_eq!(grid.cursor(), (0, 4), "still on the first line");
        feed(&mut grid, b"\rX");
        assert_eq!(screen(&grid)[0], "Xbcde", "a return undoes the wait");

        // Without wrapping the last column is written over
        let grid = fed(5, 3, b"\x1b[?7labcdefg");
        assert_eq!(screen(&grid), ["abcdg", "", ""]);
    }

    #[test]
    fn what_scrolls_off_the_top_is_history_up_to_the_limit() {
        let mut grid = Grid::new(10, 3, 4);

        for n in 1..=10 {
            feed(&mut grid, format!("line {n}\r\n").as_bytes());
        }

        assert_eq!(grid.history(), 4);
        assert_eq!(grid.forgotten(), 4);
        assert_eq!(grid.line(0).unwrap().text(0, 99), "line 5");
        assert_eq!(screen(&grid), ["line 9", "line 10", ""]);
        assert_eq!(grid.total_lines(), 7);
        assert_eq!(grid.used_lines(), 7, "the cursor's line counts");
    }

    #[test]
    fn the_cursor_moves_and_stays_on_the_screen() {
        let grid = fed(10, 5, b"\x1b[3;4HX\x1b[2AY\x1b[10BZ\x1b[99DW\x1b[HV");

        assert_eq!(
            screen(&grid),
            ["V   Y", "", "   X", "", "W    Z"],
            "{:?}",
            screen(&grid)
        );
    }

    #[test]
    fn erasing_in_the_line_and_on_the_screen() {
        let mut grid = fed(10, 3, b"aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc");

        feed(&mut grid, b"\x1b[2;5H\x1b[K");
        assert_eq!(screen(&grid), ["aaaaaaaaaa", "bbbb", "cccccccccc"]);
        feed(&mut grid, b"\x1b[1K");
        assert_eq!(screen(&grid)[1], "");
        feed(&mut grid, b"\x1b[J");
        assert_eq!(screen(&grid), ["aaaaaaaaaa", "", ""]);
        feed(&mut grid, b"\x1b[2J");
        assert_eq!(screen(&grid), ["", "", ""]);
    }

    #[test]
    fn characters_and_lines_are_inserted_and_deleted() {
        let mut grid = fed(10, 4, b"abcdef\r\nsecond\r\nthird");

        feed(&mut grid, b"\x1b[1;3H\x1b[2P");
        assert_eq!(screen(&grid)[0], "abef");
        feed(&mut grid, b"\x1b[2@");
        assert_eq!(screen(&grid)[0], "ab  ef");
        feed(&mut grid, b"\x1b[4hXY\x1b[4l");
        assert_eq!(screen(&grid)[0], "abXY  ef", "insert mode");
        feed(&mut grid, b"\x1b[3X");
        assert_eq!(screen(&grid)[0], "abXY   f");
        feed(&mut grid, b"\x1b[L");
        assert_eq!(screen(&grid), ["", "abXY   f", "second", "third"]);
        feed(&mut grid, b"\x1b[2M");
        assert_eq!(screen(&grid), ["second", "third", "", ""]);
        assert_eq!(grid.history(), 0, "deleted lines are not history");
    }

    #[test]
    fn a_scrolling_region_scrolls_alone() {
        let mut grid = fed(10, 5, b"top\r\na\r\nb\r\nc\r\nbottom");

        // Lines 2 to 4; the cursor goes home; then to the region's end
        feed(&mut grid, b"\x1b[2;4r\x1b[4;1H\nd");
        assert_eq!(screen(&grid), ["top", "b", "c", "d", "bottom"]);
        assert_eq!(grid.history(), 0, "the region is not at the top");

        // Reverse index at the top of the region
        feed(&mut grid, b"\x1b[2;1H\x1bMz");
        assert_eq!(screen(&grid), ["top", "z", "b", "c", "bottom"]);
    }

    #[test]
    fn colors_and_flags_are_kept_per_cell() {
        let grid = fed(
            20,
            2,
            b"\x1b[1;31mR\x1b[0;4;48;5;200mU\x1b[38;2;1;2;3;7mT\x1b[mP\x1b[92mB",
        );
        let row = grid.line(0).unwrap();

        assert_eq!(row.cells[0].fg, Color::Indexed(1));
        assert!(row.cells[0].flags.contains(Flags::BOLD));
        assert_eq!(row.cells[1].bg, Color::Indexed(200));
        assert!(row.cells[1].flags.contains(Flags::UNDERLINE));
        assert!(!row.cells[1].flags.contains(Flags::BOLD));
        assert_eq!(row.cells[2].fg, Color::Rgb(1, 2, 3));
        assert!(row.cells[2].flags.contains(Flags::INVERSE));
        assert_eq!(
            row.cells[3],
            Cell {
                c: 'P',
                ..Cell::default()
            }
        );
        assert_eq!(row.cells[4].fg, Color::Indexed(10), "bright green");
    }

    #[test]
    fn erasing_keeps_the_background_of_the_pen() {
        let grid = fed(6, 2, b"\x1b[44m\x1b[2Jx");

        assert_eq!(grid.line(1).unwrap().cells[3].bg, Color::Indexed(4));
        assert!(!grid.line(1).unwrap().cells[3].is_blank());
    }

    #[test]
    fn the_alternate_screen_comes_and_goes_and_leaves_no_history() {
        let mut grid = fed(10, 3, b"shell$ vi\r\n");

        feed(
            &mut grid,
            b"\x1b[?1049h\x1b[2J\x1b[Hfull\r\nscreen\r\n1\r\n2\r\n3",
        );
        assert!(grid.alternate_screen());
        assert_eq!(screen(&grid), ["1", "2", "3"]);
        assert_eq!(grid.history(), 0);

        feed(&mut grid, b"\x1b[?1049l");
        assert!(!grid.alternate_screen());
        assert_eq!(screen(&grid), ["shell$ vi", "", ""]);
        assert_eq!(grid.cursor(), (1, 0), "the cursor is where it was");
    }

    #[test]
    fn modes_the_keys_depend_on() {
        let mut grid = fed(10, 3, b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?25l");

        assert!(grid.application_cursor() && grid.application_keypad());
        assert!(grid.bracketed_paste() && !grid.cursor_visible());

        feed(&mut grid, b"\x1b[?1l\x1b>\x1b[?2004l\x1b[?25h");
        assert!(!grid.application_cursor() && !grid.application_keypad());
        assert!(!grid.bracketed_paste() && grid.cursor_visible());
    }

    #[test]
    fn tabs_stop_every_eight_columns_and_where_they_are_set() {
        let mut grid = fed(30, 2, b"a\tb\tc");

        assert_eq!(screen(&grid)[0], "a       b       c");
        feed(&mut grid, b"\r\n\x1b[3g\x1b[5G\x1bH\rx\ty\tz");
        assert_eq!(screen(&grid)[1], format!("x   y{}z", " ".repeat(24)));
    }

    #[test]
    fn a_wide_character_takes_two_cells() {
        let mut grid = fed(6, 2, "a\u{4e16}b".as_bytes());
        let row = grid.line(0).unwrap();

        assert!(row.cells[1].flags.contains(Flags::WIDE));
        assert!(row.cells[2].flags.contains(Flags::WIDE_SPACER));
        assert_eq!(row.cells[3].c, 'b');
        assert_eq!(row.text(0, 99), "a\u{4e16}b");

        // Writing over one half removes the other
        feed(&mut grid, b"\x1b[1;3Hx");
        assert_eq!(grid.line(0).unwrap().text(0, 99), "a xb");

        // It does not begin in the last column
        let grid = fed(4, 2, "abc\u{4e16}".as_bytes());
        assert_eq!(screen(&grid), ["abc", "\u{4e16}"]);
    }

    #[test]
    fn line_drawing_characters_are_mapped() {
        let grid = fed(10, 1, b"\x1b(0lqk\x1b(Blqk");

        assert_eq!(screen(&grid)[0], "\u{250c}\u{2500}\u{2510}lqk");
    }

    #[test]
    fn questions_are_answered_once() {
        let mut grid = fed(10, 5, b"\x1b[3;4H\x1b[6n\x1b[5n\x1b[c");

        assert_eq!(grid.take_answers(), b"\x1b[3;4R\x1b[0n\x1b[?6c");
        assert!(grid.take_answers().is_empty());
    }

    #[test]
    fn the_title_and_the_bell() {
        let grid = fed(10, 2, b"\x1b]0;make test\x07\x07\x07");

        assert_eq!(grid.title(), Some("make test"));
        assert_eq!(grid.bells(), 2);
    }

    #[test]
    fn a_sequence_cut_in_two_is_one_sequence() {
        let mut grid = Grid::new(10, 2, 0);
        let mut parser: Processor<Never> = Processor::new();

        parser.advance(&mut grid, b"\x1b[3");
        parser.advance(&mut grid, b"1mred \xe2\x82");
        parser.advance(&mut grid, b"\xac");

        assert_eq!(screen(&grid)[0], "red \u{20ac}");
        assert_eq!(grid.line(0).unwrap().cells[0].fg, Color::Indexed(1));
    }

    #[test]
    fn a_selection_reads_from_cell_to_cell_in_either_direction() {
        let grid = fed(12, 4, b"first line\r\nsecond line\r\nthird");

        assert_eq!(grid.selected((0, 6), (1, 5)), "line\nsecond");
        assert_eq!(grid.selected((1, 5), (0, 6)), "line\nsecond");
        assert_eq!(grid.selected((2, 0), (2, 50)), "third");
        assert_eq!(grid.selected((3, 0), (9, 9)), "");
    }

    #[test]
    fn a_new_size_keeps_the_text_and_the_cursor_on_the_screen() {
        let mut grid = fed(10, 4, b"one\r\ntwo\r\nthree\r\nfour");

        grid.resize(6, 2);
        assert_eq!(screen(&grid), ["three", "four"]);
        assert_eq!(grid.history(), 2);
        assert_eq!(grid.cursor(), (1, 4));

        grid.resize(8, 3);
        assert_eq!(screen(&grid), ["three", "four", ""]);
        feed(&mut grid, b"!\r\nfive\r\nsix");
        assert_eq!(screen(&grid), ["four!", "five", "six"]);

        // Short of everything, it is still a grid
        grid.resize(0, 0);
        assert_eq!((grid.cols(), grid.rows()), (2, 1));
    }

    #[test]
    fn a_reset_clears_the_screen_and_keeps_the_history() {
        let mut grid = fed(10, 2, b"a\r\nb\r\nc\x1b[31m\x1b[?25l");

        feed(&mut grid, b"\x1bc");
        assert_eq!(screen(&grid), ["", ""]);
        assert_eq!(grid.history(), 1);
        assert!(grid.cursor_visible());
        assert_eq!(grid.cursor(), (0, 0));
    }

    #[test]
    fn bytes_that_are_no_text_do_no_harm() {
        let mut grid = Grid::new(8, 3, 10);
        let mut bytes = Vec::new();

        // Every byte, and every escape with every final byte
        for b in 0..=255u8 {
            bytes.push(b);
            bytes.extend_from_slice(&[0x1b, b'[', b'9', b'9', b'9', b]);
            bytes.extend_from_slice(&[0x1b, b]);
        }

        feed(&mut grid, &bytes);
        feed(
            &mut grid,
            b"\x1b[999;999H\x1b[999@\x1b[999P\x1b[999L\x1b[999M",
        );
        feed(&mut grid, b"\x1b[999S\x1b[999T\x1b[0;0r\x1b[5;2r\x1b[999X!");

        assert_eq!(grid.rows(), 3);
        assert!(grid.cursor().0 < 3 && grid.cursor().1 < 8);
    }
}
