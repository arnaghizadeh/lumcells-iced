//! Run a simulation of your application without side effects.
use crate::core;
use crate::core::clipboard;
use crate::core::event;
use crate::core::keyboard;
use crate::core::mouse;
use crate::core::theme;
use crate::core::time;
use crate::core::widget;
use crate::core::window;
use crate::core::{Element, Event, Font, Point, Settings, Size, SmolStr};
use crate::renderer;
use crate::runtime::UserInterface;
use crate::runtime::focus;
use crate::runtime::user_interface;
use crate::selector::Bounded;
use crate::{Error, Selector};

use std::borrow::Cow;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A user interface that can be interacted with and inspected programmatically.
pub struct Simulator<
    'a,
    Message,
    Theme = core::Theme,
    Renderer = renderer::Renderer,
> {
    raw: UserInterface<'a, Message, Theme, Renderer>,
    renderer: Renderer,
    size: Size,
    cursor: mouse::Cursor,
    messages: Vec<Message>,
    clipboard: Clipboard,
}

impl<'a, Message, Theme, Renderer> Simulator<'a, Message, Theme, Renderer>
where
    Theme: theme::Base,
    Renderer: core::Renderer + core::renderer::Headless,
{
    /// Creates a new [`Simulator`] with default [`Settings`] and a default size (1024x768).
    pub fn new(
        element: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self::with_settings(Settings::default(), element)
    }

    /// Creates a new [`Simulator`] with the given [`Settings`] and a default size (1024x768).
    pub fn with_settings(
        settings: Settings,
        element: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self::with_size(settings, window::Settings::default().size, element)
    }

    /// Creates a new [`Simulator`] with the given [`Settings`] and size.
    pub fn with_size(
        settings: Settings,
        size: impl Into<Size>,
        element: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        let size = size.into();

        let default_font = match settings.default_font {
            Font::DEFAULT => Font::with_name("Fira Sans"),
            _ => settings.default_font,
        };

        for font in settings.fonts {
            load_font(font).expect("Font must be valid");
        }

        let mut renderer = {
            let backend = env::var("ICED_TEST_BACKEND").ok();

            crate::futures::futures::executor::block_on(Renderer::new(
                default_font,
                settings.default_text_size,
                backend.as_deref(),
            ))
            .expect("Create new headless renderer")
        };

        let raw = UserInterface::build(
            element,
            size,
            user_interface::Cache::default(),
            &mut renderer,
        );

        Simulator {
            raw,
            renderer,
            size,
            cursor: mouse::Cursor::Unavailable,
            messages: Vec::new(),
            clipboard: Clipboard::default(),
        }
    }

    /// Finds the target of the given widget [`Selector`] in the [`Simulator`].
    pub fn find<S>(&mut self, selector: S) -> Result<S::Output, Error>
    where
        S: Selector + Send,
        S::Output: Clone + Send,
    {
        use widget::Operation;

        let description = selector.description();
        let mut operation = selector.find();

        self.raw.operate(
            &self.renderer,
            &mut widget::operation::black_box(&mut operation),
        );

        match operation.finish() {
            widget::operation::Outcome::Some(output) => {
                output.ok_or(Error::SelectorNotFound {
                    selector: description,
                })
            }
            _ => Err(Error::SelectorNotFound {
                selector: description,
            }),
        }
    }

    /// Points the mouse cursor at the given position in the [`Simulator`].
    ///
    /// This does _not_ produce mouse movement events!
    pub fn point_at(&mut self, position: impl Into<Point>) {
        self.cursor = mouse::Cursor::Available(position.into());
    }

    /// Clicks the [`Bounded`] target found by the given [`Selector`], if any.
    ///
    /// This consists in:
    /// - Pointing the mouse cursor at the center of the [`Bounded`] target.
    /// - Simulating a [`click`].
    pub fn click<S>(&mut self, selector: S) -> Result<S::Output, Error>
    where
        S: Selector + Send,
        S::Output: Bounded + Clone + Send + Sync + 'static,
    {
        let target = self.find(selector)?;

        let Some(visible_bounds) = target.visible_bounds() else {
            return Err(Error::TargetNotVisible {
                target: Arc::new(target),
            });
        };

        self.point_at(visible_bounds.center());

        let _ = self.simulate(click());

        Ok(target)
    }

    /// Simulates a key press, followed by a release, in the [`Simulator`].
    pub fn tap_key(&mut self, key: impl Into<keyboard::Key>) -> event::Status {
        self.simulate(tap_key(key, None))
            .first()
            .copied()
            .unwrap_or(event::Status::Ignored)
    }

    /// Simulates a user typing in the keyboard the given text in the [`Simulator`].
    pub fn typewrite(&mut self, text: &str) -> event::Status {
        let statuses = self.simulate(typewrite(text));

        statuses
            .into_iter()
            .fold(event::Status::Ignored, event::Status::merge)
    }

    /// Simulates the given raw sequence of events in the [`Simulator`].
    pub fn simulate(
        &mut self,
        events: impl IntoIterator<Item = Event>,
    ) -> Vec<event::Status> {
        let events: Vec<Event> = events.into_iter().collect();

        let (_state, mut statuses) = self.raw.update(
            &events,
            self.cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut self.messages,
        );

        // Like a shell does: Tab and Shift+Tab move the keyboard focus when
        // no widget captured the key
        for (event, status) in events.iter().zip(statuses.iter_mut()) {
            if *status == event::Status::Ignored
                && let Some(direction) = focus::direction(event)
                && focus::shift(&mut self.raw, &self.renderer, direction)
            {
                *status = event::Status::Captured;
            }
        }

        statuses
    }

    /// Builds the tree that assistive technology would be given for the
    /// interface in the [`Simulator`], under a window with the given title.
    pub fn accessibility(
        &mut self,
        title: &str,
    ) -> crate::runtime::accessibility::Tree {
        crate::runtime::accessibility::Tree::of(
            &mut self.raw,
            &self.renderer,
            title,
            core::Rectangle::with_size(self.size),
        )
    }

    /// Presses the node with the given index of `tree` (which
    /// [`accessibility`](Self::accessibility) gave) the way a screen reader
    /// does: what is scrolled out of view is scrolled into view first,
    /// then it is clicked where it is drawn. Returns whether it was
    /// pressed.
    pub fn accessibility_press(
        &mut self,
        tree: &crate::runtime::accessibility::Tree,
        index: usize,
    ) -> bool {
        crate::runtime::accessibility::press(
            tree,
            index,
            &mut self.raw,
            &mut self.renderer,
            &mut self.clipboard,
            &mut self.messages,
        )
    }

    /// Scrolls the node with the given index of `tree` into view the way
    /// a screen reader asks for it. Returns whether anything was scrolled.
    pub fn accessibility_reveal(
        &mut self,
        tree: &crate::runtime::accessibility::Tree,
        index: usize,
    ) -> bool {
        crate::runtime::accessibility::scroll_into_view(
            tree,
            index,
            &mut self.raw,
            &self.renderer,
        )
    }

    /// Sets the number of the node with the given index of `tree` (a
    /// slider) the way a screen reader does. Returns whether it is one.
    pub fn accessibility_set_number(
        &mut self,
        tree: &crate::runtime::accessibility::Tree,
        index: usize,
        number: f32,
    ) -> bool {
        crate::runtime::accessibility::set_number(
            tree,
            index,
            number,
            &mut self.raw,
            &mut self.renderer,
            &mut self.clipboard,
            &mut self.messages,
        )
    }

    /// Scrolls the node with the given index of `tree` (a scroll area) by
    /// the given amount the way a screen reader does. Returns whether it
    /// is one.
    pub fn accessibility_scroll(
        &mut self,
        tree: &crate::runtime::accessibility::Tree,
        index: usize,
        by: core::Vector,
    ) -> bool {
        crate::runtime::accessibility::scroll_by(
            tree,
            index,
            by,
            &mut self.raw,
            &self.renderer,
        )
    }

    /// Returns what the widgets of the [`Simulator`] wrote last to the given
    /// kind of clipboard, if anything.
    ///
    /// The clipboard of a [`Simulator`] is its own: nothing reaches the one
    /// of the system.
    pub fn clipboard(&self, kind: clipboard::Kind) -> Option<&str> {
        match kind {
            clipboard::Kind::Standard => self.clipboard.standard.as_deref(),
            clipboard::Kind::Primary => self.clipboard.primary.as_deref(),
        }
    }

    /// Runs the given widget [`Operation`](widget::Operation) in the
    /// [`Simulator`] (and the ones it chains) and returns its output, if any.
    ///
    /// For instance, `operation::focusable::find_focused()` tells which
    /// widget has the keyboard focus.
    pub fn operate<T>(
        &mut self,
        operation: impl widget::Operation<T> + 'static,
    ) -> Option<T>
    where
        T: Send + 'static,
    {
        use widget::Operation;
        use widget::operation::Outcome;

        let mut current: Box<dyn Operation<T>> = Box::new(operation);

        loop {
            self.raw.operate(
                &self.renderer,
                &mut widget::operation::black_box(current.as_mut()),
            );

            match current.finish() {
                Outcome::None => return None,
                Outcome::Some(output) => return Some(output),
                Outcome::Chain(next) => {
                    current = next;
                }
            }
        }
    }

    /// Draws and takes a [`Snapshot`] of the interface in the [`Simulator`],
    /// at a scale factor of 2.
    pub fn snapshot(&mut self, theme: &Theme) -> Result<Snapshot, Error> {
        self.snapshot_with(theme, 2.0)
    }

    /// Draws and takes a [`Snapshot`] of the interface in the [`Simulator`]
    /// at the given scale factor: `1.0` gives one pixel per logical pixel.
    pub fn snapshot_with(
        &mut self,
        theme: &Theme,
        scale_factor: f32,
    ) -> Result<Snapshot, Error> {
        let base = theme.base();

        let _ = self.raw.update(
            &[Event::Window(window::Event::RedrawRequested(
                time::Instant::now(),
            ))],
            self.cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut self.messages,
        );

        self.raw.draw(
            &mut self.renderer,
            theme,
            &core::renderer::Style {
                text_color: base.text_color,
            },
            self.cursor,
        );

        let physical_size = Size::new(
            (self.size.width * scale_factor).round() as u32,
            (self.size.height * scale_factor).round() as u32,
        );

        let rgba = self.renderer.screenshot(
            physical_size,
            scale_factor,
            base.background_color,
        );

        Ok(Snapshot {
            screenshot: window::Screenshot::new(
                rgba,
                physical_size,
                scale_factor,
            ),
            renderer: self.renderer.name(),
        })
    }

    /// Turns the [`Simulator`] into the sequence of messages produced by any interactions.
    pub fn into_messages(
        self,
    ) -> impl Iterator<Item = Message> + use<Message, Theme, Renderer> {
        self.messages.into_iter()
    }
}

