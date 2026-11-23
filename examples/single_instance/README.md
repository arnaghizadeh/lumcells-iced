## Single instance

A program that runs once per user. Start it, then start it again with some arguments: the
second process ends at once, and the window of the first lists the arguments it was handed
and comes to the front.

```
cargo run --package single_instance &
cargo run --package single_instance -- --open "a file.txt"
```

`tests/two_processes.rs` does the same with the built program (it needs a display: the CI
runs it under Xvfb; without one it is skipped).

On macOS the example also sets the activation policy: it starts as `Accessory` (no Dock icon)
and becomes `Regular` while its window is open. Built in CI; not run by a person yet.
