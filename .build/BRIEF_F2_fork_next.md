# lumcells-iced, lane F2: what F1 left, and what the apps asked for since

F1 made the fork the company's toolkit (ten patches; .build/REPORT_F1.md, "What is left of the seven").
Since then the Drive desktop app (lanes S3, S4, S5) and the Workbench's window (lane K2: the terminal
widget on this branch) asked for more. Read first: .build/REPORT_F1.md, CHANGELOG.md (the Lumcells section),
README.md, ~/lumcells/drive/docs/WINDOWS_NOTES.md and MACOS_NOTES.md (the items marked "fork"),
~/lumcells/drive/.build/REPORT_S5.md section 1, ~/fasterlabeling/workbench/.build/REPORT_K2.md (the
terminal widget's limits), ~/fasterlabeling/workbench/crates/desktop and ~/lumcells/drive/crates/desktop (the two
apps on the fork: what they work around today). Work in /home/mehran/lumcells/iced on branch `lumcells`;
commit as "Lumcells Engineering <engineering@lumcells.com>"; push `origin lumcells`, never `upstream`. Each
patch: a test or an example, a CHANGELOG line, `cargo check --workspace --all-targets`, `cargo test
--workspace`, clippy clean on the crates touched, `cargo check` for `x86_64-pc-windows-msvc` and
`aarch64-apple-darwin`. Do NOT change the two apps (their lanes move the rev); say in the report which
app workaround each patch retires. Caps as the runner sets; another lane shares this machine.

## F2.1 Focus, finished
`slider`, `scrollable` (focus and arrow keys scroll), `combo_box`, `text_editor` take part in focus; a
focus trap for a modal layer (focus cycles inside an overlay while it is open); `focus_next` skips what is
disabled or hidden. Example `focus_ring` extended; tests of the order.
## F2.2 Accessibility, second step
Stable node ids (a widget's id survives a re-layout: readers keep their place); text with caret and
selection; scroll areas with their position; the open state of menus and pick lists; live regions for
status text (the Drive app's "Up to date" and the Workbench's run state); the names of F1/S5 shown in the
tree. Orca on this workstation reads the examples; write what it said.
## F2.3 Selectable text, finished
Shift+arrows, select all (Ctrl/Cmd+A inside a selectable text), `rich_text` and `markdown` selectable,
selection across several text widgets in one `column` (a selection container).
## F2.4 The terminal widget
From K2's limits: scrollback search, a selection that follows the pointer past the edge, bracketed paste,
mouse reporting pass-through when the program asks, 256 colours and true colour, URL detection with an
on-click message; tests of the parser and the grid; the example grows.
## F2.5 Tray and windows
Submenus and icons in tray menu items; a "no tray here" signal; `window::Settings` for a window that
remembers its place and size by a key (the apps write this themselves today); a `Task` that brings an
existing window to the front on every system (Wayland through xdg-activation with the token the single
instance passes).
## Before you end
`.build/REPORT_F2.md` as F1's, with the list of app workarounds each patch retires and the upstream pull
recipe unchanged or amended. Commit, push origin.
