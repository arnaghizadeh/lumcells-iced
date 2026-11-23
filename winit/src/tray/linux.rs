//! The tray icon on Linux: a StatusNotifierItem with a menu, served on the
//! session bus.
//!
//! Two objects are exported on a connection of their own:
//!
//! - `/StatusNotifierItem` with the `org.kde.StatusNotifierItem` interface
//!   (the icon, the tooltip, the activation), registered with the
//!   `org.kde.StatusNotifierWatcher` of the desktop, and again whenever a
//!   watcher appears (the panel was restarted, the extension enabled);
//! - `/MenuBar` with the `com.canonical.dbusmenu` interface (the menu the
//!   desktop draws itself).
//!
//! Everything runs as one future on the executor of the program. Nothing
//! is polled: the future wakes when the bus or the program has something.
use crate::futures::futures::channel::mpsc;
use crate::futures::futures::future;
use crate::futures::futures::stream::{self, BoxStream};
use crate::futures::futures::{FutureExt, StreamExt};
use crate::runtime::tray::{Event, Icon, Item as MenuItem, Menu, Settings};

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, Structure, Value};

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// The handle of the tray icon; dropping it takes the icon away.
pub struct Tray {
    commands: mpsc::UnboundedSender<Settings>,
}

impl Tray {
    /// Starts serving the tray icon described by the [`Settings`].
    ///
    /// `spawn` runs the given stream on an executor until it ends.
    pub fn new(
        settings: Settings,
        spawn: impl FnOnce(BoxStream<'static, ()>),
    ) -> Self {
        let (commands, receiver) = mpsc::unbounded();

        spawn(
            stream::once(async move {
                if let Err(error) = serve(settings, receiver, None).await {
                    log::error!("the tray icon stopped: {error}");

                    // No session bus, or it went away: nothing shows the
                    // icon
                    super::emit(Event::Unavailable);
                }
            })
            .filter_map(|()| future::ready(None))
            .boxed(),
        );

        Self { commands }
    }

    /// Whether the icon is still being served (it is not after an error of
    /// the bus).
    pub fn is_alive(&self) -> bool {
        !self.commands.is_closed()
    }

    /// Changes the tray icon into the one described by the [`Settings`].
    pub fn update(&mut self, settings: Settings) {
        let _ = self.commands.unbounded_send(settings);
    }
}

/// What both objects show; changed by [`Tray::update`].
struct Shared {
    settings: Settings,
    /// The revision of the layout of the menu, as `com.canonical.dbusmenu`
    /// counts it.
    revision: u32,
}

type State = Arc<Mutex<Shared>>;

fn lock(state: &State) -> MutexGuard<'_, Shared> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What wakes the serving future.
enum Wake {
    Update(Settings),
    WatcherAppeared,
    WatcherVanished,
    HostsChanged,
    Closed,
}

/// Serves the tray icon until the sender of `updates` is dropped.
///
/// `address` is the bus to serve on; `None` is the session bus.
async fn serve(
    settings: Settings,
    updates: mpsc::UnboundedReceiver<Settings>,
    address: Option<&str>,
) -> zbus::Result<()> {
    static COUNTER: AtomicU32 = AtomicU32::new(1);

    let state = Arc::new(Mutex::new(Shared {
        settings,
        revision: 1,
    }));

    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address)?,
        None => zbus::connection::Builder::session()?,
    };

    // The connection gets no thread of its own: its tasks are run below
    let connection = builder
        .internal_executor(false)
        .serve_at(
            ITEM_PATH,
            StatusNotifierItem {
                state: state.clone(),
            },
        )?
        .serve_at(
            MENU_PATH,
            DbusMenu {
                state: state.clone(),
            },
        )?
        .build()
        .await?;

    let run = {
        let connection = connection.clone();

        async move {
            loop {
                connection.executor().tick().await;
            }
        }
    };

    let main = async {
        // The name the specification asks for; the watcher is given it
        let name = format!(
            "org.kde.StatusNotifierItem-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );

        connection.request_name(name.as_str()).await?;

        // A watcher that appears later (or again) has to be told too
        let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
        let appearances = dbus
            .receive_name_owner_changed_with_args(&[(0, WATCHER)])
            .await?
            .filter_map(|signal| {
                future::ready(signal.args().ok().map(|args| {
                    if args.new_owner().is_some() {
                        Wake::WatcherAppeared
                    } else {
                        Wake::WatcherVanished
                    }
                }))
            });

        // A watcher can be there before anything draws the icons (at a
        // login the panel may come after the program): it says when a
        // host arrives or leaves
        let hosts = zbus::MessageStream::for_match_rule(
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface(WATCHER)?
                .path(WATCHER_PATH)?
                .build(),
            &connection,
            None,
        )
        .await?
        .filter_map(|message| {
            future::ready(message.ok().and_then(|message| {
                let header = message.header();
                let member = header.member()?.as_str();

                member
                    .starts_with("StatusNotifierHost")
                    .then_some(Wake::HostsChanged)
            }))
        });

        let registered = register(&connection, &name).await;
        tell(registered && is_hosted(&connection).await);

        let mut wakes = stream::select(
            updates
                .map(Wake::Update)
                .chain(stream::once(future::ready(Wake::Closed))),
            stream::select(appearances, hosts),
        );

        while let Some(wake) = wakes.next().await {
            match wake {
                Wake::Update(settings) => {
                    apply(&connection, &state, settings).await?;
                }
                Wake::WatcherAppeared => {
                    let registered = register(&connection, &name).await;
                    tell(registered && is_hosted(&connection).await);
                }
                Wake::WatcherVanished => tell(false),
                Wake::HostsChanged => {
                    tell(is_hosted(&connection).await);
                }
                Wake::Closed => break,
            }
        }

        let _ = connection.release_name(name.as_str()).await;

        Ok(())
    };

    futures_select(run, main).await
}

