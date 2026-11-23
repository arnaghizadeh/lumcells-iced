## Terminal

`iced::widget::terminal` (the `terminal` feature): a grid of cells fed with the bytes a program
writes to a pseudo-terminal. The example starts your shell in one (`portable-pty`), feeds what
it writes into a `terminal::Content`, and writes what you type back to it.

- Click into the terminal (or Tab to it and press Enter) and type. Shift+Escape gives the keys
  back, so that Tab moves the focus on.
- The wheel, Shift+Page Up and Shift+Page Down scroll through what scrolled away.
- A drag selects, a double click a word, a triple click a line; Ctrl+Shift+C copies and
  Ctrl+Shift+V pastes (Cmd+C and Cmd+V on macOS). On X11 and Wayland the selection is also the
  primary selection and a middle click pastes it.
- The grid takes the size of the window (`Terminal::on_fit`), and the shell is told.

The widget starts nothing by itself: an application can feed it a recording or what a remote
machine sends just as well.

```
cargo run --package terminal
cargo test --package terminal
```
