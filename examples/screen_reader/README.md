## Screen reader

What a screen reader is told beyond names (lane F2.2): a status line that is said when it changes
(`text(..).live(Live::Polite)`), the caret and the selection in a field and an editor, whether a
pick list is open and what is in it, a slider with its number, a list with how far it is
scrolled, and identifiers that stay with a widget while a notice comes and goes above it.

The tests read the tree assistive technology is given (`Simulator::accessibility`) and act
through it. `.github/scripts/orca-read.sh` lets Orca read it; what it said is in
`.build/proof/orca-f2.txt`.

```
cargo run --package screen_reader
cargo test --package screen_reader
```
