//! Keys as the bytes a terminal sends for them (the sequences of xterm).
use crate::core::keyboard::key::Named;
use crate::core::keyboard::{Key, Modifiers};

/// The bytes for a key press, or `None` when the key sends nothing (a
/// modifier alone, a key a terminal does not have).
///
/// `text` is what the key types with the modifiers that are held, as the
/// window system says it. `application_cursor` is the mode a full screen
/// program asks for (`ESC [ ? 1 h`): the arrows are then sent as `ESC O A`.
pub fn encode(
    key: &Key,
    modifiers: Modifiers,
    text: Option<&str>,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    let alt = modifiers.alt();
    let control = modifiers.control();
    let shift = modifiers.shift();

    // 1 + shift (1) + alt (2) + control (4), as xterm counts
    let modifier =
        1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);

    let escaped = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);

        if alt {
            out.push(0x1b);
        }

        out.extend_from_slice(bytes);
        Some(out)
    };

    // `ESC [ A` and the like, `ESC [ 1 ; m A` with modifiers
    let letter = |last: char| {
        Some(if modifier > 1 {
            format!("\x1b[1;{modifier}{last}").into_bytes()
        } else if application_cursor {
            format!("\x1bO{last}").into_bytes()
        } else {
            format!("\x1b[{last}").into_bytes()
        })
    };

    // `ESC [ 3 ~` and the like
    let tilde = |number: u8| {
        Some(if modifier > 1 {
            format!("\x1b[{number};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        })
    };

    // F1 to F4
    let ss3 = |last: char| {
        Some(if modifier > 1 {
            format!("\x1b[1;{modifier}{last}").into_bytes()
        } else {
            format!("\x1bO{last}").into_bytes()
        })
    };

    match key {
        Key::Named(named) => match named {
            Named::Enter => escaped(b"\r"),
            Named::Backspace => {
                escaped(if control { b"\x08" } else { b"\x7f" })
            }
            Named::Tab if shift => Some(b"\x1b[Z".to_vec()),
            Named::Tab => escaped(b"\t"),
            Named::Escape => escaped(b"\x1b"),
            Named::Space if control => escaped(b"\0"),
            Named::Space => escaped(b" "),
            Named::ArrowUp => letter('A'),
            Named::ArrowDown => letter('B'),
            Named::ArrowRight => letter('C'),
            Named::ArrowLeft => letter('D'),
            Named::Home => letter('H'),
            Named::End => letter('F'),
            Named::Insert => tilde(2),
            Named::Delete => tilde(3),
            Named::PageUp => tilde(5),
            Named::PageDown => tilde(6),
            Named::F1 => ss3('P'),
            Named::F2 => ss3('Q'),
            Named::F3 => ss3('R'),
            Named::F4 => ss3('S'),
            Named::F5 => tilde(15),
            Named::F6 => tilde(17),
            Named::F7 => tilde(18),
            Named::F8 => tilde(19),
            Named::F9 => tilde(20),
            Named::F10 => tilde(21),
            Named::F11 => tilde(23),
            Named::F12 => tilde(24),
            _ => None,
        },
        Key::Character(character) => {
            if control {
                let mut letters = character.chars();
                let (first, rest) = (letters.next()?, letters.next());

                if rest.is_none() {
                    let code = match first.to_ascii_lowercase() {
                        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
                        '@' | '2' | ' ' => Some(0),
                        '[' | '3' => Some(0x1b),
                        '\\' | '4' => Some(0x1c),
                        ']' | '5' => Some(0x1d),
                        '^' | '6' => Some(0x1e),
                        '_' | '7' | '/' => Some(0x1f),
                        '?' | '8' => Some(0x7f),
                        _ => None,
                    };

                    if let Some(code) = code {
                        return escaped(&[code]);
                    }
                }

                return None;
            }

            if modifiers.logo() {
                return None;
            }

            match text {
                Some(text) if !text.is_empty() => escaped(text.as_bytes()),
                // With Alt held some systems give no text
                _ => escaped(character.as_bytes()),
            }
        }
        Key::Unidentified => match text {
            Some(text) if !text.is_empty() && !control => {
                escaped(text.as_bytes())
            }
            _ => None,
        },
    }
}

/// Pasted text as a terminal sends it: line ends as the Enter key, and
/// between the paste marks when the program asked for them (`bracketed`),
/// without anything in it that could end the paste early.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");

    if bracketed {
        let text = text.replace('\x1b', "");

        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(key: Named, modifiers: Modifiers, app: bool) -> Option<Vec<u8>> {
        encode(&Key::Named(key), modifiers, None, app)
    }

    fn typed(character: &str, modifiers: Modifiers) -> Option<Vec<u8>> {
        encode(
            &Key::Character(character.into()),
            modifiers,
            Some(character),
            false,
        )
    }

    #[test]
    fn letters_are_what_they_type() {
        assert_eq!(typed("a", Modifiers::empty()).unwrap(), b"a");
        assert_eq!(typed("A", Modifiers::SHIFT).unwrap(), b"A");
        assert_eq!(typed("é", Modifiers::empty()).unwrap(), "é".as_bytes());
        assert_eq!(typed("x", Modifiers::ALT).unwrap(), b"\x1bx");
        assert_eq!(typed("v", Modifiers::LOGO), None);
    }

    #[test]
    fn control_makes_control_codes() {
        assert_eq!(typed("c", Modifiers::CTRL).unwrap(), [3]);
        assert_eq!(typed("D", Modifiers::CTRL).unwrap(), [4]);
        assert_eq!(typed("z", Modifiers::CTRL).unwrap(), [26]);
        assert_eq!(typed("[", Modifiers::CTRL).unwrap(), [0x1b]);
        assert_eq!(typed("\\", Modifiers::CTRL).unwrap(), [0x1c]);
        assert_eq!(
            typed("c", Modifiers::CTRL | Modifiers::ALT).unwrap(),
            [0x1b, 3]
        );
        assert_eq!(typed("=", Modifiers::CTRL), None);
        assert_eq!(named(Named::Space, Modifiers::CTRL, false).unwrap(), [0]);
    }

    #[test]
    fn the_keys_of_the_line() {
        let none = Modifiers::empty();

        assert_eq!(named(Named::Enter, none, false).unwrap(), b"\r");
        assert_eq!(named(Named::Backspace, none, false).unwrap(), [0x7f]);
        assert_eq!(named(Named::Tab, none, false).unwrap(), b"\t");
        assert_eq!(
            named(Named::Tab, Modifiers::SHIFT, false).unwrap(),
            b"\x1b[Z"
        );
        assert_eq!(named(Named::Escape, none, false).unwrap(), [0x1b]);
        assert_eq!(named(Named::Shift, none, false), None);
    }

    #[test]
    fn arrows_in_both_modes_and_with_modifiers() {
        let none = Modifiers::empty();

        assert_eq!(named(Named::ArrowUp, none, false).unwrap(), b"\x1b[A");
        assert_eq!(named(Named::ArrowUp, none, true).unwrap(), b"\x1bOA");
        assert_eq!(named(Named::ArrowLeft, none, false).unwrap(), b"\x1b[D");
        assert_eq!(
            named(Named::ArrowRight, Modifiers::CTRL, true).unwrap(),
            b"\x1b[1;5C"
        );
        assert_eq!(
            named(Named::ArrowDown, Modifiers::SHIFT | Modifiers::ALT, false)
                .unwrap(),
            b"\x1b[1;4B"
        );
        assert_eq!(named(Named::Home, none, false).unwrap(), b"\x1b[H");
        assert_eq!(named(Named::End, none, true).unwrap(), b"\x1bOF");
    }

    #[test]
    fn the_other_keys() {
        let none = Modifiers::empty();

        assert_eq!(named(Named::Delete, none, false).unwrap(), b"\x1b[3~");
        assert_eq!(named(Named::PageUp, none, false).unwrap(), b"\x1b[5~");
        assert_eq!(
            named(Named::PageDown, Modifiers::CTRL, false).unwrap(),
            b"\x1b[6;5~"
        );
        assert_eq!(named(Named::F1, none, false).unwrap(), b"\x1bOP");
        assert_eq!(named(Named::F5, none, false).unwrap(), b"\x1b[15~");
        assert_eq!(named(Named::F12, none, false).unwrap(), b"\x1b[24~");
        assert_eq!(
            named(Named::F2, Modifiers::SHIFT, false).unwrap(),
            b"\x1b[1;2Q"
        );
    }

    #[test]
    fn pasted_lines_end_as_the_enter_key_does() {
        assert_eq!(paste("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(paste("ls\n", true), b"\x1b[200~ls\r\x1b[201~");
        // A paste cannot end itself
        assert_eq!(
            paste("x\x1b[201~rm -rf", true),
            b"\x1b[200~x[201~rm -rf\x1b[201~"
        );
    }
}