/// Runs both futures until `main` ends (`run` never does).
async fn futures_select<T>(
    run: impl Future<Output = ()>,
    main: impl Future<Output = T>,
) -> T {
    let run = std::pin::pin!(run.fuse());
    let main = std::pin::pin!(main);

    match future::select(main, run).await {
        future::Either::Left((output, _)) => output,
        future::Either::Right(((), main)) => main.await,
    }
}

/// Tells the program whether this desktop shows the icon.
fn tell(is_available: bool) {
    super::emit(if is_available {
        Event::Available
    } else {
        Event::Unavailable
    });
}

/// Whether something draws the icons the watcher knows: its
/// `IsStatusNotifierHostRegistered`. A watcher that is there and does not
/// answer this is believed to have one.
async fn is_hosted(connection: &zbus::Connection) -> bool {
    let reply = connection
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(WATCHER, "IsStatusNotifierHostRegistered"),
        )
        .await;

    match reply {
        Ok(reply) => reply
            .body()
            .deserialize::<zbus::zvariant::OwnedValue>()
            .ok()
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(true),
        Err(zbus::Error::MethodError(name, _, _)) => !matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        Err(_) => false,
    }
}

/// Tells the watcher of the desktop about the item, and returns whether
/// it took it. With no watcher (no tray on this desktop, yet) there is
/// nothing to tell.
async fn register(connection: &zbus::Connection, name: &str) -> bool {
    let result = connection
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(WATCHER),
            "RegisterStatusNotifierItem",
            &(name,),
        )
        .await;

    if let Err(error) = &result {
        log::info!(
            "no tray on this desktop for now ({WATCHER} did not take the \
            icon: {error})"
        );
    }

    result.is_ok()
}

/// Changes what is shown into the new [`Settings`] and tells the desktop
/// what changed.
async fn apply(
    connection: &zbus::Connection,
    state: &State,
    settings: Settings,
) -> zbus::Result<()> {
    let (icon, tooltip, title, menu) = {
        let mut shared = lock(state);

        let icon = shared.settings.icon != settings.icon;
        let tooltip = shared.settings.tooltip != settings.tooltip;
        let title = shared.settings.title() != settings.title();
        let menu = MenuChange::between(&shared.settings.menu, &settings.menu);

        if let MenuChange::Layout = menu {
            shared.revision += 1;
        }

        let revision = shared.revision;
        shared.settings = settings;

        (icon, tooltip, title, (menu, revision))
    };

    let server = connection.object_server();

    if icon || tooltip || title {
        let item = server.interface::<_, StatusNotifierItem>(ITEM_PATH).await?;
        let emitter = item.signal_emitter();

        if icon {
            StatusNotifierItem::new_icon(emitter).await?;
        }

        if tooltip {
            StatusNotifierItem::new_tool_tip(emitter).await?;
        }

        if title {
            StatusNotifierItem::new_title(emitter).await?;
        }
    }

    let (menu, revision) = menu;

    match menu {
        MenuChange::None => {}
        MenuChange::Properties { updated, removed } => {
            let menu = server.interface::<_, DbusMenu>(MENU_PATH).await?;

            DbusMenu::items_properties_updated(
                menu.signal_emitter(),
                updated,
                removed,
            )
            .await?;
        }
        MenuChange::Layout => {
            let menu = server.interface::<_, DbusMenu>(MENU_PATH).await?;

            DbusMenu::layout_updated(menu.signal_emitter(), revision, ROOT)
                .await?;
        }
    }

    Ok(())
}

/// How a menu differs from the one before it.
enum MenuChange {
    /// It does not.
    None,
    /// The same items in the same places, with other labels, ticks or
    /// greyed states: a menu that is open stays open and changes in place.
    Properties {
        updated: Vec<(i32, Properties)>,
        removed: Vec<(i32, Vec<String>)>,
    },
    /// Items came, went or changed their kind: the desktop reads the menu
    /// again.
    Layout,
}

