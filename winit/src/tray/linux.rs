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
                future::ready(
                    signal
                        .args()
                        .ok()
                        .and_then(|args| {
                            args.new_owner().is_some().then_some(())
                        })
                        .map(|()| Wake::WatcherAppeared),
                )
            });

        register(&connection, &name).await;

        let mut wakes = stream::select(
            updates
                .map(Wake::Update)
                .chain(stream::once(future::ready(Wake::Closed))),
            appearances,
        );

        while let Some(wake) = wakes.next().await {
            match wake {
                Wake::Update(settings) => {
                    apply(&connection, &state, settings).await?;
                }
                Wake::WatcherAppeared => {
                    register(&connection, &name).await;
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

/// Tells the watcher of the desktop about the item. With no watcher (no
/// tray on this desktop, yet) there is nothing to tell.
async fn register(connection: &zbus::Connection, name: &str) {
    let result = connection
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(WATCHER),
            "RegisterStatusNotifierItem",
            &(name,),
        )
        .await;

    if let Err(error) = result {
        log::info!(
            "no tray on this desktop for now ({WATCHER} did not take the \
            icon: {error})"
        );
    }
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

        let same_shape = old.items.len() == new.items.len()
            && old.items.iter().zip(&new.items).all(|(old, new)| {
                matches!(old, MenuItem::Separator)
                    == matches!(new, MenuItem::Separator)
            });

        if !same_shape {
            return MenuChange::Layout;
        }

        let mut updated = Vec::new();
        let mut removed = Vec::new();

        for ((old, new), id) in old.items.iter().zip(&new.items).zip(1..) {
            if old == new {
                continue;
            }

            let before = item_properties(old);
            let mut after = item_properties(new);

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
        MenuItem::Button {
            label,
            enabled,
            checked,
            ..
        } => {
            let mut properties = HashMap::from([
                // An underscore marks the next letter as the access key
                ("label".to_owned(), Value::from(label.replace('_', "__"))),
                ("enabled".to_owned(), Value::from(*enabled)),
            ]);

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

fn item(menu: &Menu, id: i32) -> Option<&MenuItem> {
    menu.items.get(usize::try_from(id).ok()?.checked_sub(1)?)
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

        if let Some(MenuItem::Button {
            id, enabled: true, ..
        }) = item(&shared.settings.menu, id)
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

        let layout = if parent_id == ROOT {
            let children = if recursion_depth == 0 {
                Vec::new()
            } else {
                menu.items
                    .iter()
                    .zip(1..)
                    .map(|(item, id): (_, i32)| {
                        Value::from(Structure::from((
                            id,
                            filter(item_properties(item), &property_names),
                            Vec::<Value<'static>>::new(),
                        )))
                    })
                    .collect()
            };

            (ROOT, filter(root_properties(), &property_names), children)
        } else if let Some(item) = item(menu, parent_id) {
            (
                parent_id,
                filter(item_properties(item), &property_names),
                Vec::new(),
            )
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

        let all: Vec<i32>;
        let ids = if ids.is_empty() {
            all = (ROOT..=menu.items.len() as i32).collect();
            &all
        } else {
            &ids
        };

        ids.iter()
            .filter_map(|id| {
                let properties = if *id == ROOT {
                    root_properties()
                } else {
                    item_properties(item(menu, *id)?)
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
            item(&shared.settings.menu, id)
                .map(item_properties)
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
            let known = id == ROOT
                || item(&lock(&self.state).settings.menu, id).is_some();

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

    /// The watcher of a desktop: it remembers who registered.
    struct Watcher {
        registered: mpsc::UnboundedSender<String>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(&self, service: &str) {
            let _ = self.registered.unbounded_send(service.to_owned());
        }
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
            ])
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
        let (registered, registrations) = mpsc::unbounded();

        let connection = zbus::connection::Builder::address(address)
            .unwrap()
            .serve_at(WATCHER_PATH, Watcher { registered })
            .unwrap()
            .name(WATCHER)
            .unwrap()
            .build()
            .await
            .unwrap();

        (connection, registrations)
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
                    ["Up to date", "-", "Open the __folder", "Pause", "Quit"]
                );

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

                for id in [3i32, 5, 4] {
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

                assert_eq!(events.next().await, Some(Event::Click));
                assert_eq!(
                    events.next().await,
                    Some(Event::MenuItem("open".to_owned()))
                );
                assert_eq!(
                    events.next().await,
                    Some(Event::MenuItem("pause".to_owned()))
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

                // Then an item more: only that is a new layout
                changed.menu.items.push(Item::button("more", "More"));
                updates.unbounded_send(changed).unwrap();

                let mut signals = Vec::new();
                while signals.len() < 4 {
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
                        "LayoutUpdated"
                    ]
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
}
