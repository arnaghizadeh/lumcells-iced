## Three-state check box

A folder and the files in it. The check box of the folder is ticked when all of its files are,
empty when none is, and shows a dash (`checkbox::State::Indeterminate`) when some are. A click
on a box that shows the dash asks for `true`, like a click on an empty one.

```
cargo run --package tri_state_checkbox
cargo test --package tri_state_checkbox
```
