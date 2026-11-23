//! The mouse as the bytes a terminal sends for it, when the program asked
//! to be told (the sequences of xterm).
use crate::core::keyboard::Modifiers;

pub use super::grid::{MouseEncoding, MouseMode};

/// A button of the mouse, or the wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// The left button.
    Left,
    /// The middle button.
    Middle,
    /// The right button.
    Right,
    /// The wheel, a step up.
    WheelUp,
    /// The wheel, a step down.
    WheelDown,
    /// The wheel, a step to the left.
    WheelLeft,
    /// The wheel, a step to the right.
    WheelRight,
}

/// What the mouse did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A button went down, or the wheel made a step.
    Press(Button),
    /// A button came up.
    Release(Button),
    /// The pointer came to another cell, with a button held or none.
    Move(Option<Button>),
}

impl Action {
    /// Whether a program that asked for `mode` is told of it.
    pub fn is_reported(self, mode: MouseMode) -> bool {
        match (mode, self) {
            (MouseMode::None, _) => false,
            (_, Action::Press(_) | Action::Release(_)) => true,
            (MouseMode::Clicks, Action::Move(_)) => false,
            (MouseMode::Drags, Action::Move(held)) => held.is_some(),
            (MouseMode::Motion, Action::Move(_)) => true,
        }
    }
}

