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
- A drag past the top or the bottom scrolls for as long as the button is held there.
- Ctrl+Shift+F (Cmd+F) shows a search field (`Terminal::on_search`, `Content::search`): every
  place is marked, Enter and "Previous" go to the place before, "Next" to the one after.
- An address (`https://...`) or a link a program made (`OSC 8`) is underlined under the pointer;
  Ctrl+click (Cmd+click) hands it to the example (`Terminal::on_link`), which only shows it in
  the line below: opening it is the application's.
- A program that asks for the mouse (`vim` with `:set mouse=a`, `htop`, `tmux`) gets the clicks,
  drags and the wheel; hold Shift to select text as before.

The widget starts nothing by itself: an application can feed it a recording or what a remote
machine sends just as well.

```
cargo run --package terminal
cargo test --package terminal
```
