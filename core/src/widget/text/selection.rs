//! Select a part of a piece of text with the pointer and the keyboard.
//!
//! A [`Selection`] is the state of it: `Text` keeps one when it is
//! selectable, and so does `rich_text` of `iced_widget`. A widget that
//! draws a paragraph can keep one too: hand it the events
//! ([`Selection::update`]), let it draw behind the text
//! ([`Selection::draw`]) and let it answer a container that selects over
//! several texts ([`Selection::operate`], with a [`Part`]).
use crate::clipboard::{self, Clipboard};
use crate::keyboard;
use crate::keyboard::key;
use crate::mouse;
use crate::renderer;
use crate::text::Paragraph;
use crate::touch;
use crate::widget::Operation;
use crate::widget::tree::{self, Tree};
use crate::{Color, Event, Point, Rectangle, Shell, Vector};

/// The selection in a piece of text: two offsets in bytes of its content,
/// where it began and where it is now.
#[derive(Debug, Clone, Copy, Default)]
pub struct Selection {
    anchor: usize,
    head: usize,
    is_dragging: bool,
    /// Whether the last press of the pointer was on this text: the keys
    /// (Shift with the arrows, select all) are for it.
    is_active: bool,
    last_click: Option<mouse::Click>,
}

/// What a selectable text tells whoever asks, and what it can be told to
/// select: it travels through [`Operation::custom`] in the `operate` of the
/// text. A container that selects over several texts (the `selection_area`
/// of `iced_widget`) reads [`bounds`](Self::bounds) and
/// [`selected`](Self::selected), and sets [`command`](Self::command).
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Where the text is, as its layout says.
    pub bounds: Rectangle,
    /// What is selected in it, if anything.
    pub selected: Option<String>,
    /// What the text is to select, set by the [`Operation`].
    pub command: Option<Command>,
}

/// What a selectable text is told to select. The points are in the space
/// of [`Part::bounds`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Nothing.
    Clear,
    /// Everything.
    All,
    /// From the first point to the second.
    Between(Point, Point),
    /// From the point to its end.
    From(Point),
    /// From its start to the point.
    Until(Point),
}

impl Selection {
    /// Creates the [`Tree`] that holds a [`Selection`]: a selectable text
    /// keeps it as a child of its own, so that its state stays what it
    /// was.
    pub fn tree() -> Tree {
        Tree {
            tag: tree::Tag::of::<Selection>(),
            state: tree::State::new(Selection::default()),
            children: Vec::new(),
        }
    }

    /// Makes the children of the tree of a text hold a [`Selection`], or
    /// nothing when the text is not selectable.
    pub fn diff(tree: &mut Tree, is_selectable: bool) {
        if !is_selectable {
            tree.children.clear();
        } else if tree.children.len() != 1
            || tree.children[0].tag != tree::Tag::of::<Selection>()
        {
            tree.children = vec![Selection::tree()];
        }
    }

    /// Where the selection began, in bytes of the content.
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// Where the selection is now: the end that moves.
    pub fn head(&self) -> usize {
        self.head
    }

    /// The selected range, in order; `None` when nothing is selected.
    pub fn range(&self) -> Option<(usize, usize)> {
        let start = self.anchor.min(self.head);
        let end = self.anchor.max(self.head);

        (start < end).then_some((start, end))
    }