/// The clipboard of a [`Simulator`]: it remembers what was written to it.
#[derive(Debug, Clone, Default)]
struct Clipboard {
    standard: Option<String>,
    primary: Option<String>,
}

impl clipboard::Clipboard for Clipboard {
    fn read(&self, kind: clipboard::Kind) -> Option<String> {
        match kind {
            clipboard::Kind::Standard => self.standard.clone(),
            clipboard::Kind::Primary => self.primary.clone(),
        }
    }

    fn write(&mut self, kind: clipboard::Kind, contents: String) {
        match kind {
            clipboard::Kind::Standard => self.standard = Some(contents),
            clipboard::Kind::Primary => self.primary = Some(contents),
        }
    }
}

/// A frame of a user interface rendered by a [`Simulator`].
#[derive(Debug, Clone)]
pub struct Snapshot {
    screenshot: window::Screenshot,
    renderer: String,
}

impl Snapshot {
    /// Returns the pixels of the [`Snapshot`], as RGBA bytes in rows from
    /// the top, and its size in pixels.
    ///
    /// Nothing is written to disk: a test can look at the pixels itself, or
    /// save them under a name and in a format of its own.
    pub fn into_rgba(self) -> (Vec<u8>, Size<u32>) {
        (self.screenshot.rgba.to_vec(), self.screenshot.size)
    }

