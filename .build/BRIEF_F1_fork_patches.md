# lumcells-iced, lane F1: the company's iced, first seven patches, CI, and the desktop app on it

You are the first builder of **lumcells-iced**, the Lumcells fork of the iced GUI library (MIT). The owner's
decision (22 Nov 2026): this fork is the toolkit Lumcells ships; we change it at our own pace and bring
upstream changes in later, only when needed. Read first: /home/mehran/lumcells/drive/docs/ICED_FORK_NOTES.md
(the seven gaps, each with "Today" = the workaround in the desktop app and "Patch" = what to build),
/home/mehran/lumcells/drive/docs/PLAN.md sections 11-14, /home/mehran/lumcells/drive/crates/desktop (the app
that will use this fork), and this repository's CONTRIBUTING.md, core/, widget/, winit/, runtime/.

You work in **/home/mehran/lumcells/iced** on branch **`lumcells`** (branched from the tag 0.14.0, the release
the desktop app uses today). Commit after each patch as "Lumcells Engineering <engineering@lumcells.com>"
(never any personal name), push with `git push origin lumcells` after each milestone (remote `origin` =
github.com/arnaghizadeh/lumcells-iced; never push to `upstream`). Keep `cargo check --workspace` and the
existing tests green on every commit; clippy clean on the crates you touch. Times in CT. Never print secrets.
One patch = one commit (or a few), each with a test or an example that shows it, and a line in CHANGELOG.md
under "Lumcells". Keep the public API of 0.14.0 working: the app must build before and after each commit.

## Milestones, in order
### F1.1 CI
- `.github/workflows/ci.yml`: `cargo check --workspace` and the unit tests on ubuntu, windows and macos
  runners (each under 30 min, cargo cached), clippy on Linux, the examples that the patches add built on
  Linux under Xvfb. Green before any patch is merged.
### F1.2 Keyboard focus on every control (notes item 1)
- `Button`, `Checkbox`, `Toggler`, `Radio`, `PickList` take part in focus: `operation::Focusable`, a focused
  state, `Status::Focused` so a style can draw the ring (the default theme draws one), activation on Enter and
  Space, unfocus on a click elsewhere; Tab / Shift+Tab move focus by default in `iced_winit` when no widget
  captured the key. Example `examples/focus_ring` and a test of the focus order.
### F1.3 Three-state check box (item 3) and selectable text (item 7)
- `checkbox::State { Unchecked, Checked, Indeterminate }` with a dash icon and the status carrying the
  state; `bool` still converts. `Text` with `.selectable(true)`: pointer selection, Ctrl/Cmd+C copies through
  the clipboard, Shift+arrows later; example `examples/selectable_text`.
### F1.4 Tray without GTK (item 4)
- A `tray` module in `iced_winit`: `tray::Settings { icon, tooltip, menu }` on the daemon, `tray::update(..)`
  as a `Task`, `tray::events()` as a `Subscription` giving `MenuItem(id)` and `Click`. Linux: the
  StatusNotifierItem and `com.canonical.dbusmenu` interfaces directly over D-Bus with `zbus`, no GTK, no
  extra thread, no polling. macOS and Windows: wrap `tray-icon` inside the event loop. Example
  `examples/tray` (a counter in the tooltip, a menu with Open, Pause, Quit). Prove it on this workstation's
  desktop (a screenshot of the tray icon and its menu).
### F1.5 macOS activation policy (item 5) and single instance (item 6)
- `window::Settings`/a `Task` to set `Regular | Accessory` on macOS (winit's `set_activation_policy` at
  start, `NSApp.setActivationPolicy` later). `daemon(..).single_instance("id")`: a second process hands its
  arguments to the first over a per-user socket (named pipe on Windows) and exits; the first receives them as
  a `Subscription<Vec<String>>`; on Wayland pass an xdg-activation token to raise the window. Tests where a
  display is not needed; the macOS part builds in CI and is marked "not run by a person".
### F1.6 Accessibility tree, the start (item 2)
- An `accesskit` adapter in `iced_winit` with roles and names for button, text, text input, check box,
  toggler, progress bar and window, built from the widget tree each frame; read by Orca on this workstation
  for the `focus_ring` example (write what Orca said). Track upstream's work in the notes; do not wait for it.
### F1.7 The desktop app on the fork
- In /home/mehran/lumcells/drive: `[patch.crates-io]` (or git dependencies) pointing `iced` and its crates at
  this fork's `lumcells` branch by commit; remove the workarounds the notes name (own Tab handling, own
  tri-box, the GTK tray thread and `bus.rs` polling, the "already running" window); the app's tests and the
  S2 proof script pass; GTK leaves the app's dependencies and packages. Commit and push the Drive repo too
  (`origin` and `backup`). If a patch is not ready, the app keeps that workaround and the notes say so.

## Before you end
Write `.build/REPORT_F1.md` here (what each patch does with its test or example, the CI run URL, what was not
verified, what the Drive app dropped, what is left of the seven, and how to pull from upstream later: the
rebase/merge recipe). Commit, push origin. Leave the workstation's desktop as you found it.

## Added 22 Nov 2026 23:40 CT, after lane S2's report
- **GitHub Actions does not run** for this account right now (billing; the owner settles it). Write
  `ci.yml` anyway (F1.1) and push it, but do not wait for a run: verify every milestone on this
  workstation (`cargo test`, clippy, the examples under Xvfb, the Drive app's tests and
  `scripts/s2-proof.sh`). The report says which runs never happened.
- The notes now have ten entries. After the seven, if small: item 8 (folder dialog: the answer is
  `rfd`, no patch, only say so), item 9 (a subscription that owns a channel: a `Subscription::run_with`
  that hands the channel's receiver to the stream), item 10 (the test library keeps its pixels: a
  way to drop them).
- Use at most 6 build jobs and 4 test threads (set by the lane runner). The box has restarted under
  heavy load before; commit often.
