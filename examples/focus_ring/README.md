## Focus ring

Every control takes part in keyboard focus: Tab and Shift+Tab move between the text input, the
buttons, the check box, the toggler, the radio buttons and the pick list; Enter and Space
activate the one that has the focus (the arrows choose in the pick list), and the built-in
theme draws a ring around it. A click elsewhere takes the focus away.

Nothing in `main.rs` wires the Tab key: the shell does it when no widget captured the key
(`iced::application(..).tab_focus(false)` turns that off).

```
cargo run --package focus_ring
cargo test --package focus_ring
```
