## Selectable text

Text that can be selected with the pointer and copied: `text(..).selectable(true)`.

Drag to select, double click for a word, triple click for everything; Ctrl+C (Cmd+C on macOS)
copies through the clipboard, and on X11 and Wayland the selection is also the primary
selection (middle click pastes it). A click elsewhere drops the selection.

Since lane F2: after a click on a text, Shift with the arrows, Home and End moves the end of the
selection and Ctrl+A selects all of it; `rich_text(..).selectable(true)` selects over spans; the
report in the middle is three texts in a `selection_area`, where a drag goes from one text to the
next and the copy key copies them as one; the Markdown under it is selectable
(`markdown::Settings::selectable(true)`), and its link is still clicked.

```
cargo run --package selectable_text
cargo test --package selectable_text
```