impl MenuChange {
    fn between(old: &Menu, new: &Menu) -> Self {
        if old == new {
            return MenuChange::None;
        }

        let (_, old) = entries(old);
        let (_, new) = entries(new);

        // The same kinds of items with as many items below each: the
        // same tree, since both are numbered from the top, depth first
        let same_shape = old.len() == new.len()
            && old.iter().zip(&new).all(|(old, new)| {
                std::mem::discriminant(old.item)
                    == std::mem::discriminant(new.item)
                    && old.children.len() == new.children.len()
            });

        if !same_shape {
            return MenuChange::Layout;
        }

        let mut updated = Vec::new();
        let mut removed = Vec::new();

        for ((old, new), id) in old.iter().zip(&new).zip(1..) {
            if old.item == new.item {
                continue;
            }

            let before = item_properties(old.item);
            let mut after = item_properties(new.item);

            let gone: Vec<String> = before
                .keys()
                .filter(|name| !after.contains_key(*name))
                .cloned()
                .collect();

            after.retain(|name, value| before.get(name) != Some(value));

            if !after.is_empty() {
                updated.push((id, after));
            }

            if !gone.is_empty() {
                removed.push((id, gone));
            }
        }

        MenuChange::Properties { updated, removed }
    }
}

/// An item of the menu with the identifiers of the items of its submenu.
struct Entry<'a> {
    item: &'a MenuItem,
    children: Vec<i32>,
}

/// The items of a menu and of every menu inside it, numbered from 1 as
/// they are met from the top, an item before the ones of its submenu: the
/// item with the identifier `i` is at `i - 1`. Also returns the
/// identifiers of the items of the menu itself.
fn entries(menu: &Menu) -> (Vec<i32>, Vec<Entry<'_>>) {
    fn walk<'a>(menu: &'a Menu, entries: &mut Vec<Entry<'a>>) -> Vec<i32> {
        menu.items
            .iter()
            .map(|item| {
                let index = entries.len();

                entries.push(Entry {
                    item,
                    children: Vec::new(),
                });

                if let MenuItem::Submenu { menu, .. } = item {
                    entries[index].children = walk(menu, entries);
                }

                index as i32 + 1
            })
            .collect()
    }

    let mut entries = Vec::new();
    let root = walk(menu, &mut entries);

    (root, entries)
}

/// A picture as the bytes of a PNG file: the `icon-data` of an item.
fn png(icon: &Icon) -> Vec<u8> {
    let mut bytes = Vec::new();

    let mut encoder =
        png::Encoder::new(&mut bytes, icon.width(), icon.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let written = encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(icon.rgba()));

    match written {
        Ok(()) => bytes,
        Err(error) => {
            log::error!("the picture of a menu item is not a PNG: {error}");
            Vec::new()
        }
    }
}

/// A picture as `a(iiay)`: width, height, and the pixels as ARGB in
/// network byte order.
type Pixmap = (i32, i32, Vec<u8>);

fn pixmap(icon: &Icon) -> Pixmap {
    let argb = icon
        .rgba()
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
        .collect();

    (icon.width() as i32, icon.height() as i32, argb)
}

struct StatusNotifierItem {
    state: State,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItem {
    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".to_owned()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "iced".to_owned())
    }

    #[zbus(property)]
    fn title(&self) -> String {
        lock(&self.state).settings.title().to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "Active".to_owned()
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        vec![pixmap(&lock(&self.state).settings.icon)]
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Vec<Pixmap> {
        Vec::new()
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Vec<Pixmap> {
        Vec::new()
    }

    #[zbus(property)]
    fn attention_movie_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        String::new()
    }

    /// The tooltip: the name of an icon, an icon, a title and a text.
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
        (
            String::new(),
            Vec::new(),
            lock(&self.state).settings.tooltip.clone(),
            String::new(),
        )
    }

    /// `false`: the main button activates the item, the other one opens
    /// the menu.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    /// The icon was clicked with the main button.
    fn activate(&self, _x: i32, _y: i32) {
        super::emit(Event::Click);
    }

    /// The icon was clicked with the middle button.
    fn secondary_activate(&self, _x: i32, _y: i32) {}

    /// The desktop asks the item to show its own menu; it draws the one of
    /// [`DbusMenu`] itself instead.
    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_status(
        emitter: &SignalEmitter<'_>,
        status: &str,
    ) -> zbus::Result<()>;
}

/// The properties of an item of a menu, as `a{sv}`.
type Properties = HashMap<String, Value<'static>>;

