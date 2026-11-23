## Tray

A program that lives in the tray: `iced::daemon` with no window at start, an icon in the tray
whose tooltip counts the seconds, and a menu with Open, Pause and Quit.

- A click on the icon, or "Open", opens the window (or brings it to the front).
- "Pause" stops the count; the item gets a tick and the icon turns grey.
- "Quit" ends the program. Closing the window does not.

On Linux the icon is a StatusNotifierItem with a `com.canonical.dbusmenu` menu, served on the
session bus without GTK; the desktop needs something that shows such items (KDE, Xfce, Cinnamon,
MATE, GNOME with the AppIndicator extension). On macOS and Windows it is made with `tray-icon`
inside the event loop.

```
cargo run --package tray
```