/// The bytes for what the mouse did in the cell at `column` and `line` of
/// the screen (both from 0), or `None` when it cannot be said: the wheel
/// does not come up, and the [`Legacy`](MouseEncoding::Legacy) encoding
/// has no way to name a cell beyond 223 (the
/// [`Utf8`](MouseEncoding::Utf8) one beyond 2015).
pub fn encode(
    action: Action,
    column: usize,
    line: usize,
    modifiers: Modifiers,
    encoding: MouseEncoding,
) -> Option<Vec<u8>> {
    let number = |button: Button| match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
        Button::WheelUp => 64,
        Button::WheelDown => 65,
        Button::WheelLeft => 66,
        Button::WheelRight => 67,
    };
    let is_wheel = |button: Button| number(button) >= 64;

    let (button, released) = match action {
        Action::Press(button) => (number(button), false),
        Action::Release(button) if is_wheel(button) => return None,
        Action::Release(button) => (number(button), true),
        Action::Move(Some(button)) if is_wheel(button) => return None,
        Action::Move(Some(button)) => (number(button) + 32, false),
        // 3 is "no button"
        Action::Move(None) => (3 + 32, false),
    };

    // Shift (4), Alt (8), Control (16), as xterm counts
    let code = button
        + 4 * u32::from(modifiers.shift())
        + 8 * u32::from(modifiers.alt())
        + 16 * u32::from(modifiers.control());

    match encoding {
        MouseEncoding::Sgr => Some(
            format!(
                "\x1b[<{code};{};{}{}",
                column + 1,
                line + 1,
                if released { 'm' } else { 'M' }
            )
            .into_bytes(),
        ),
        MouseEncoding::Legacy | MouseEncoding::Utf8 => {
            // A release does not say which button
            let code = if released { (code & !3) | 3 } else { code };
            let mut bytes = b"\x1b[M".to_vec();

            bytes.push(u8::try_from(32 + code).ok()?);

            for place in [column, line] {
                let place = u32::try_from(place + 1 + 32).ok()?;

                if encoding == MouseEncoding::Legacy {
                    bytes.push(u8::try_from(place).ok()?);
                } else {
                    let mut buffer = [0; 4];
                    let character =
                        char::from_u32(place).filter(|_| place < 2048)?;

                    bytes.extend_from_slice(
                        character.encode_utf8(&mut buffer).as_bytes(),
                    );
                }
            }

            Some(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_told_what_it_asked_for() {
        let press = Action::Press(Button::Left);
        let drag = Action::Move(Some(Button::Left));
        let hover = Action::Move(None);

        assert!(!press.is_reported(MouseMode::None));
        assert!(press.is_reported(MouseMode::Clicks));
        assert!(Action::Release(Button::Left).is_reported(MouseMode::Clicks));
        assert!(Action::Press(Button::WheelUp).is_reported(MouseMode::Clicks));
        assert!(!drag.is_reported(MouseMode::Clicks));
        assert!(drag.is_reported(MouseMode::Drags));
        assert!(!hover.is_reported(MouseMode::Drags));
        assert!(hover.is_reported(MouseMode::Motion));
    }

    #[test]
    fn the_sgr_encoding_says_everything_in_digits() {
        let none = Modifiers::empty();
        let sgr = |action, column, line, modifiers| {
            String::from_utf8(
                encode(action, column, line, modifiers, MouseEncoding::Sgr)
                    .unwrap(),
            )
            .unwrap()
        };

        assert_eq!(
            sgr(Action::Press(Button::Left), 0, 0, none),
            "\x1b[<0;1;1M"
        );
        assert_eq!(
            sgr(Action::Release(Button::Left), 9, 4, none),
            "\x1b[<0;10;5m",
            "a release names its button and ends in a small m"
        );
        assert_eq!(
            sgr(Action::Press(Button::Right), 299, 99, none),
            "\x1b[<2;300;100M",
            "no cell is too far"
        );
        assert_eq!(
            sgr(Action::Press(Button::Middle), 1, 1, Modifiers::CTRL),
            "\x1b[<17;2;2M"
        );
        assert_eq!(
            sgr(Action::Move(Some(Button::Left)), 5, 6, none),
            "\x1b[<32;6;7M"
        );
        assert_eq!(sgr(Action::Move(None), 5, 6, none), "\x1b[<35;6;7M");
        assert_eq!(
            sgr(Action::Press(Button::WheelUp), 2, 3, none),
            "\x1b[<64;3;4M"
        );
        assert_eq!(
            sgr(Action::Press(Button::WheelDown), 2, 3, Modifiers::ALT),
            "\x1b[<73;3;4M"
        );
    }

    #[test]
    fn the_legacy_encoding_is_three_bytes_and_ends_at_cell_223() {
        let none = Modifiers::empty();
        let legacy = |action, column, line| {
            encode(action, column, line, none, MouseEncoding::Legacy)
        };

        assert_eq!(
            legacy(Action::Press(Button::Left), 0, 0).unwrap(),
            b"\x1b[M\x20\x21\x21"
        );
        assert_eq!(
            legacy(Action::Release(Button::Right), 9, 4).unwrap(),
            b"\x1b[M\x23\x2a\x25",
            "a release does not say which button"
        );
        assert_eq!(
            legacy(Action::Press(Button::WheelDown), 1, 1).unwrap(),
            b"\x1b[M\x61\x22\x22"
        );
        assert_eq!(
            legacy(Action::Press(Button::Left), 222, 0).unwrap(),
            [0x1b, b'[', b'M', 0x20, 0xff, 0x21]
        );
        assert_eq!(legacy(Action::Press(Button::Left), 223, 0), None);
        assert_eq!(legacy(Action::Press(Button::Left), 0, 500), None);

        // The same cell as UTF-8
        assert_eq!(
            encode(
                Action::Press(Button::Left),
                223,
                0,
                none,
                MouseEncoding::Utf8
            )
            .unwrap(),
            "\x1b[M\x20\u{100}\x21".as_bytes()
        );
        assert_eq!(
            encode(
                Action::Press(Button::Left),
                2100,
                0,
                none,
                MouseEncoding::Utf8
            ),
            None
        );
    }

    #[test]
    fn the_wheel_has_no_release() {
        for encoding in [MouseEncoding::Sgr, MouseEncoding::Legacy] {
            assert_eq!(
                encode(
                    Action::Release(Button::WheelUp),
                    0,
                    0,
                    Modifiers::empty(),
                    encoding
                ),
                None
            );
        }
    }
}