    /// Returns the scale factor the [`Snapshot`] was taken at.
    pub fn scale_factor(&self) -> f32 {
        self.screenshot.scale_factor
    }

    /// Returns the name of the renderer that drew the [`Snapshot`]; it is
    /// what [`matches_image`](Self::matches_image) and
    /// [`matches_hash`](Self::matches_hash) add to the name of their files.
    pub fn renderer(&self) -> &str {
        &self.renderer
    }

    /// Compares the [`Snapshot`] with the PNG image found in the given path, returning
    /// `true` if they are identical.
    ///
    /// If the PNG image does not exist, it will be created by the [`Snapshot`] for future
    /// testing and `true` will be returned.
    pub fn matches_image(&self, path: impl AsRef<Path>) -> Result<bool, Error> {
        let path = self.path(path, "png");

        if path.exists() {
            let file = fs::File::open(&path)?;
            let decoder = png::Decoder::new(io::BufReader::new(file));

            let mut reader = decoder.read_info()?;
            let n = reader
                .output_buffer_size()
                .expect("snapshot should fit in memory");
            let mut bytes = vec![0; n];
            let info = reader.next_frame(&mut bytes)?;

            Ok(self.screenshot.rgba == bytes[..info.buffer_size()])
        } else {
            if let Some(directory) = path.parent() {
                fs::create_dir_all(directory)?;
            }

            let file = fs::File::create(path)?;

            let mut encoder = png::Encoder::new(
                file,
                self.screenshot.size.width,
                self.screenshot.size.height,
            );
            encoder.set_color(png::ColorType::Rgba);

            let mut writer = encoder.write_header()?;
            writer.write_image_data(&self.screenshot.rgba)?;
            writer.finish()?;

            Ok(true)
        }
    }

