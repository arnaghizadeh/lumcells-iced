## Selectable text

Text that can be selected with the pointer and copied: `text(..).selectable(true)`.

Drag to select, double click for a word, triple click for everything; Ctrl+C (Cmd+C on macOS)
copies through the clipboard, and on X11 and Wayland the selection is also the primary
selection (middle click pastes it). A click elsewhere drops the selection.

```
cargo run --package selectable_text
cargo test --package selectable_text
```
