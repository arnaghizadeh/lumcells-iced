//! Bring an existing window to the front (`window::present`).
//!
//! Every system has its own way, and Wayland has one only for a program
//! that holds a token: see [`Way`].

/// How a window is brought to the front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Way {
    /// Wayland, with the activation token that a later start of the
    /// program was given by whatever started it and handed to this one
    /// (`single_instance`): `xdg_activation_v1.activate(token, surface)`.
    Activate(String),
    /// Wayland without a token: a program may not come to the front by
    /// itself there. The compositor is asked to draw attention to the
    /// window (most mark it in their panel or dock).
    Attention,
    /// Everywhere else: the window asks for the focus (`_NET_ACTIVE_WINDOW`
    /// on X11, `SetForegroundWindow` on Windows, activating the
    /// application and `makeKeyAndOrderFront:` on macOS).
    Focus,
}

impl Way {
    /// The [`Way`] for a window on Wayland or elsewhere, with the token
    /// of the latest start if there is one. An empty token is none.
    pub fn of(is_wayland: bool, token: Option<String>) -> Self {
        match (is_wayland, token.filter(|token| !token.is_empty())) {
            (true, Some(token)) => Way::Activate(token),
            (true, None) => Way::Attention,
            (false, _) => Way::Focus,
        }
    }
}

/// Brings the window to the front: shown if it was hidden, restored if
/// it was minimized, then in the [`Way`] of the system. `token` is the
/// activation token of the latest start of the program, if any; it is
/// used up.
pub(crate) fn present(window: &winit::window::Window, token: Option<String>) {
    if window.is_visible() == Some(false) {
        window.set_visible(true);
    }

    // On X11 this is already the request to be the active window
    if window.is_minimized() == Some(true) {
        window.set_minimized(false);
    }

    match Way::of(is_wayland(window), token) {
        Way::Activate(token) => {
            #[cfg(all(feature = "wayland", target_os = "linux"))]
            if let Err(error) = wayland::activate(window, &token) {
                log::warn!(
                    "the window could not be activated with the token of \
                    the latest start: {error}"
                );

                window.request_user_attention(Some(
                    winit::window::UserAttentionType::Informational,
                ));
            }

            #[cfg(not(all(feature = "wayland", target_os = "linux")))]
            let _ = token;
        }
        Way::Attention => {
            window.request_user_attention(Some(
                winit::window::UserAttentionType::Informational,
            ));
        }
        Way::Focus => {
            window.focus_window();
        }
    }
}

fn is_wayland(window: &winit::window::Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    window.window_handle().is_ok_and(|handle| {
        matches!(handle.as_raw(), RawWindowHandle::Wayland(_))
    })
}

#[cfg(all(feature = "wayland", target_os = "linux"))]
mod wayland {
    //! `winit` 0.30 gives an activation token only to a window it is
    //! about to make (`with_activation_token`); its `focus_window` does
    //! nothing on Wayland. So the request is made here, on the
    //! connection `winit` holds, through a registry and a queue of our
    //! own (as the clipboard does on the same connection).
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::WlRegistry;
    use wayland_client::protocol::wl_surface::WlSurface;
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

    use winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    };

    struct State;

    impl Dispatch<WlRegistry, GlobalListContents> for State {
        fn event(
            _state: &mut Self,
            _registry: &WlRegistry,
            _event: <WlRegistry as Proxy>::Event,
            _data: &GlobalListContents,
            _connection: &Connection,
            _queue: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<XdgActivationV1, ()> for State {
        fn event(
            _state: &mut Self,
            _activation: &XdgActivationV1,
            _event: <XdgActivationV1 as Proxy>::Event,
            _data: &(),
            _connection: &Connection,
            _queue: &QueueHandle<Self>,
        ) {
        }
    }

    #[allow(unsafe_code)]
    pub fn activate(
        window: &winit::window::Window,
        token: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (
            RawDisplayHandle::Wayland(display),
            RawWindowHandle::Wayland(surface),
        ) = (
            window.display_handle()?.as_raw(),
            window.window_handle()?.as_raw(),
        )
        else {
            return Err("not a window of Wayland".into());
        };

        // SAFETY: the pointer is the `wl_display` of the connection that
        // `winit` keeps open for as long as the window (which is borrowed
        // here) lives. A backend made from a foreign display does not
        // close it when it is dropped.
        let backend = unsafe {
            Backend::from_foreign_display(display.display.as_ptr().cast())
        };
        let connection = Connection::from_backend(backend);

        // The globals, read on a queue of our own: the events of `winit`
        // stay on its queue
        let (globals, queue) = registry_queue_init::<State>(&connection)?;

        let activation: XdgActivationV1 =
            globals.bind(&queue.handle(), 1..=1, ())?;

        // SAFETY: the pointer is the `wl_surface` of the window, alive
        // for as long as the window is, and of the same connection.
        let surface = unsafe {
            ObjectId::from_ptr(
                WlSurface::interface(),
                surface.surface.as_ptr().cast(),
            )
        }?;
        let surface = WlSurface::from_id(&connection, surface)?;

        activation.activate(token.to_owned(), &surface);
        activation.destroy();
        connection.flush()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_takes_a_token_and_the_others_ask_for_the_focus() {
        assert_eq!(
            Way::of(true, Some("token-of-the-second-start".to_owned())),
            Way::Activate("token-of-the-second-start".to_owned())
        );

        // No second start (the tray icon was clicked), or one that was
        // given no token: all a program may do is ask for attention
        assert_eq!(Way::of(true, None), Way::Attention);
        assert_eq!(Way::of(true, Some(String::new())), Way::Attention);

        // X11 (where the token is a startup notification, not a right),
        // Windows, macOS
        assert_eq!(Way::of(false, None), Way::Focus);
        assert_eq!(Way::of(false, Some("id".to_owned())), Way::Focus);
    }
}
