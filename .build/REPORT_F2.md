# Lane F2: report

23 Nov 2026. Branch `lumcells` of https://github.com/arnaghizadeh/lumcells-iced, from `3104cf75b`
to `8faeaf87e` (plus this report), pushed to `origin`. The two apps were not changed.

## In short

All five parts have a patch with tests and an example, and every gate of the brief passed on the
head here. What nobody has done: run any of it on a Mac, on Windows or on Wayland (there is no
compositor on this workstation), look at a real panel for the tray's submenus, or listen to a
screen reader (Orca's own log is what is written down). The fork's Actions did not run for any
push of this lane (the last run on `origin` is F1's `91e38fd18`), so there is no CI result for
the Windows and macOS unit tests: only `cargo check --target` from Linux. Seven lines under
"Changed" in the CHANGELOG say what breaks; the apps will meet four of them (below).

F2.4 and F2.5 were written by two helpers of mine in git worktrees while I did F2.1 to F2.3,
and cherry-picked; the gates below ran on the merged tree. I read their reports and ran their
tests, not every line of their code.

## The patches

| Part | Commit | What it does | Test / example |
|---|---|---|---|
| F2.1 Focus | `134a13847` | `Slider`, `VerticalSlider` take the focus (arrows, Page Up/Down, Home, End; `Status::Focused`, ring on the handle). A `Scrollable` is a stop when it overflows and holds nothing focusable; the same keys and Space scroll it; `tab_stop(false)`. `ComboBox` takes part (it had no `operate`), Tab leaves it. `TextEditor` without `on_action` is no stop. `focus_next`/`focus_previous` skip what has no size and what is disabled (`Focusable::is_tab_stop`). `focus_trap(content)` with `.on_escape(..)`: Tab goes round inside, keys do not reach what is below. Tab scrolls the newly focused widget into view. | 6 tests of the order in `iced_core`; `focus_ring` 18 tests (8 before); `.build/proof/focus_ring-f2-*.png` |
| F2.2 Accessibility | `8426859a8` | Stable `Node::id` (the widget's `Id`, else parent + role + name + which one), used as the AccessKit id; caret and selection of `TextInput`, `TextEditor`, the field of a `ComboBox`, a selectable `Text` (AccessKit text runs); `expanded` on `PickList` and `ComboBox`, the open list as `Menu`/`MenuItem` that can be pressed; `text(..).live(Live::Polite)`; slider and scroll area can be moved by assistive technology. | `screen_reader` (new, 8 tests); 3 new conversion tests in `iced_winit`; **Orca: `.build/proof/orca-f2.txt`** |
| found on the way | `8426859a8` | `keyed_column` handed children the state of other widgets when one was put in the middle: a panic "Downcast widget state" with widgets of two kinds. Upstream `0.14` has it too. | 4 tests, 3 of which fail on the old code |
| F2.3 Selectable text | `8faeaf87e` | Shift+arrows (by word with Ctrl/Option), Shift+Home/End/Up/Down by wrapped lines, Ctrl/Cmd+A, for the text clicked last; `rich_text(..).selectable(true)`; `markdown::Settings::selectable(true)`; `selection_area(content)`: one selection over several texts, copied with line feeds. `core::widget::text::Selection` is public. | `selectable_text` 15 tests (7 before); 2 in `iced_widget`; `.build/proof/selectable_text-f2-area.png` |
| F2.4 Terminal | `42999ddfc` | Search (`Content::search`, next, previous, `on_search`), a drag past the edge scrolls while held, mouse reporting (1000/1002/1003; SGR, UTF-8, legacy; Shift keeps the selection), links (`on_link`, Ctrl+click; OSC 8). Bracketed paste and 256/true colour were already there: tests added, nothing fixed. | 39 terminal unit tests (28 before), `terminal` example 22 (11 before) |
| F2.5 Tray and windows | `eff3cf5bd`, `06af659f8` | `tray::Item::submenu`, `Item::icon`; `tray::Event::Available` / `Unavailable`, `tray::is_available()`; `window::Settings::remember` (`window::Remember::new("main").directory(..)`): place, size and maximized state in a file per key; `window::present(id)`: un-minimize and focus, on Wayland `xdg_activation_v1.activate` with the token of the latest second start. | D-Bus test extended; 9 new unit tests in `iced_winit`; `tray` 5 tests; `single_instance` 2 window tests under Xvfb/openbox |

Each has its CHANGELOG line under "Lumcells"; the README names the new abilities and
`screen_reader`. `.github/scripts/orca-read.sh` is new (not run by CI).

## Gates, on `8faeaf87e`

| Gate | Result |
|---|---|
| `cargo check --workspace --all-targets` | finished |
| `cargo test --workspace` (under `xvfb-run`, `ICED_TEST_BACKEND=tiny-skia`, 4 threads) | 327 passed, 0 failed, 3 ignored (upstream's, as before) |
| `cargo lint` (clippy, workspace, all features, `-D warnings`) and `cargo fmt --check` | clean |
| `cargo check --target x86_64-pc-windows-msvc` and `aarch64-apple-darwin` for `iced_core`, `iced_runtime`, `iced_widget` (with `terminal`), `iced_winit` (with `tray`, `single-instance`, `accessibility`), `iced_test` | finished, both |
| `.github/scripts/fork-examples.sh` | 8 examples built, 82 tests passed, each alive after 6 s without a panic |

The same gates ran after F2.1, after F2.2 and after the merge of F2.4 and F2.5 (317 tests then).

## Orca

Orca 46.1 read `focus_ring`, `screen_reader` and `accessible_names`, each in a session of its
own (own Xvfb, D-Bus, AT-SPI bus, home; sound to a null device). **Nobody listened**: the file is
Orca's log of what it sent to the synthesizer, with the keys. It said the slider with its number
at each arrow, "panel" and the words of a focused scrollable, combo boxes and their open lists,
"Notes entry", characters and "selected" for Shift+Left, the status line when it changed
("Syncing 3 files") while the focus was on a button, the three buttons of the dialog and nothing
of the form under it, and the names of F1/S5 ("Sync Work check box partially checked",
"Password password text"). Not said: the first Tab from the window (as in F1; not solved), the
option chosen with Return and the list's new value, anything about how far a scrollable is
scrolled. The first reading of `screen_reader` ended in the `keyed_column` panic above; the file
holds the runs after the fix.

## What breaks for the apps (their lanes, when they move the rev)

1. Drive `tray.rs`: `match` over `tray::Event` needs arms for `Available` / `Unavailable`, and the
   one over `tray::Item` an arm for `Submenu`.
2. Both `theme.rs` give a scrollable one style for every status, so a focused scrollable draws no
   ring there until the style gives the container a border for `scrollable::Status::Focused`.
   Pages that overflow and hold no control become a new Tab stop.
3. Drive's `screens.rs` reads `Tree::describe()`: lines of fields now end in `caret=0`, pick
   lists in `closed`. Tests that compare whole lines need the new words.
4. `slider::Status::Focused` for a style that matches every variant (neither app has one today).

## App workarounds each patch retires

| Patch | Retires |
|---|---|
| `tray::Event::Unavailable` | Drive's lost "no tray icon" notice and "keeps running with no icon, start it again" (REPORT_F1, "No-tray desktops"): the app can keep its window, or say so. |
| `window::present` | `window::gain_focus` on a second start: Drive `app.rs:375` and `:386`, Workbench `app.rs:485`. The only way that can work on Wayland. |
| `focus_trap` | Drive draws confirmations ("Revoke it?", "Log out?") in place of the button because there is no dialog (ICED_FORK_NOTES, "Smaller things"): a `stack` with `opaque(focus_trap(..))` is one now. |
| Focusable scrollable, Tab reveals | Nothing to delete; Drive's Status page under the fold and the Workbench's log are reached by keyboard without the app doing anything. |
| `text(..).live(..)` | Nothing to delete; it is what makes "Up to date" (Drive) and the run state (Workbench) heard. One call each. |
| Stable ids, press by id | Drive's S3 scripts found a box "by the row of the text beside it"; ids from `.id(..)` are stable across layouts now. |
| Selectable `rich_text` / `markdown`, `selection_area` | Workbench `view.rs:34` makes each line its own selectable text, so a report is copied by a triple click per text: a `selection_area` around them copies it whole. ICED_FORK_NOTES item 7's open list (Shift+arrows, select all, rich text) is closed. |
| Terminal: mouse reporting | REPORT_K2 item 8, that limit. Search and the edge-following drag make item 6 (18 lines at once) bearable; the app has to add a search field and `on_search`. |
| `window::Remember` | **Nothing.** The brief says the apps write this themselves today; neither does (Drive opens centered at 980 x 680, the Workbench at 1200 x 780). It is a new ability: `Remember::new("main").directory(<profile dir>)`. |
| Tray submenus and icons | Nothing; Drive's menu is flat. Closes F1's left item 4. |
| `keyed_column` fix | Nothing; neither app uses it. |

## Not verified

- macOS and Windows: everything of this lane is type-checked only. Tray submenus and item icons
  (`muda`), `present`, the remembered window, the AccessKit text runs with VoiceOver or Narrator.
- Wayland: `xdg_activation_v1.activate` compiles and its rule has a test; it has never run. On
  Wayland a window remembers its size only.
- `window::present` on X11 is tested under openbox, where `gain_focus` passes the same test: it
  shows that it works, not that it is better. Raising from behind another window was not tested.
- A real panel for submenus, item icons and the no-tray signal (private `dbus-daemon` only). A
  watcher that outlives its panel cannot be told from one with a panel.
- The two window tests need `openbox`, `xdotool`, `xprop`, `wmctrl`; where one is missing they
  print that and pass.
- The terminal with a real full-screen program using the mouse (only `cat -v`); OSC 8 on screen;
  each wheel click of xdotool gave two reports, cause not found.
- AccessKit is told that the characters of a line share its width evenly: a reader that draws a
  box at the caret gets it at the wrong place inside the field.
- A person at any window. The pictures in `.build/proof` were looked at by me.
- The apps built against this head.

## What is left

- Focus: `mouse_area` and other clickable things that are not buttons (F1's left item 1, in part).
- Accessibility: character positions, setting the caret or text from assistive technology, the
  terminal in the tree, the first Tab under Orca.
- Selectable text: Shift+arrows from one text into the next, Shift+click, scrolling while dragging.
- Terminal: combining characters, re-wrap on resize, mode 1007, focus reports, a blinking cursor.
  The older terminal line in the CHANGELOG still lists mouse reporting as missing.
- CI: find out why pushes to `lumcells` start no run.

## Pulling from upstream later

F1's recipe, amended in its list of conflicts:

```
git fetch upstream
git checkout lumcells
git merge upstream/0.14          # or a release tag such as 0.15.0
# expect conflicts in: widget/src/{button,checkbox,toggler,radio,pick_list,text_input,slider,
# vertical_slider,scrollable,combo_box,text_editor,markdown}.rs, widget/src/text/rich.rs,
# widget/src/overlay/menu.rs, widget/src/keyed/column.rs (ours no longer calls
# tree::diff_children_custom_with_search: keep ours unless upstream fixed its search),
# core/src/widget/{text.rs,operation.rs}, core/src/widget/operation/focusable.rs,
# core/src/window/settings.rs, runtime/src/window.rs, winit/src/lib.rs, test/src/simulator.rs,
# Cargo.toml, .github/workflows/ (keep ours: git rm what upstream re-adds)
cargo check --workspace --all-targets && cargo test --workspace && cargo lint
.github/scripts/fork-examples.sh
git push origin lumcells         # never upstream
```

Upstream master has changed `keyed::Column` and the tree diff since 0.14 (229 lines); a merge
of 0.15 needs that file read, not resolved by side.

## The workstation

- Caps as set: `CARGO_BUILD_JOBS` 8 or fewer (3 per helper while three builds ran),
  `RUST_TEST_THREADS` 4.
- Everything graphical ran on Xvfb displays of its own (:90 to :110) with private D-Bus sessions;
  the live desktop (:20) and its pointer were not touched. No Orca, Xvfb or openbox of mine is
  left running.
- The helpers' worktrees and branches are removed. `/tmp/f2` and `/tmp/f24` hold logs and
  pictures and can go. `.claude/` in the checkout is ignored through `.git/info/exclude`.
- Nothing was installed.
