//! The tray icon on macOS and Windows: the `tray-icon` crate. It has to be
//! made, changed and dropped on the thread of the event loop.
use crate::runtime::tray::{Event, Icon, Item, Menu, Settings};

use tray_icon::menu::{CheckMenuItem, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use std::sync::Once;

pub struct Tray {
    raw: TrayIcon,
    settings: Settings,
}

impl Tray {
    pub fn new(settings: Settings) -> Option<Self> {
        static HANDLERS: Once = Once::new();

        HANDLERS.call_once(|| {
            MenuEvent::set_event_handler(Some(|event: MenuEvent| {
                super::emit(Event::MenuItem(event.id.0));
            }));

            TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    super::emit(Event::Click);
                }
            }));
        });

        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu(&settings.menu)))
            // On macOS a click on a status item opens its menu; on Windows
            // the main button activates and the other one opens the menu
            .with_menu_on_left_click(cfg!(target_os = "macos"));

        if let Some(icon) = icon(&settings.icon) {
            builder = builder.with_icon(icon);
        }

        if !settings.tooltip.is_empty() {
            builder = builder.with_tooltip(&settings.tooltip);
        }

        match builder.build() {
            Ok(raw) => Some(Self { raw, settings }),
            Err(error) => {
                log::error!("the tray icon could not be made: {error}");
                None
            }
        }
    }

    pub fn update(&mut self, settings: Settings) {
        if settings.icon != self.settings.icon
            && let Err(error) = self.raw.set_icon(icon(&settings.icon))
        {
            log::error!("the tray icon could not be changed: {error}");
        }

        if settings.tooltip != self.settings.tooltip
            && let Err(error) = self.raw.set_tooltip(
                (!settings.tooltip.is_empty()).then_some(&settings.tooltip),
            )
        {
            log::error!("the tray tooltip could not be changed: {error}");
        }

        if settings.menu != self.settings.menu {
            self.raw.set_menu(Some(Box::new(menu(&settings.menu))));
        }

        self.settings = settings;
    }
}

fn icon(icon: &Icon) -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(
        icon.rgba().to_vec(),
        icon.width(),
        icon.height(),
    )
    .map_err(|error| log::error!("the tray icon is not valid: {error}"))
    .ok()
}

fn menu(menu: &Menu) -> tray_icon::menu::Menu {
    let raw = tray_icon::menu::Menu::new();

    for item in &menu.items {
        let result = match item {
            Item::Button {
                id,
                label,
                enabled,
                checked: None,
            } => raw.append(&MenuItem::with_id(
                id.as_str(),
                label,
                *enabled,
                None,
            )),
            Item::Button {
                id,
                label,
                enabled,
                checked: Some(checked),
            } => raw.append(&CheckMenuItem::with_id(
                id.as_str(),
                label,
                *enabled,
                *checked,
                None,
            )),
            Item::Separator => raw.append(&PredefinedMenuItem::separator()),
        };

        if let Err(error) = result {
            log::error!("an item of the tray menu could not be added: {error}");
        }
    }

    raw
}