/// An item of a menu with the ones below it, as `(ia{sv}av)`.
type Layout = (i32, Properties, Vec<Value<'static>>);

/// The identifier of the root of the menu. The item at index `i` of the
/// [`Menu`] has the identifier `i + 1`.
const ROOT: i32 = 0;

fn root_properties() -> Properties {
    HashMap::from([("children-display".to_owned(), Value::from("submenu"))])
}

fn item_properties(item: &MenuItem) -> Properties {
    match item {
        MenuItem::Separator => {
            HashMap::from([("type".to_owned(), Value::from("separator"))])
        }
        MenuItem::Submenu {
            label,
            enabled,
            icon,
            ..
        } => {
            let mut properties = HashMap::from([
                ("label".to_owned(), Value::from(label.replace('_', "__"))),
                ("enabled".to_owned(), Value::from(*enabled)),
                ("children-display".to_owned(), Value::from("submenu")),
            ]);

            if let Some(icon) = icon {
                let _ = properties
                    .insert("icon-data".to_owned(), Value::from(png(icon)));
            }

            properties
        }
        MenuItem::Button {
            label,
            enabled,
            checked,
            icon,
            ..
        } => {
            let mut properties = HashMap::from([
                // An underscore marks the next letter as the access key
                ("label".to_owned(), Value::from(label.replace('_', "__"))),
                ("enabled".to_owned(), Value::from(*enabled)),
            ]);

            if let Some(icon) = icon {
                let _ = properties
                    .insert("icon-data".to_owned(), Value::from(png(icon)));
            }

            if let Some(checked) = checked {
                let _ = properties
                    .insert("toggle-type".to_owned(), Value::from("checkmark"));
                let _ = properties.insert(
                    "toggle-state".to_owned(),
                    Value::from(i32::from(*checked)),
                );
            }

            properties
        }
    }
}

fn entry<'a, 'b>(entries: &'b [Entry<'a>], id: i32) -> Option<&'b Entry<'a>> {
    entries.get(usize::try_from(id).ok()?.checked_sub(1)?)
}

