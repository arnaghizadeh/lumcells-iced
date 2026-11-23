## Focus ring

Every control takes part in keyboard focus: Tab and Shift+Tab move between the text input, the
buttons, the check box, the toggler, the radio buttons and the pick list; Enter and Space
activate the one that has the focus (the arrows choose in the pick list), and the built-in
theme draws a ring around it. A click elsewhere takes the focus away.

Since lane F2 also: a slider (the arrows, Page Up, Page Down, Home, End), a scrollable with
nothing focusable inside (it is a stop itself, and the same keys scroll it), a combo box (its
list is open while it has the focus; Tab leaves it), a text editor, and a field that cannot be
typed in and a disabled button, which Tab passes. "Clear…" opens a dialog in a `stack`:
`focus_trap` keeps Tab going round inside it and the keys away from the form below, Escape
answers it.

Nothing in `main.rs` wires the Tab key: the shell does it when no widget captured the key
(`iced::application(..).tab_focus(false)` turns that off).

```
cargo run --package focus_ring
cargo test --package focus_ring
```