    /// The selected part of the given content, if any. A selection that no
    /// longer fits the content (it changed since) selects nothing.
    pub fn selected<'a>(&self, content: &'a str) -> Option<&'a str> {
        let (start, end) = self.range()?;

        content.get(start..end.min(content.len()))
    }

    /// Selects from one offset to another.
    pub fn select(&mut self, anchor: usize, head: usize) {
        self.anchor = anchor;
        self.head = head;
        self.is_dragging = false;
    }

    /// Processes an event for a text with the given paragraph and content,
    /// laid out in `bounds` and drawn from `origin`.
    ///
    /// A drag selects, a double click a word, a triple click everything; a
    /// click elsewhere drops the selection. The copy key of the platform
    /// copies. After a click on the text, Shift with the arrows (by word
    /// with Ctrl, or Option on macOS), Home and End moves the end of the
    /// selection, and Ctrl+A (Cmd+A) selects everything. What is selected
    /// is also the primary selection.
    pub fn update<P, Message>(
        &mut self,
        event: &Event,
        paragraph: &P,
        content: &str,
        bounds: Rectangle,
        origin: Point,
        cursor: mouse::Cursor,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) where
        P: Paragraph,
    {
        let offset_at = |position: Point| {
            paragraph
                .hit_offset(position - Vector::new(origin.x, origin.y))
                .map(|offset| floor_char_boundary(content, offset))
        };

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                let before = self.range();

                if let Some(position) = cursor.position_over(bounds)
                    && let Some(offset) = offset_at(position)
                {
                    let click = mouse::Click::new(
                        position,
                        mouse::Button::Left,
                        self.last_click,
                    );

                    match click.kind() {
                        mouse::click::Kind::Single => {
                            self.anchor = offset;
                            self.head = offset;
                            self.is_dragging = true;
                        }
                        mouse::click::Kind::Double => {
                            let (start, end) = word_around(content, offset);

                            self.select(start, end);
                        }
                        mouse::click::Kind::Triple => {
                            self.select(0, content.len());
                        }
                    }

                    self.is_active = true;
                    self.last_click = Some(click);
                } else {
                    // A click elsewhere drops the selection
                    *self = Self::default();
                }

                if self.range() != before {
                    if !self.is_dragging
                        && let Some(selected) = self.selected(content)
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
                if self.is_dragging =>
            {
                if let Some(position) = cursor.position()
                    && let Some(offset) = offset_at(position)
                    && offset != self.head
                {
                    self.head = offset;
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(
                touch::Event::FingerLifted { .. }
                | touch::Event::FingerLost { .. },
            ) if self.is_dragging => {
                self.is_dragging = false;

                if let Some(selected) = self.selected(content) {
                    clipboard
                        .write(clipboard::Kind::Primary, selected.to_owned());
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) if !shell.is_event_captured() => {
                let latin = key.to_latin(*physical_key);

                if modifiers.command() && latin == Some('c') {
                    if let Some(selected) = self.selected(content) {
                        clipboard.write(
                            clipboard::Kind::Standard,
                            selected.to_owned(),
                        );

                        shell.capture_event();
                    }

                    return;
                }

                if !self.is_active {
                    return;
                }

                let before = (self.anchor, self.head);

                if modifiers.command() && latin == Some('a') {
                    self.select(0, content.len());
                } else if modifiers.shift()
                    && let keyboard::Key::Named(named) = key
                    && let Some(head) =
                        self.moved(paragraph, content, *named, *modifiers)
                {
                    self.head = head;
                    self.is_dragging = false;
                } else {
                    return;
                }

                // The key was for this text, whether it moved anything
                shell.capture_event();

                if (self.anchor, self.head) != before {
                    if let Some(selected) = self.selected(content) {
                        clipboard.write(
                            clipboard::Kind::Primary,
                            selected.to_owned(),
                        );
                    }

                    shell.request_redraw();
                }
            }
            _ => {}
        }
    }

    /// Where a key takes the end of the selection, if it is a key that
    /// moves it.
    fn moved<P: Paragraph>(
        &self,
        paragraph: &P,
        content: &str,
        named: key::Named,
        modifiers: keyboard::Modifiers,
    ) -> Option<usize> {
        let head = floor_char_boundary(content, self.head);
        let is_word = |c: char| c.is_alphanumeric() || c == '_';

        // The line as it is drawn, through where the caret is
        let caret = caret(paragraph, content, head);
        let hit = |x: f32, y: f32| {
            paragraph
                .hit_offset(Point::new(x, y))
                .map(|offset| floor_char_boundary(content, offset))
        };

        Some(match named {
            key::Named::ArrowLeft if modifiers.jump() => {
                let before = &content[..head];
                let words = before.trim_end_matches(|c| !is_word(c));

                words
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| is_word(*c))
                    .last()
                    .map_or(0, |(i, _)| i)
            }
            key::Named::ArrowRight if modifiers.jump() => {
                let after = &content[head..];
                let words = after.trim_start_matches(|c| !is_word(c));
                let skipped = after.len() - words.len();

                head + skipped
                    + words
                        .char_indices()
                        .find(|(_, c)| !is_word(*c))
                        .map_or(words.len(), |(i, _)| i)
            }
            key::Named::ArrowLeft => content[..head]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i),
            key::Named::ArrowRight => content[head..]
                .chars()
                .next()
                .map_or(head, |c| head + c.len_utf8()),
            key::Named::Home if modifiers.command() => 0,
            key::Named::End if modifiers.command() => content.len(),
            key::Named::Home => caret
                .and_then(|caret| hit(-1.0e4, caret.center_y()))
                .unwrap_or_else(|| {
                    content[..head].rfind('\n').map_or(0, |i| i + 1)
                }),
            key::Named::End => caret
                .and_then(|caret| hit(1.0e6, caret.center_y()))
                .unwrap_or_else(|| {
                    content[head..]
                        .find('\n')
                        .map_or(content.len(), |i| head + i)
                }),
            key::Named::ArrowUp => caret
                .filter(|caret| caret.y >= caret.height / 2.0)
                .and_then(|caret| hit(caret.x, caret.center_y() - caret.height))
                .filter(|offset| *offset < head)
                .unwrap_or(0),
            key::Named::ArrowDown => caret
                .and_then(|caret| hit(caret.x, caret.center_y() + caret.height))
                .filter(|offset| *offset > head)
                .unwrap_or(content.len()),
            _ => return None,
        })
    }

    /// Draws what is selected behind the text: call it before the text is
    /// drawn.
    pub fn draw<Renderer, P>(
        &self,
        renderer: &mut Renderer,
        paragraph: &P,
        origin: Point,
        color: Color,
        viewport: &Rectangle,
    ) where
        Renderer: renderer::Renderer,
        P: Paragraph,
    {
        let Some((start, end)) = self.range() else {
            return;
        };

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

    /// Tells the given [`Operation`] of this text as a [`Part`], and
    /// selects what the [`Operation`] asks for.
    pub fn operate<P: Paragraph>(
        &mut self,
        operation: &mut dyn Operation,
        paragraph: &P,
        content: &str,
        bounds: Rectangle,
        origin: Point,
    ) {
        let mut part = Part {
            bounds,
            selected: self.selected(content).map(str::to_owned),
            command: None,
        };

        operation.custom(None, bounds, &mut part);

        let Some(command) = part.command else {
            return;
        };

        // The nearest place in the text to a point, wherever it is
        let near = |point: Point| {
            let point = point - Vector::new(origin.x, origin.y);

            paragraph
                .hit_offset(point)
                .map(|offset| floor_char_boundary(content, offset))
                .unwrap_or(if point.y < 0.0 { 0 } else { content.len() })
        };

        let (anchor, head) = match command {
            Command::Clear => (0, 0),
            Command::All => (0, content.len()),
            Command::Between(from, to) => (near(from), near(to)),
            Command::From(point) => (near(point), content.len()),
            Command::Until(point) => (0, near(point)),
        };

        self.select(anchor, head);

        // Another text has the keys now, or all of them have
        if !matches!(command, Command::Between(..)) {
            self.is_active = false;
        }
    }
}

/// Where the caret stands before the character at the given offset: a
/// rectangle without width, as high as its line, in the space of the
/// paragraph. `None` on a line without anything to measure.
fn caret<P: Paragraph>(
    paragraph: &P,
    content: &str,
    offset: usize,
) -> Option<Rectangle> {
    let next = content[offset..].chars().next();
    let previous = content[..offset].chars().next_back();

    // Before the character that follows; a space a line was wrapped at has
    // no width, then after the character before
    let before_next = next.filter(|c| *c != '\n').and_then(|c| {
        let bounds = paragraph.selection_bounds(offset, offset + c.len_utf8());

        bounds.first().map(|first| Rectangle {
            width: 0.0,
            ..*first
        })
    });

    before_next.or_else(|| {
        let c = previous.filter(|c| *c != '\n')?;
        let bounds = paragraph.selection_bounds(offset - c.len_utf8(), offset);

        bounds.last().map(|last| Rectangle {
            x: last.x + last.width,
            width: 0.0,
            ..*last
        })
    })
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