/// The item with the given identifier and, as far as `depth` allows (a
/// negative one allows all), the items below it.
fn layout_of(
    entries: &[Entry<'_>],
    id: i32,
    depth: i32,
    names: &[String],
) -> Option<Layout> {
    let entry = entry(entries, id)?;

    Some((
        id,
        filter(item_properties(entry.item), names),
        children_of(entries, &entry.children, depth, names),
    ))
}

fn children_of(
    entries: &[Entry<'_>],
    children: &[i32],
    depth: i32,
    names: &[String],
) -> Vec<Value<'static>> {
    if depth == 0 {
        return Vec::new();
    }

    children
        .iter()
        .filter_map(|child| layout_of(entries, *child, depth - 1, names))
        .map(|layout| Value::from(Structure::from(layout)))
        .collect()
}

/// Keeps the properties that were asked for; an empty list asks for all.
fn filter(mut properties: Properties, names: &[String]) -> Properties {
    if !names.is_empty() {
        properties.retain(|name, _| names.contains(name));
    }

    properties
}

struct DbusMenu {
    state: State,
}

impl DbusMenu {
    fn activate(&self, id: i32, event: &str) {
        if event != "clicked" {
            return;
        }

        let shared = lock(&self.state);
        let (_, entries) = entries(&shared.settings.menu);

        if let Some(MenuItem::Button {
            id, enabled: true, ..
        }) = entry(&entries, id).map(|entry| entry.item)
        {
            super::emit(Event::MenuItem(id.clone()));
        }
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let shared = lock(&self.state);
        let menu = &shared.settings.menu;

        let (root, entries) = entries(menu);

        let layout = if parent_id == ROOT {
            (
                ROOT,
                filter(root_properties(), &property_names),
                children_of(&entries, &root, recursion_depth, &property_names),
            )
        } else if let Some(layout) =
            layout_of(&entries, parent_id, recursion_depth, &property_names)
        {
            layout
        } else {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "the menu has no item {parent_id}"
            )));
        };

        Ok((shared.revision, layout))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, Properties)> {
        let shared = lock(&self.state);
        let menu = &shared.settings.menu;

        let (_, entries) = entries(menu);

        let all: Vec<i32>;
        let ids = if ids.is_empty() {
            all = (ROOT..=entries.len() as i32).collect();
            &all
        } else {
            &ids
        };

        ids.iter()
            .filter_map(|id| {
                let properties = if *id == ROOT {
                    root_properties()
                } else {
                    item_properties(entry(&entries, *id)?.item)
                };

                Some((*id, filter(properties, &property_names)))
            })
            .collect()
    }

    fn get_property(
        &self,
        id: i32,
        name: &str,
    ) -> zbus::fdo::Result<Value<'static>> {
        let shared = lock(&self.state);

        let mut properties = if id == ROOT {
            root_properties()
        } else {
            let (_, entries) = entries(&shared.settings.menu);

            entry(&entries, id)
                .map(|entry| item_properties(entry.item))
                .unwrap_or_default()
        };

        properties.remove(name).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs(format!(
                "item {id} of the menu has no property {name}"
            ))
        })
    }

    fn event(
        &self,
        id: i32,
        event_id: &str,
        _data: Value<'_>,
        _timestamp: u32,
    ) {
        self.activate(id, event_id);
    }

    /// Returns the identifiers that are not of the menu.
    fn event_group(
        &self,
        events: Vec<(i32, String, Value<'_>, u32)>,
    ) -> Vec<i32> {
        let mut unknown = Vec::new();

        for (id, event_id, _data, _timestamp) in events {
            let known = id == ROOT || {
                let shared = lock(&self.state);
                let (_, entries) = entries(&shared.settings.menu);

                entry(&entries, id).is_some()
            };

            if known {
                self.activate(id, &event_id);
            } else {
                unknown.push(id);
            }
        }

        unknown
    }

    /// Returns whether the menu has to be asked for again: never, the
    /// layout is announced when it changes.
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".to_owned()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated: Vec<(i32, Properties)>,
        removed: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_activation_requested(
        emitter: &SignalEmitter<'_>,
        id: i32,
        timestamp: u32,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    //! The item is served on a bus of its own (a `dbus-daemon` started for
    //! the test) and a second connection plays the desktop: the watcher
    //! that takes the registration and the host that reads the icon and
    //! the menu and clicks.
    use super::*;

    use crate::futures::futures::executor::block_on;
    use crate::runtime::tray::Item;

    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    struct Bus {
        daemon: Child,
        address: String,
    }

    impl Bus {
        /// Starts a session bus of its own; `None` where there is no
        /// `dbus-daemon` to start.
        fn start() -> Option<Self> {
            let mut daemon = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;

            let mut address = String::new();
            let _ = BufReader::new(daemon.stdout.take()?)
                .read_line(&mut address)
                .ok()?;

            Some(Self {
                daemon,
                address: address.trim().to_owned(),
            })
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    /// The watcher of a desktop: it remembers who registered, and says
    /// whether something draws the icons.
    struct Watcher {
        registered: mpsc::UnboundedSender<String>,
        hosted: Arc<std::sync::atomic::AtomicBool>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(&self, service: &str) {
            let _ = self.registered.unbounded_send(service.to_owned());
        }

        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            self.hosted.load(Ordering::SeqCst)
        }

        #[zbus(signal)]
        async fn status_notifier_host_registered(
            emitter: &SignalEmitter<'_>,
        ) -> zbus::Result<()>;
    }

    fn dot(red: u8) -> Icon {
        Icon::from_rgba(vec![red, 0, 0, 255], 1, 1).unwrap()
    }

    /// The pixels of a PNG file.
    fn pixels(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        pixels.truncate(info.buffer_size());

        (info.width, info.height, pixels)
    }

    fn icon(red: u8) -> Icon {
        Icon::from_rgba(vec![red, 2, 3, 4, red, 6, 7, 8], 2, 1).unwrap()
    }

    fn settings() -> Settings {
        Settings::new(icon(1))
            .name("Example")
            .tooltip("Up to date")
            .menu([
                Item::label("Up to date"),
                Item::separator(),
                Item::button("open", "Open the _folder"),
                Item::check("pause", "Pause", false),
                Item::button("quit", "Quit").enabled(false),
                Item::submenu(
                    "Recent",
                    [
                        Item::button("recent-a", "a.txt").icon(dot(7)),
                        Item::submenu(
                            "Older",
                            [Item::button("recent-b", "b.txt")],
                        ),
                    ],
                )
                .icon(dot(200)),
            ])
    }

    /// The items below an item of a layout.
    fn children(layout: &OwnedLayout) -> Vec<OwnedLayout> {
        layout
            .2
            .iter()
            .map(|child| child.try_clone().unwrap().try_into().unwrap())
            .collect()
    }

    fn bytes(layout: &OwnedLayout, name: &str) -> Option<Vec<u8>> {
        layout.1.get(name).map(|value| {
            Vec::<u8>::try_from(value.try_clone().unwrap()).unwrap()
        })
    }

    fn string(layout: &OwnedLayout, name: &str) -> Option<String> {
        layout
            .1
            .get(name)
            .map(|value| String::try_from(value.try_clone().unwrap()).unwrap())
    }

    async fn host(address: &str) -> zbus::Connection {
        zbus::connection::Builder::address(address)
            .unwrap()
            .build()
            .await
            .unwrap()
    }

    async fn watcher(
        address: &str,
    ) -> (zbus::Connection, mpsc::UnboundedReceiver<String>) {
        let (connection, registrations, _) = watcher_with(address, true).await;

        (connection, registrations)
    }

    /// A watcher, and whether something draws its icons (which can be
    /// changed: [`host_arrives`]).
    async fn watcher_with(
        address: &str,
        is_hosted: bool,
    ) -> (
        zbus::Connection,
        mpsc::UnboundedReceiver<String>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (registered, registrations) = mpsc::unbounded();
        let hosted = Arc::new(std::sync::atomic::AtomicBool::new(is_hosted));

        let connection = zbus::connection::Builder::address(address)
            .unwrap()
            .serve_at(
                WATCHER_PATH,
                Watcher {
                    registered,
                    hosted: hosted.clone(),
                },
            )
            .unwrap()
            .name(WATCHER)
            .unwrap()
            .build()
            .await
            .unwrap();

        (connection, registrations, hosted)
    }

    /// A panel starts to draw the icons of the watcher.
    async fn host_arrives(
        watcher: &zbus::Connection,
        hosted: &std::sync::atomic::AtomicBool,
    ) {
        hosted.store(true, Ordering::SeqCst);

        let interface = watcher
            .object_server()
            .interface::<_, Watcher>(WATCHER_PATH)
            .await
            .unwrap();

        Watcher::status_notifier_host_registered(interface.signal_emitter())
            .await
            .unwrap();
    }

    async fn property<T>(
        host: &zbus::Connection,
        item: &str,
        path: &str,
        interface: &str,
        name: &str,
    ) -> T
    where
        T: TryFrom<zbus::zvariant::OwnedValue>,
        T::Error: std::fmt::Debug,
    {
        let reply = host
            .call_method(
                Some(item),
                path,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(interface, name),
            )
            .await
            .unwrap();

        let value: zbus::zvariant::OwnedValue =
            reply.body().deserialize().unwrap();

        T::try_from(value).unwrap()
    }

    type OwnedLayout = (
        i32,
        HashMap<String, zbus::zvariant::OwnedValue>,
        Vec<zbus::zvariant::OwnedValue>,
    );

    async fn layout(host: &zbus::Connection, item: &str) -> (u32, OwnedLayout) {
        host.call_method(
            Some(item),
            MENU_PATH,
            Some("com.canonical.dbusmenu"),
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()),
        )
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap()
    }

    fn labels(layout: &OwnedLayout) -> Vec<String> {
        layout
            .2
            .iter()
            .map(|child| {
                let (_id, properties, _children): OwnedLayout =
                    child.try_clone().unwrap().try_into().unwrap();

                match properties.get("label") {
                    Some(label) => {
                        String::try_from(label.try_clone().unwrap()).unwrap()
                    }
                    None => "-".to_owned(),
                }
            })
            .collect()
    }

    #[test]
    fn the_item_registers_shows_its_icon_and_menu_and_tells_what_is_clicked() {
        let Some(bus) = Bus::start() else {
            eprintln!("no dbus-daemon here: the tray is not tested");
            return;
        };

        let _alone = super::super::TESTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        super::super::forget();
        let mut events = super::super::listen();

        block_on(async {
            let (_watcher, mut registrations) = watcher(&bus.address).await;
            let host = host(&bus.address).await;

            let (updates, receiver) = mpsc::unbounded();
            let serving = serve(settings(), receiver, Some(&bus.address));

            let playing = async {
                // The item tells the watcher its name
                let item = registrations.next().await.unwrap();
                assert!(
                    item.starts_with(&format!(
                        "org.kde.StatusNotifierItem-{}-",
                        std::process::id()
                    )),
                    "{item}"
                );

                // The icon: 2x1 pixels, ARGB
                let pixmaps: Vec<Pixmap> = property(
                    &host,
                    &item,
                    ITEM_PATH,
                    "org.kde.StatusNotifierItem",
                    "IconPixmap",
                )
                .await;
                assert_eq!(pixmaps, vec![(2, 1, vec![4, 1, 2, 3, 8, 1, 6, 7])]);

                let tooltip: (String, Vec<Pixmap>, String, String) = property(
                    &host,
                    &item,
                    ITEM_PATH,
                    "org.kde.StatusNotifierItem",
                    "ToolTip",
                )
                .await;
                assert_eq!(tooltip.2, "Up to date");

                // The item is called by the name of the program, not by
                // its status
                let title: String = property(
                    &host,
                    &item,
                    ITEM_PATH,
                    "org.kde.StatusNotifierItem",
                    "Title",
                )
                .await;
                assert_eq!(title, "Example");

                let menu: zbus::zvariant::OwnedObjectPath = property(
                    &host,
                    &item,
                    ITEM_PATH,
                    "org.kde.StatusNotifierItem",
                    "Menu",
                )
                .await;
                assert_eq!(menu.as_str(), MENU_PATH);

                // The menu, as the desktop reads it
                let (revision, root) = layout(&host, &item).await;
                assert_eq!(revision, 1);
                assert_eq!(
                    labels(&root),
                    [
                        "Up to date",
                        "-",
                        "Open the __folder",
                        "Pause",
                        "Quit",
                        "Recent"
                    ]
                );

                // "Recent" opens a menu of its own and has a picture, as
                // has the first item inside; the second opens one more
                let recent = &children(&root)[5];
                assert_eq!(recent.0, 6);
                assert_eq!(
                    string(recent, "children-display").as_deref(),
                    Some("submenu")
                );
                assert_eq!(
                    pixels(&bytes(recent, "icon-data").unwrap()),
                    (1, 1, vec![200, 0, 0, 255])
                );

                let inside = children(recent);
                assert_eq!(labels(recent), ["a.txt", "Older"]);
                assert_eq!(inside[0].0, 7);
                assert_eq!(
                    pixels(&bytes(&inside[0], "icon-data").unwrap()),
                    (1, 1, vec![7, 0, 0, 255])
                );
                assert_eq!(string(&inside[0], "children-display"), None);
                assert_eq!(inside[1].0, 8);
                assert_eq!(labels(&inside[1]), ["b.txt"]);
                assert_eq!(children(&inside[1])[0].0, 9);
                assert_eq!(bytes(&children(&inside[1])[0], "icon-data"), None);

                // Asked for one level only, the desktop gets one level;
                // asked for a submenu, it gets that
                let (_, shallow): (u32, OwnedLayout) = host
                    .call_method(
                        Some(item.as_str()),
                        MENU_PATH,
                        Some("com.canonical.dbusmenu"),
                        "GetLayout",
                        &(0i32, 1i32, Vec::<String>::new()),
                    )
                    .await
                    .unwrap()
                    .body()
                    .deserialize()
                    .unwrap();
                assert_eq!(children(&shallow).len(), 6);
                assert!(children(&shallow)[5].2.is_empty());

                let (_, older): (u32, OwnedLayout) = host
                    .call_method(
                        Some(item.as_str()),
                        MENU_PATH,
                        Some("com.canonical.dbusmenu"),
                        "GetLayout",
                        &(8i32, -1i32, vec!["label".to_owned()]),
                    )
                    .await
                    .unwrap()
                    .body()
                    .deserialize()
                    .unwrap();
                assert_eq!(string(&older, "label").as_deref(), Some("Older"));
                assert_eq!(string(&older, "children-display"), None);
                assert_eq!(labels(&older), ["b.txt"]);

                // A click on the icon, on "Open", on the disabled "Quit"
                // (which tells nothing) and on "Pause"
                let _ = host
                    .call_method(
                        Some(item.as_str()),
                        ITEM_PATH,
                        Some("org.kde.StatusNotifierItem"),
                        "Activate",
                        &(0i32, 0i32),
                    )
                    .await
                    .unwrap();

                // ... then on "Recent" itself (which only opens), on
                // "b.txt" two menus deep and on "a.txt"
                for id in [3i32, 5, 4, 6, 9, 7] {
                    let _ = host
                        .call_method(
                            Some(item.as_str()),
                            MENU_PATH,
                            Some("com.canonical.dbusmenu"),
                            "Event",
                            &(id, "clicked", Value::from(0i32), 0u32),
                        )
                        .await
                        .unwrap();
                }

                // The watcher has a host: the desktop shows the icon
                assert_eq!(events.next().await, Some(Event::Available));
                assert_eq!(events.next().await, Some(Event::Click));
                assert_eq!(
                    events.next().await,
                    Some(Event::MenuItem("open".to_owned()))
                );
                assert_eq!(
                    events.next().await,
                    Some(Event::MenuItem("pause".to_owned()))
                );
                assert_eq!(
                    events.next().await,
                    Some(Event::MenuItem("recent-b".to_owned()))
                );
                assert_eq!(
                    events.next().await,
                    Some(Event::MenuItem("recent-a".to_owned()))
                );

                // The program changes the menu and the icon: the desktop
                // is told, and reads the new ones
                let mut changes = zbus::MessageStream::for_match_rule(
                    zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .sender(item.as_str())
                        .unwrap()
                        .build(),
                    &host,
                    None,
                )
                .await
                .unwrap();

                let mut changed = settings();
                changed.icon = icon(9);
                changed.tooltip = "Paused".to_owned();
                changed.menu.items[0] = Item::label("Paused");
                changed.menu.items[3] = Item::check("pause", "Pause", true);
                updates.unbounded_send(changed.clone()).unwrap();

                // Then a file renamed inside the submenu: still in place
                changed.menu.items[5] = Item::submenu(
                    "Recent",
                    [
                        Item::button("recent-a", "a.txt").icon(dot(7)),
                        Item::submenu(
                            "Older",
                            [Item::button("recent-c", "c.txt")],
                        ),
                    ],
                )
                .icon(dot(200));
                updates.unbounded_send(changed.clone()).unwrap();

                // Then an item more inside it: only that is a new layout
                if let Item::Submenu { menu, .. } = &mut changed.menu.items[5] {
                    menu.items.push(Item::button("more", "More"));
                }
                updates.unbounded_send(changed).unwrap();

                let mut signals = Vec::new();
                while signals.len() < 5 {
                    let message = changes.next().await.unwrap().unwrap();
                    signals
                        .push(message.header().member().unwrap().to_string());
                }
                assert_eq!(
                    signals,
                    [
                        "NewIcon",
                        "NewToolTip",
                        "ItemsPropertiesUpdated",
                        "ItemsPropertiesUpdated",
                        "LayoutUpdated"
                    ]
                );

                // The renamed file is chosen by its new identifier
                let _ = host
                    .call_method(
                        Some(item.as_str()),
                        MENU_PATH,
                        Some("com.canonical.dbusmenu"),
                        "Event",
                        &(9i32, "clicked", Value::from(0i32), 0u32),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    events.next().await,
                    Some(Event::MenuItem("recent-c".to_owned()))
                );

                let (revision, root) = layout(&host, &item).await;
                assert_eq!(revision, 2);
                assert_eq!(labels(&root)[0], "Paused");

                let pixmaps: Vec<Pixmap> = property(
                    &host,
                    &item,
                    ITEM_PATH,
                    "org.kde.StatusNotifierItem",
                    "IconPixmap",
                )
                .await;
                assert_eq!(pixmaps[0].2[1], 9);

                // The panel is restarted: a new watcher appears and the
                // item registers with it
                drop(_watcher);
                let (_second, mut registrations) = watcher(&bus.address).await;
                assert_eq!(registrations.next().await.unwrap(), item);

                // In between there was no tray
                assert_eq!(events.next().await, Some(Event::Unavailable));
                assert_eq!(events.next().await, Some(Event::Available));

                // The program takes the icon away: its name leaves the bus
                drop(updates);
                item
            };

            let (served, item) = future::join(serving, playing).await;
            served.unwrap();

            let dbus = zbus::fdo::DBusProxy::new(&host).await.unwrap();
            let name = zbus::names::BusName::try_from(item).unwrap();
            assert!(!dbus.name_has_owner(name).await.unwrap());
        });
    }

    #[test]
    fn a_desktop_without_a_tray_is_told_and_a_tray_that_comes_later_too() {
        let Some(bus) = Bus::start() else {
            eprintln!("no dbus-daemon here: the tray is not tested");
            return;
        };

        let _alone = super::super::TESTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        super::super::forget();
        let mut events = super::super::listen();

        block_on(async {
            let (updates, receiver) = mpsc::unbounded();
            let serving = serve(settings(), receiver, Some(&bus.address));

            let playing = async {
                // Nothing on the bus watches for icons: no tray here
                assert_eq!(events.next().await, Some(Event::Unavailable));
                assert_eq!(super::super::is_available(), Some(false));

                // A watcher comes, with nothing that draws its icons yet
                // (the service of the desktop before its panel): the item
                // registers, and there is still no tray
                let (watcher, mut registrations, hosted) =
                    watcher_with(&bus.address, false).await;
                let item = registrations.next().await.unwrap();
                assert_eq!(super::super::is_available(), Some(false));

                // The panel comes
                host_arrives(&watcher, &hosted).await;
                assert_eq!(events.next().await, Some(Event::Available));
                assert_eq!(super::super::is_available(), Some(true));

                // Who listens from now on is told at once
                let mut late = super::super::listen();
                assert_eq!(late.next().await, Some(Event::Available));

                // The watcher leaves the bus: no tray again
                drop(watcher);
                assert_eq!(events.next().await, Some(Event::Unavailable));
                assert_eq!(late.next().await, Some(Event::Unavailable));

                drop(updates);
                item
            };

            let (served, _item) = future::join(serving, playing).await;
            served.unwrap();
        });

        super::super::forget();
    }

    #[test]
    fn a_menu_that_keeps_its_shape_changes_in_place() {
        let menu = |name: &str, more: bool| {
            let mut inside = vec![Item::button("a", name)];

            if more {
                inside.push(Item::separator());
            }

            Menu::new([
                Item::label("Status"),
                Item::submenu("Recent", inside),
                Item::button("quit", "Quit"),
            ])
        };

        assert!(matches!(
            MenuChange::between(&menu("a.txt", false), &menu("a.txt", false)),
            MenuChange::None
        ));

        // A label inside a submenu: the item 3 (1 "Status", 2 "Recent",
        // 3 the one inside, 4 "Quit")
        match MenuChange::between(&menu("a.txt", false), &menu("b.txt", false))
        {
            MenuChange::Properties { updated, removed } => {
                assert_eq!(updated.len(), 1);
                assert_eq!(updated[0].0, 3);
                assert_eq!(
                    updated[0].1.get("label"),
                    Some(&Value::from("b.txt"))
                );
                assert!(removed.is_empty());
            }
            _ => panic!("not in place"),
        }

        // One more item inside: a new layout
        assert!(matches!(
            MenuChange::between(&menu("a.txt", false), &menu("a.txt", true)),
            MenuChange::Layout
        ));

        // A picture that goes away is a property that goes away
        let with = Menu::new([Item::button("a", "A").icon(dot(1))]);
        let without = Menu::new([Item::button("a", "A")]);

        match MenuChange::between(&with, &without) {
            MenuChange::Properties { updated, removed } => {
                assert!(updated.is_empty());
                assert_eq!(removed, [(1, vec!["icon-data".to_owned()])]);
            }
            _ => panic!("not in place"),
        }
    }
}