    /// Compares the [`Snapshot`] with the SHA-256 hash file found in the given path, returning
    /// `true` if they are identical.
    ///
    /// If the hash file does not exist, it will be created by the [`Snapshot`] for future
    /// testing and `true` will be returned.
    pub fn matches_hash(&self, path: impl AsRef<Path>) -> Result<bool, Error> {
        use sha2::{Digest, Sha256};

        let path = self.path(path, "sha256");

        let hash = {
            let mut hasher = Sha256::new();
            hasher.update(&self.screenshot.rgba);
            format!("{:x}", hasher.finalize())
        };

        if path.exists() {
            let saved_hash = fs::read_to_string(&path)?;

            Ok(hash == saved_hash)
        } else {
            if let Some(directory) = path.parent() {
                fs::create_dir_all(directory)?;
            }

            fs::write(path, hash)?;
            Ok(true)
        }
    }

    fn path(&self, path: impl AsRef<Path>, extension: &str) -> PathBuf {
        let path = path.as_ref();

        path.with_file_name(format!(
            "{name}-{renderer}",
            name = path
                .file_stem()
                .map(std::ffi::OsStr::to_string_lossy)
                .unwrap_or_default(),
            renderer = self.renderer
        ))
        .with_extension(extension)
    }
}

/// Creates a new [`Simulator`].
///
/// This is just a function version of [`Simulator::new`].
pub fn simulator<'a, Message, Theme, Renderer>(
    element: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Simulator<'a, Message, Theme, Renderer>
where
    Theme: theme::Base,
    Renderer: core::Renderer + core::renderer::Headless,
{
    Simulator::new(element)
}

/// Returns the sequence of events of a click.
pub fn click() -> impl Iterator<Item = Event> {
    [
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ]
    .into_iter()
}

/// Returns the sequence of events of a key press.
pub fn press_key(
    key: impl Into<keyboard::Key>,
    text: Option<SmolStr>,
) -> Event {
    let key = key.into();

    Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        repeat: false,
        text,
    })
}

/// Returns the sequence of events of a key release.
pub fn release_key(key: impl Into<keyboard::Key>) -> Event {
    let key = key.into();

    Event::Keyboard(keyboard::Event::KeyReleased {
        key: key.clone(),
        modified_key: key,
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
    })
}

/// Returns the sequence of events of a "key tap" (i.e. pressing and releasing a key).
pub fn tap_key(
    key: impl Into<keyboard::Key>,
    text: Option<SmolStr>,
) -> impl Iterator<Item = Event> {
    let key = key.into();

    [press_key(key.clone(), text), release_key(key)].into_iter()
}

/// Returns the sequence of events of typewriting the given text in a keyboard.
pub fn typewrite(text: &str) -> impl Iterator<Item = Event> + '_ {
    text.chars()
        .map(|c| SmolStr::new_inline(&c.to_string()))
        .flat_map(|c| tap_key(keyboard::Key::Character(c.clone()), Some(c)))
}

fn load_font(font: impl Into<Cow<'static, [u8]>>) -> Result<(), Error> {
    renderer::graphics::text::font_system()
        .write()
        .expect("Write to font system")
        .load_font(font.into());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::widget::Text;

    type Element<'a> =
        crate::core::Element<'a, (), core::Theme, renderer::Renderer>;

    fn hello<'a>() -> Element<'a> {
        Text::new("Hello").into()
    }

    #[test]
    fn a_snapshot_hands_out_its_pixels() -> Result<(), Error> {
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(200.0, 100.0),
            hello(),
        );

        let snapshot = ui.snapshot(&core::Theme::Light)?;
        assert_eq!(snapshot.scale_factor(), 2.0);
        assert!(!snapshot.renderer().is_empty());

        let (rgba, size) = snapshot.into_rgba();
        assert_eq!(size, Size::new(400, 200));
        assert_eq!(rgba.len(), 400 * 200 * 4);

        // White paper and something dark on it: the word
        assert_eq!(&rgba[..4], &[255, 255, 255, 255]);
        assert!(rgba.chunks(4).any(|pixel| pixel[0] < 64));

        Ok(())
    }

    #[test]
    fn a_snapshot_can_be_taken_at_another_scale() -> Result<(), Error> {
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(200.0, 100.0),
            hello(),
        );

        let (rgba, size) =
            ui.snapshot_with(&core::Theme::Light, 1.0)?.into_rgba();

        assert_eq!(size, Size::new(200, 100));
        assert_eq!(rgba.len(), 200 * 100 * 4);

        Ok(())
    }
}
