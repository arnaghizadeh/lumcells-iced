//! A windowing shell for Iced, on top of [`winit`].
//!
//! ![The native path of the Iced ecosystem](https://github.com/iced-rs/iced/blob/0525d76ff94e828b7b21634fa94a747022001c83/docs/graphs/native.png?raw=true)
//!
//! `iced_winit` offers some convenient abstractions on top of [`iced_runtime`]
//! to quickstart development when using [`winit`].
//!
//! It exposes a renderer-agnostic [`Program`] trait that can be implemented
//! and then run with a simple call. The use of this trait is optional.
//!
//! Additionally, a [`conversion`] module is available for users that decide to
//! implement a custom event loop.
//!
//! [`iced_runtime`]: https://github.com/iced-rs/iced/tree/0.14/runtime
//! [`winit`]: https://github.com/rust-windowing/winit
//! [`conversion`]: crate::conversion
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/iced-rs/iced/9ab6923e943f784985e9ef9ca28b10278297225d/docs/logo.svg"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]
pub use iced_debug as debug;
pub use iced_program as program;
pub use program::core;
pub use program::graphics;
pub use program::runtime;
pub use runtime::futures;
pub use winit;

#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
pub mod accessibility;
#[cfg(target_os = "android")]
pub mod android;
pub mod clipboard;
pub mod conversion;
pub mod look;
pub mod present;
pub mod remember;
pub mod single_instance;
pub mod tray;

mod error;
#[cfg(target_os = "macos")]
mod platform;
mod proxy;
mod window;

pub use clipboard::Clipboard;
pub use error::Error;
pub use proxy::Proxy;

use crate::core::mouse;
use crate::core::renderer;
use crate::core::theme;
use crate::core::time::Instant;
use crate::core::widget::operation;
use crate::core::{Point, Size};
use crate::futures::futures::channel::mpsc;
use crate::futures::futures::channel::oneshot;
use crate::futures::futures::task;
use crate::futures::futures::{Future, StreamExt};
use crate::futures::subscription;
use crate::futures::{Executor, Runtime};
use crate::graphics::{Compositor, Shell, compositor};
use crate::runtime::image;
use crate::runtime::system;
use crate::runtime::user_interface::{self, UserInterface};
use crate::runtime::{Action, Task};

use program::Program;
use window::WindowManager;

use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::mem::ManuallyDrop;
use std::slice;
use std::sync::Arc;

/// Runs a [`Program`] with the provided settings.
pub fn run<P>(program: P) -> Result<(), Error>
where
    P: Program + 'static,
    P::Theme: theme::Base,
{
    use winit::event_loop::EventLoop;

    let boot_span = debug::boot();
    let settings = program.settings();
    let shell_settings = program.shell();
    let window_settings = program.window();

    // The windows that are remembered keep their files under this name
    remember::name_program([
        settings.id.as_deref(),
        shell_settings.single_instance.as_deref(),
    ]);

    // A second start hands its arguments to the first and ends here,
    // before anything of a window system is touched
    #[cfg(feature = "single-instance")]
    if let Some(id) = &shell_settings.single_instance
        && single_instance::start(id) == single_instance::Start::Forwarded
    {
        return Ok(());
    }

    let event_loop = {
        #[allow(unused_mut)]
        let mut builder = EventLoop::with_user_event();

        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::EventLoopBuilderExtMacOS;

            if let Some(policy) = shell_settings.activation_policy {
                let _ = builder.with_activation_policy(
                    platform::activation_policy(policy),
                );
            }

            // A program that starts without a window does not take the
            // keyboard from what the person is typing in
            if !shell_settings.activate_at_start {
                let _ = builder.with_activate_ignoring_other_apps(false);
            }
        }

        // Android gives the program its activity in `android_main`; the
        // event loop is built on it
        #[cfg(target_os = "android")]
        {
            use winit::platform::android::EventLoopBuilderExtAndroid;

            let _ = builder.with_android_app(android::app().expect(
                "iced::android::set_app is called in android_main, before run",
            ));
        }

        builder.build().expect("Create event loop")
    };

    let graphics_settings = settings.clone().into();
    let display_handle = event_loop.owned_display_handle();

    let (proxy, worker) = Proxy::new(event_loop.create_proxy());

    #[cfg(feature = "debug")]
    {
        let proxy = proxy.clone();

        debug::on_hotpatch(move || {
            proxy.send_action(Action::Reload);
        });
    }

    let mut runtime = {
        let executor =
            P::Executor::new().map_err(Error::ExecutorCreationFailed)?;
        executor.spawn(worker);

        Runtime::new(executor, proxy.clone())
    };

    let (program, task) = runtime.enter(|| program::Instance::new(program));
    let is_daemon = window_settings.is_none();

    let task = if let Some(window_settings) = window_settings {
        let mut task = Some(task);

        let (_id, open) = runtime::window::open(window_settings);

        open.then(move |_| task.take().unwrap_or_else(Task::none))
    } else {
        task
    };

    if let Some(stream) = runtime::task::into_stream(task) {
        runtime.run(stream);
    }

    runtime.track(subscription::into_recipes(
        runtime.enter(|| program.subscription().map(Action::Output)),
    ));

    let (event_sender, event_receiver) = mpsc::unbounded();
    let (control_sender, control_receiver) = mpsc::unbounded();
    let (system_theme_sender, system_theme_receiver) = oneshot::channel();

    // What only the platform tells, asked for once the event loop runs
    let on_start: Box<dyn FnOnce()> = {
        let watch_system_look = shell_settings.watch_system_look;
        let platform_keys = shell_settings.platform_keys;
        let proxy = proxy.clone();

        Box::new(move || {
            if watch_system_look {
                let proxy = proxy.clone();

                look::watch(move |mode| {
                    proxy.send_action(Action::System(
                        runtime::system::Action::NotifyTheme(mode),
                    ));
                });
            }

            #[cfg(target_os = "macos")]
            if platform_keys {
                let proxy = proxy.clone();

                let _ = platform::take_over_quit(move || {
                    proxy.send_action(Action::System(
                        runtime::system::Action::RequestQuit,
                    ));
                });
            }

            #[cfg(not(target_os = "macos"))]
            let _ = platform_keys;
        })
    };

    let instance = Box::pin(run_instance::<P>(
        program,
        runtime,
        proxy.clone(),
        event_receiver,
        control_sender,
        display_handle,
        is_daemon,
        shell_settings,
        graphics_settings,
        settings.fonts,
        system_theme_receiver,
    ));

    let context = task::Context::from_waker(task::noop_waker_ref());

    struct Runner<Message: 'static, F> {
        instance: std::pin::Pin<Box<F>>,
        context: task::Context<'static>,
        id: Option<String>,
        sender: mpsc::UnboundedSender<Event<Action<Message>>>,
        receiver: mpsc::UnboundedReceiver<Control>,
        error: Option<Error>,
        system_theme: Option<oneshot::Sender<theme::Mode>>,

        /// What is done once, when the event loop runs.
        on_start: Option<Box<dyn FnOnce()>>,

        /// Android: what happened before the activity had a surface to
        /// draw on. A window is made, and its graphics with it, once the
        /// system says `resumed` for the first time.
        #[cfg(target_os = "android")]
        held: Option<Vec<Event<Action<Message>>>>,

        /// Makes the event loop turn and redraw; given to the
        /// accessibility of each window.
        #[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
        wake: Arc<dyn Fn() + Send + Sync>,

        #[cfg(target_arch = "wasm32")]
        canvas: Option<web_sys::HtmlCanvasElement>,
    }

    let runner = Runner {
        instance,
        context,
        id: settings.id,
        sender: event_sender,
        receiver: control_receiver,
        error: None,
        system_theme: Some(system_theme_sender),
        on_start: Some(on_start),
        #[cfg(target_os = "android")]
        held: Some(Vec::new()),

        #[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
        wake: {
            let proxy = proxy.clone();

            Arc::new(move || {
                proxy.send_action(Action::Window(
                    runtime::window::Action::RedrawAll,
                ));
            })
        },

        #[cfg(target_arch = "wasm32")]
        canvas: None,
    };

    boot_span.finish();

    impl<Message, F> winit::application::ApplicationHandler<Action<Message>>
        for Runner<Message, F>
    where
        F: Future<Output = ()>,
    {
        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Some(sender) = self.system_theme.take() {
                let _ = sender.send(
                    event_loop
                        .system_theme()
                        .map(conversion::theme_mode)
                        .unwrap_or_default(),
                );
            }

            if let Some(on_start) = self.on_start.take() {
                on_start();
            }

            // The activity has its surface: what waited for it happens now
            #[cfg(target_os = "android")]
            if let Some(held) = self.held.take() {
                for event in held {
                    self.process_event(event_loop, event);
                }
            } else {
                // The program is on the screen again, with a new surface
                self.process_event(
                    event_loop,
                    Event::EventLoopAwakened(winit::event::Event::Resumed),
                );
            }
        }

        // Android takes the surface when the program leaves the screen.
        // What is drawn on it is let go before this returns
        #[cfg(target_os = "android")]
        fn suspended(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
        ) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::Suspended),
            );
        }

        fn new_events(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            cause: winit::event::StartCause,
        ) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::NewEvents(cause)),
            );
        }

        fn window_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            window_id: winit::window::WindowId,
            event: winit::event::WindowEvent,
        ) {
            #[cfg(target_os = "windows")]
            let is_move_or_resize = matches!(
                event,
                winit::event::WindowEvent::Resized(_)
                    | winit::event::WindowEvent::Moved(_)
            );

            // A window that is remembered: where it is now
            remember::window_event(window_id, &event);

            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::WindowEvent {
                    window_id,
                    event,
                }),
            );

            // TODO: Remove when unnecessary
            // On Windows, we emulate an `AboutToWait` event after every `Resized` event
            // since the event loop does not resume during resize interaction.
            // More details: https://github.com/rust-windowing/winit/issues/3272
            #[cfg(target_os = "windows")]
            {
                if is_move_or_resize {
                    self.process_event(
                        event_loop,
                        Event::EventLoopAwakened(
                            winit::event::Event::AboutToWait,
                        ),
                    );
                }
            }
        }

        fn user_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            action: Action<Message>,
        ) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::UserEvent(
                    action,
                )),
            );
        }

        fn about_to_wait(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
        ) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::AboutToWait),
            );
        }
    }

    impl<Message, F> Runner<Message, F>
    where
        F: Future<Output = ()>,
    {
        fn process_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            event: Event<Action<Message>>,
        ) {
            if event_loop.exiting() {
                return;
            }

            #[cfg(target_os = "android")]
            if let Some(held) = &mut self.held {
                held.push(event);
                return;
            }

            self.sender.start_send(event).expect("Send event");

            loop {
                let poll = self.instance.as_mut().poll(&mut self.context);

                match poll {
                    task::Poll::Pending => match self.receiver.try_next() {
                        Ok(Some(control)) => match control {
                            Control::ChangeFlow(flow) => {
                                use winit::event_loop::ControlFlow;

                                match (event_loop.control_flow(), flow) {
                                    (
                                        ControlFlow::WaitUntil(current),
                                        ControlFlow::WaitUntil(new),
                                    ) if current > Instant::now()
                                        && current < new => {}
                                    (
                                        ControlFlow::WaitUntil(current),
                                        ControlFlow::Wait,
                                    ) if current > Instant::now() => {}
                                    _ => {
                                        event_loop.set_control_flow(flow);
                                    }
                                }
                            }
                            Control::CreateWindow {
                                id,
                                settings,
                                title,
                                scale_factor,
                                monitor,
                                on_open,
                            } => {
                                let exit_on_close_request =
                                    settings.exit_on_close_request;

                                let visible = settings.visible;

                                #[cfg(target_arch = "wasm32")]
                                let target =
                                    settings.platform_specific.target.clone();

                                let monitor =
                                    monitor.or(event_loop.primary_monitor());

                                // A window that is remembered is opened
                                // where and as large as it was left
                                let mut settings = settings;
                                let remembered = remember::open(
                                    &mut settings,
                                    scale_factor,
                                    &monitor
                                        .clone()
                                        .into_iter()
                                        .chain(event_loop.available_monitors())
                                        .map(|monitor| remember::Monitor {
                                            position: monitor.position().into(),
                                            size: monitor.size().into(),
                                            scale_factor: monitor
                                                .scale_factor(),
                                        })
                                        .collect::<Vec<_>>(),
                                );

                                let window_attributes =
                                    conversion::window_attributes(
                                        settings,
                                        &title,
                                        scale_factor,
                                        monitor,
                                        self.id.clone(),
                                    )
                                    .with_visible(false);

                                let window_attributes = match remembered
                                    .as_ref()
                                    .and_then(|remembered| remembered.position)
                                {
                                    Some((x, y)) => window_attributes
                                        .with_position(
                                            winit::dpi::PhysicalPosition::new(
                                                x, y,
                                            ),
                                        ),
                                    None => window_attributes,
                                };

                                // The activation token of the start that
                                // asked for this window lets it come to the
                                // front (Wayland), or ends the "starting"
                                // feedback of the desktop (X11)
                                #[cfg(all(
                                    unix,
                                    not(any(
                                        target_os = "macos",
                                        target_os = "ios",
                                        target_os = "android"
                                    ))
                                ))]
                                let window_attributes = {
                                    use winit::platform::startup_notify::WindowAttributesExtStartupNotify;

                                    match single_instance::take_activation_token() {
                                        Some(token) => window_attributes
                                            .with_activation_token(
                                                winit::window::ActivationToken::from_raw(token),
                                            ),
                                        None => window_attributes,
                                    }
                                };

                                #[cfg(target_arch = "wasm32")]
                                let window_attributes = {
                                    use winit::platform::web::WindowAttributesExtWebSys;
                                    window_attributes
                                        .with_canvas(self.canvas.take())
                                };

                                log::info!(
                                    "Window attributes for id `{id:#?}`: {window_attributes:#?}"
                                );

                                // On macOS, the `position` in `WindowAttributes` represents the "inner"
                                // position of the window; while on other platforms it's the "outer" position.
                                // We fix the inconsistency on macOS by positioning the window after creation.
                                #[cfg(target_os = "macos")]
                                let mut window_attributes = window_attributes;

                                #[cfg(target_os = "macos")]
                                let position =
                                    window_attributes.position.take();

                                let window = event_loop
                                    .create_window(window_attributes)
                                    .expect("Create window");

                                // Before the window is shown
                                #[cfg(all(
                                    feature = "accessibility",
                                    not(target_arch = "wasm32")
                                ))]
                                let accessibility = {
                                    let wake = self.wake.clone();

                                    accessibility::Adapter::new(
                                        event_loop,
                                        &window,
                                        move || wake(),
                                    )
                                };

                                #[cfg(target_os = "macos")]
                                if let Some(position) = position {
                                    window.set_outer_position(position);
                                }

                                #[cfg(target_arch = "wasm32")]
                                {
                                    use winit::platform::web::WindowExtWebSys;

                                    let canvas = window
                                        .canvas()
                                        .expect("Get window canvas");

                                    let _ = canvas.set_attribute(
                                        "style",
                                        "display: block; width: 100%; height: 100%",
                                    );

                                    let window = web_sys::window().unwrap();
                                    let document = window.document().unwrap();
                                    let body = document.body().unwrap();

                                    let target = target.and_then(|target| {
                                        body.query_selector(&format!(
                                            "#{target}"
                                        ))
                                        .ok()
                                        .unwrap_or(None)
                                    });

                                    match target {
                                        Some(node) => {
                                            let _ = node
                                                .replace_with_with_node_1(
                                                    &canvas,
                                                )
                                                .expect(&format!(
                                                    "Could not replace #{}",
                                                    node.id()
                                                ));
                                        }
                                        None => {
                                            let _ = body
                                                .append_child(&canvas)
                                                .expect(
                                                "Append canvas to HTML body",
                                            );
                                        }
                                    };
                                }

                                let window = Arc::new(window);

                                if let Some(remembered) = remembered {
                                    remember::track(&window, remembered);
                                }

                                self.process_event(
                                    event_loop,
                                    Event::WindowCreated {
                                        id,
                                        #[cfg(all(
                                            feature = "accessibility",
                                            not(target_arch = "wasm32")
                                        ))]
                                        accessibility,
                                        window,
                                        exit_on_close_request,
                                        make_visible: visible,
                                        on_open,
                                    },
                                );
                            }
                            Control::Exit => {
                                remember::flush_all();

                                self.process_event(event_loop, Event::Exit);
                                event_loop.exit();
                                break;
                            }
                            Control::Crash(error) => {
                                self.error = Some(error);
                                event_loop.exit();
                            }
                            Control::SetAutomaticWindowTabbing(_enabled) => {
                                #[cfg(target_os = "macos")]
                                {
                                    use winit::platform::macos::ActiveEventLoopExtMacOS;
                                    event_loop
                                        .set_allows_automatic_window_tabbing(
                                            _enabled,
                                        );
                                }
                            }
                        },
                        _ => {
                            break;
                        }
                    },
                    task::Poll::Ready(_) => {
                        event_loop.exit();
                        break;
                    }
                };
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut runner = runner;
        let _ = event_loop.run_app(&mut runner);

        runner.error.map(Err).unwrap_or(Ok(()))
    }

    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        let _ = event_loop.spawn_app(runner);

        Ok(())
    }
}

#[derive(Debug)]
enum Event<Message: 'static> {
    WindowCreated {
        id: window::Id,
        #[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
        accessibility: accessibility::Adapter,
        window: Arc<winit::window::Window>,
        exit_on_close_request: bool,
        make_visible: bool,
        on_open: oneshot::Sender<window::Id>,
    },
    EventLoopAwakened(winit::event::Event<Message>),
    Exit,
}

#[derive(Debug)]
enum Control {
    ChangeFlow(winit::event_loop::ControlFlow),
    Exit,
    Crash(Error),
    CreateWindow {
        id: window::Id,
        settings: window::Settings,
        title: String,
        monitor: Option<winit::monitor::MonitorHandle>,
        on_open: oneshot::Sender<window::Id>,
        scale_factor: f32,
    },
    SetAutomaticWindowTabbing(bool),
}

async fn run_instance<P>(
    mut program: program::Instance<P>,
    mut runtime: Runtime<P::Executor, Proxy<P::Message>, Action<P::Message>>,
    mut proxy: Proxy<P::Message>,
    mut event_receiver: mpsc::UnboundedReceiver<Event<Action<P::Message>>>,
    mut control_sender: mpsc::UnboundedSender<Control>,
    display_handle: winit::event_loop::OwnedDisplayHandle,
    is_daemon: bool,
    shell_settings: runtime::shell::Settings,
    graphics_settings: graphics::Settings,
    default_fonts: Vec<Cow<'static, [u8]>>,
    mut _system_theme: oneshot::Receiver<theme::Mode>,
) where
    P: Program + 'static,
    P::Theme: theme::Base,
{
    use winit::event;
    use winit::event_loop::ControlFlow;

    let mut window_manager = WindowManager::new();
    let mut is_window_opening = !is_daemon;

    let mut compositor = None;
    let mut events = Vec::new();
    let mut messages = Vec::new();
    let mut actions = 0;

    let mut ui_caches = FxHashMap::default();
    let mut user_interfaces = ManuallyDrop::new(FxHashMap::default());
    let mut clipboard = Clipboard::unconnected();

    #[cfg(all(feature = "linux-theme-detection", target_os = "linux"))]
    let mut system_theme = {
        let to_mode = |color_scheme| match color_scheme {
            mundy::ColorScheme::NoPreference => theme::Mode::None,
            mundy::ColorScheme::Light => theme::Mode::Light,
            mundy::ColorScheme::Dark => theme::Mode::Dark,
        };

        runtime.run(
            mundy::Preferences::stream(mundy::Interest::ColorScheme)
                .map(move |preferences| {
                    Action::System(system::Action::NotifyTheme(to_mode(
                        preferences.color_scheme,
                    )))
                })
                .boxed(),
        );

        runtime
            .enter(|| {
                mundy::Preferences::once_blocking(
                    mundy::Interest::ColorScheme,
                    core::time::Duration::from_millis(200),
                )
            })
            .map(|preferences| to_mode(preferences.color_scheme))
            .unwrap_or_default()
    };

    #[cfg(not(all(feature = "linux-theme-detection", target_os = "linux")))]
    let mut system_theme =
        _system_theme.try_recv().ok().flatten().unwrap_or_default();

    log::info!("System theme: {system_theme:?}");

    // The tray icon the program starts with. This runs at the first poll of
    // the instance: inside the event loop, on its thread.
    let mut tray = tray::Tray::default();
    let mut shell_settings = shell_settings;

    if let Some(settings) = shell_settings.tray.take() {
        tray.perform(runtime::tray::Action::Update(settings), |stream| {
            spawn(&mut runtime, stream);
        });
    }

    // macOS: in the Dock only while there is a window
    #[cfg(target_os = "macos")]
    let mut has_windows = false;

    // Android: what the shell hands over of the keyboard makes the loop turn
    #[cfg(target_os = "android")]
    android::set_wake({
        let proxy = proxy.clone();

        Box::new(move || {
            proxy.send_action(Action::Window(
                runtime::window::Action::RedrawAll,
            ));
        })
    });

    'next_event: loop {
        #[cfg(target_os = "macos")]
        if shell_settings.dock_follows_windows
            && has_windows == window_manager.is_empty()
        {
            has_windows = !window_manager.is_empty();

            platform::set_activation_policy(
                runtime::shell::ActivationPolicy::for_windows(usize::from(
                    has_windows,
                )),
            );
        }

        // Empty the queue if possible
        let event = if let Ok(event) = event_receiver.try_next() {
            event
        } else {
            event_receiver.next().await
        };

        let Some(event) = event else {
            break;
        };

        match event {
            Event::WindowCreated {
                id,
                #[cfg(all(
                    feature = "accessibility",
                    not(target_arch = "wasm32")
                ))]
                accessibility,
                window,
                exit_on_close_request,
                make_visible,
                on_open,
            } => {
                if compositor.is_none() {
                    let (compositor_sender, compositor_receiver) =
                        oneshot::channel();

                    let create_compositor = {
                        let window = window.clone();
                        let display_handle = display_handle.clone();
                        let proxy = proxy.clone();
                        let default_fonts = default_fonts.clone();

                        async move {
                            let shell = Shell::new(proxy.clone());

                            let mut compositor =
                                <P::Renderer as compositor::Default>::Compositor::new(
                                    graphics_settings,
                                    display_handle,
                                    window,
                                    shell,
                                ).await;

                            if let Ok(compositor) = &mut compositor {
                                for font in default_fonts {
                                    compositor.load_font(font.clone());
                                }
                            }

                            compositor_sender
                                .send(compositor)
                                .ok()
                                .expect("Send compositor");

                            // HACK! Send a proxy event on completion to trigger
                            // a runtime re-poll
                            // TODO: Send compositor through proxy (?)
                            {
                                let (sender, _receiver) = oneshot::channel();

                                proxy.send_action(Action::Window(
                                    runtime::window::Action::GetLatest(sender),
                                ));
                            }
                        }
                    };

                    #[cfg(target_arch = "wasm32")]
                    wasm_bindgen_futures::spawn_local(create_compositor);

                    #[cfg(not(target_arch = "wasm32"))]
                    runtime.block_on(create_compositor);

                    match compositor_receiver
                        .await
                        .expect("Wait for compositor")
                    {
                        Ok(new_compositor) => {
                            compositor = Some(new_compositor);
                        }
                        Err(error) => {
                            let _ = control_sender
                                .start_send(Control::Crash(error.into()));
                            continue;
                        }
                    }
                }

                let window_theme = window
                    .theme()
                    .map(conversion::theme_mode)
                    .unwrap_or_default();

                if system_theme != window_theme {
                    system_theme = window_theme;

                    runtime.broadcast(subscription::Event::SystemThemeChanged(
                        window_theme,
                    ));
                }

                let is_first = window_manager.is_empty();
                let window = window_manager.insert(
                    id,
                    window,
                    &program,
                    compositor
                        .as_mut()
                        .expect("Compositor must be initialized"),
                    exit_on_close_request,
                    system_theme,
                );

                #[cfg(all(
                    feature = "accessibility",
                    not(target_arch = "wasm32")
                ))]
                {
                    window.accessibility = Some(accessibility);
                }

                window.raw.set_theme(conversion::window_theme(
                    window.state.theme_mode(),
                ));

                debug::theme_changed(|| {
                    if is_first {
                        theme::Base::palette(window.state.theme())
                    } else {
                        None
                    }
                });

                let logical_size = window.state.logical_size();

                let _ = user_interfaces.insert(
                    id,
                    build_user_interface(
                        &program,
                        user_interface::Cache::default(),
                        &mut window.renderer,
                        logical_size,
                        id,
                    ),
                );
                let _ = ui_caches.insert(id, user_interface::Cache::default());

                if make_visible {
                    window.raw.set_visible(true);
                }

                events.push((
                    id,
                    core::Event::Window(window::Event::Opened {
                        position: window.position(),
                        size: window.logical_size(),
                    }),
                ));

                if let Some(safe_area) = window.safe_area_changed() {
                    events.push((
                        id,
                        core::Event::Window(window::Event::SafeAreaChanged(
                            safe_area,
                        )),
                    ));
                }

                if let Some(keyboard) = window.keyboard_changed() {
                    events.push((
                        id,
                        core::Event::Window(window::Event::KeyboardChanged(
                            keyboard,
                        )),
                    ));
                }

                if clipboard.window_id().is_none() {
                    clipboard = Clipboard::connect(window.raw.clone());
                }

                let _ = on_open.send(id);
                is_window_opening = false;
            }
            Event::EventLoopAwakened(event) => {
                match event {
                    event::Event::NewEvents(event::StartCause::Init) => {
                        for (_id, window) in window_manager.iter_mut() {
                            window.raw.request_redraw();
                        }
                    }
                    event::Event::NewEvents(
                        event::StartCause::ResumeTimeReached { .. },
                    ) => {
                        let now = Instant::now();

                        for (_id, window) in window_manager.iter_mut() {
                            if let Some(redraw_at) = window.redraw_at
                                && redraw_at <= now
                            {
                                window.raw.request_redraw();
                                window.redraw_at = None;
                            }
                        }

                        if let Some(redraw_at) = window_manager.redraw_at() {
                            let _ =
                                control_sender.start_send(Control::ChangeFlow(
                                    ControlFlow::WaitUntil(redraw_at),
                                ));
                        } else {
                            let _ = control_sender.start_send(
                                Control::ChangeFlow(ControlFlow::Wait),
                            );
                        }
                    }
                    event::Event::UserEvent(action) => {
                        // "Quit" of the platform: the program is told,
                        // and ends unless it wants to end by itself
                        if matches!(
                            action,
                            Action::System(system::Action::RequestQuit)
                        ) && shell_settings.exit_on_quit_request
                        {
                            control_sender
                                .start_send(Control::Exit)
                                .expect("Send control action");
                        }

                        run_action(
                            action,
                            &program,
                            &mut runtime,
                            &mut compositor,
                            &mut events,
                            &mut messages,
                            &mut clipboard,
                            &mut control_sender,
                            &mut user_interfaces,
                            &mut window_manager,
                            &mut ui_caches,
                            &mut is_window_opening,
                            &mut system_theme,
                            &mut tray,
                        );
                        actions += 1;
                    }
                    event::Event::WindowEvent {
                        window_id: id,
                        event: event::WindowEvent::RedrawRequested,
                        ..
                    } => {
                        let Some(mut current_compositor) = compositor.as_mut()
                        else {
                            continue;
                        };

                        let Some((id, mut window)) =
                            window_manager.get_mut_alias(id)
                        else {
                            continue;
                        };

                        let physical_size = window.state.physical_size();
                        let mut logical_size = window.state.logical_size();

                        if physical_size.width == 0 || physical_size.height == 0
                        {
                            continue;
                        }

                        // Android: nothing to draw on while the program
                        // is not on the screen
                        if window.surface.is_none() {
                            continue;
                        }

                        // Window was resized between redraws
                        if window.surface_version
                            != window.state.surface_version()
                        {
                            let ui = user_interfaces
                                .remove(&id)
                                .expect("Remove user interface");

                            let layout_span = debug::layout(id);
                            let _ = user_interfaces.insert(
                                id,
                                ui.relayout(logical_size, &mut window.renderer),
                            );
                            layout_span.finish();

                            if let Some(surface) = &mut window.surface {
                                current_compositor.configure_surface(
                                    surface,
                                    physical_size.width,
                                    physical_size.height,
                                );
                            }

                            window.surface_version =
                                window.state.surface_version();
                        }

                        let redraw_event = core::Event::Window(
                            window::Event::RedrawRequested(Instant::now()),
                        );

                        let cursor = window.state.cursor();

                        let mut interface = user_interfaces
                            .get_mut(&id)
                            .expect("Get user interface");

                        let interact_span = debug::interact(id);
                        let mut redraw_count = 0;

                        let state = loop {
                            let message_count = messages.len();
                            let (state, _) = interface.update(
                                slice::from_ref(&redraw_event),
                                cursor,
                                &mut window.renderer,
                                &mut clipboard,
                                &mut messages,
                            );

                            if message_count == messages.len()
                                && !state.has_layout_changed()
                            {
                                break state;
                            }

                            if redraw_count >= 2 {
                                log::warn!(
                                    "More than 3 consecutive RedrawRequested events \
                                    produced layout invalidation"
                                );

                                break state;
                            }

                            redraw_count += 1;

                            if !messages.is_empty() {
                                let caches: FxHashMap<_, _> =
                                    ManuallyDrop::into_inner(user_interfaces)
                                        .into_iter()
                                        .map(|(id, interface)| {
                                            (id, interface.into_cache())
                                        })
                                        .collect();

                                let actions = update(
                                    &mut program,
                                    &mut runtime,
                                    &mut messages,
                                );

                                user_interfaces =
                                    ManuallyDrop::new(build_user_interfaces(
                                        &program,
                                        &mut window_manager,
                                        caches,
                                    ));

                                for action in actions {
                                    // Defer all window actions to avoid compositor
                                    // race conditions while redrawing
                                    if let Action::Window(_) = action {
                                        proxy.send_action(action);
                                        continue;
                                    }

                                    run_action(
                                        action,
                                        &program,
                                        &mut runtime,
                                        &mut compositor,
                                        &mut events,
                                        &mut messages,
                                        &mut clipboard,
                                        &mut control_sender,
                                        &mut user_interfaces,
                                        &mut window_manager,
                                        &mut ui_caches,
                                        &mut is_window_opening,
                                        &mut system_theme,
                                        &mut tray,
                                    );
                                }

                                for (window_id, window) in
                                    window_manager.iter_mut()
                                {
                                    // We are already redrawing this window
                                    if window_id == id {
                                        continue;
                                    }

                                    window.raw.request_redraw();
                                }

                                let Some(next_compositor) = compositor.as_mut()
                                else {
                                    continue 'next_event;
                                };

                                current_compositor = next_compositor;
                                window = window_manager.get_mut(id).unwrap();

                                // Window scale factor changed during a redraw request
                                if logical_size != window.state.logical_size() {
                                    logical_size = window.state.logical_size();

                                    log::debug!(
                                        "Window scale factor changed during a redraw request"
                                    );

                                    let ui = user_interfaces
                                        .remove(&id)
                                        .expect("Remove user interface");

                                    let layout_span = debug::layout(id);
                                    let _ = user_interfaces.insert(
                                        id,
                                        ui.relayout(
                                            logical_size,
                                            &mut window.renderer,
                                        ),
                                    );
                                    layout_span.finish();
                                }

                                interface =
                                    user_interfaces.get_mut(&id).unwrap();
                            }
                        };
                        interact_span.finish();

                        let draw_span = debug::draw(id);
                        interface.draw(
                            &mut window.renderer,
                            window.state.theme(),
                            &renderer::Style {
                                text_color: window.state.text_color(),
                            },
                            cursor,
                        );
                        draw_span.finish();

                        // What was drawn is told to assistive technology,
                        // if any listens
                        #[cfg(all(
                            feature = "accessibility",
                            not(target_arch = "wasm32")
                        ))]
                        if let Some(accessibility) = &mut window.accessibility {
                            let title = program.title(id);
                            let bounds = core::Rectangle::with_size(
                                window.state.logical_size(),
                            );
                            let renderer = &window.renderer;

                            accessibility.update(
                                f64::from(window.state.scale_factor()),
                                || {
                                    runtime::accessibility::Tree::of(
                                        interface, renderer, &title, bounds,
                                    )
                                },
                            );
                        }

                        if let user_interface::State::Updated {
                            redraw_request,
                            input_method,
                            mouse_interaction,
                            ..
                        } = state
                        {
                            window.request_redraw(redraw_request);
                            window.request_input_method(input_method);
                            window.update_mouse(mouse_interaction);
                        }

                        runtime.broadcast(subscription::Event::Interaction {
                            window: id,
                            event: redraw_event,
                            status: core::event::Status::Ignored,
                        });

                        window.draw_preedit();

                        let present_span = debug::present(id);
                        let Some(surface) = &mut window.surface else {
                            continue;
                        };

                        match current_compositor.present(
                            &mut window.renderer,
                            surface,
                            window.state.viewport(),
                            window.state.background_color(),
                            || window.raw.pre_present_notify(),
                        ) {
                            Ok(()) => {
                                present_span.finish();
                            }
                            Err(error) => match error {
                                compositor::SurfaceError::OutOfMemory => {
                                    // This is an unrecoverable error.
                                    panic!("{error:?}");
                                }
                                compositor::SurfaceError::Outdated
                                | compositor::SurfaceError::Lost => {
                                    present_span.finish();

                                    // Reconfigure surface and try redrawing
                                    let physical_size =
                                        window.state.physical_size();

                                    if error == compositor::SurfaceError::Lost {
                                        window.surface = Some(
                                            current_compositor.create_surface(
                                                window.raw.clone(),
                                                physical_size.width,
                                                physical_size.height,
                                            ),
                                        );
                                    } else if let Some(surface) =
                                        &mut window.surface
                                    {
                                        current_compositor.configure_surface(
                                            surface,
                                            physical_size.width,
                                            physical_size.height,
                                        );
                                    }

                                    window.raw.request_redraw();
                                }
                                _ => {
                                    present_span.finish();

                                    log::error!(
                                        "Error {error:?} when \
                                        presenting surface."
                                    );

                                    // Try rendering all windows again next frame.
                                    for (_id, window) in
                                        window_manager.iter_mut()
                                    {
                                        window.raw.request_redraw();
                                    }
                                }
                            },
                        }
                    }
                    event::Event::WindowEvent {
                        event: window_event,
                        window_id,
                    } => {
                        if !is_daemon
                            && matches!(
                                window_event,
                                winit::event::WindowEvent::Destroyed
                            )
                            && !is_window_opening
                            && window_manager.is_empty()
                        {
                            control_sender
                                .start_send(Control::Exit)
                                .expect("Send control action");

                            continue;
                        }

                        let Some((id, window)) =
                            window_manager.get_mut_alias(window_id)
                        else {
                            continue;
                        };

                        #[cfg(all(
                            feature = "accessibility",
                            not(target_arch = "wasm32")
                        ))]
                        if let Some(accessibility) = &mut window.accessibility {
                            accessibility
                                .process_event(&window.raw, &window_event);
                        }

                        match window_event {
                            winit::event::WindowEvent::Resized(_) => {
                                window.raw.request_redraw();
                            }
                            winit::event::WindowEvent::ThemeChanged(theme) => {
                                let mode = conversion::theme_mode(theme);

                                if mode != system_theme {
                                    system_theme = mode;

                                    runtime.broadcast(
                                        subscription::Event::SystemThemeChanged(
                                            mode,
                                        ),
                                    );
                                }
                            }
                            _ => {}
                        }

                        if matches!(
                            window_event,
                            winit::event::WindowEvent::CloseRequested
                        ) && window.exit_on_close_request
                        {
                            run_action(
                                Action::Window(runtime::window::Action::Close(
                                    id,
                                )),
                                &program,
                                &mut runtime,
                                &mut compositor,
                                &mut events,
                                &mut messages,
                                &mut clipboard,
                                &mut control_sender,
                                &mut user_interfaces,
                                &mut window_manager,
                                &mut ui_caches,
                                &mut is_window_opening,
                                &mut system_theme,
                                &mut tray,
                            );
                        } else {
                            window.state.update(
                                &program,
                                &window.raw,
                                &window_event,
                            );

                            if let Some(event) = conversion::window_event(
                                window_event,
                                window.state.scale_factor(),
                                window.state.modifiers(),
                            ) {
                                events.push((id, event));
                            }

                            // A phone was turned: the system's bars are
                            // at other edges now
                            if let Some(safe_area) = window.safe_area_changed()
                            {
                                events.push((
                                    id,
                                    core::Event::Window(
                                        window::Event::SafeAreaChanged(
                                            safe_area,
                                        ),
                                    ),
                                ));
                                window.raw.request_redraw();
                            }
                        }
                    }
                    // Android took the surface (the program left the
                    // screen): what is drawn on it is let go now, before
                    // the system destroys it
                    #[cfg(target_os = "android")]
                    event::Event::Suspended => {
                        for (_id, window) in window_manager.iter_mut() {
                            window.surface = None;
                        }
                    }
                    // Android gave a new surface: the windows are drawn
                    // on it from now on
                    #[cfg(target_os = "android")]
                    event::Event::Resumed => {
                        let Some(compositor) = compositor.as_mut() else {
                            continue;
                        };

                        for (_id, window) in window_manager.iter_mut() {
                            let size = window.state.physical_size();

                            window.surface = None;
                            window.surface = Some(compositor.create_surface(
                                window.raw.clone(),
                                size.width,
                                size.height,
                            ));
                            window.raw.request_redraw();
                        }
                    }
                    event::Event::AboutToWait => {
                        if actions > 0 {
                            proxy.free_slots(actions);
                            actions = 0;
                        }

                        // Android: what the shell caught of the keyboard,
                        // as the input method's events; and the insets it
                        // read (the system's bars, the keyboard's height)
                        #[cfg(target_os = "android")]
                        if let Some((id, window)) =
                            window_manager.iter_mut().next()
                        {
                            use core::input_method;

                            if let Some(safe_area) = window.safe_area_changed()
                            {
                                events.push((
                                    id,
                                    core::Event::Window(
                                        window::Event::SafeAreaChanged(
                                            safe_area,
                                        ),
                                    ),
                                ));
                                window.raw.request_redraw();
                            }

                            if let Some(keyboard) = window.keyboard_changed() {
                                events.push((
                                    id,
                                    core::Event::Window(
                                        window::Event::KeyboardChanged(
                                            keyboard,
                                        ),
                                    ),
                                ));
                                window.raw.request_redraw();
                            }

                            for typed in android::typed() {
                                let event = match typed {
                                    android::TextInput::Commit(text) => {
                                        input_method::Event::Commit(text)
                                    }
                                    android::TextInput::Preedit(text) => {
                                        input_method::Event::Preedit(text, None)
                                    }
                                };

                                events.push((
                                    id,
                                    core::Event::InputMethod(event),
                                ));
                            }
                        }

                        // What assistive technology asked for
                        #[cfg(all(
                            feature = "accessibility",
                            not(target_arch = "wasm32")
                        ))]
                        for (id, window) in window_manager.iter_mut() {
                            let Some(accessibility) = &mut window.accessibility
                            else {
                                continue;
                            };

                            if accessibility.wants_tree() {
                                window.raw.request_redraw();
                            }

                            let requests = accessibility.requests();

                            let Some(ui) = user_interfaces.get_mut(&id) else {
                                continue;
                            };

                            for request in requests {
                                accessibility::perform(
                                    request,
                                    ui,
                                    &mut window.renderer,
                                    &mut clipboard,
                                    &mut messages,
                                );

                                window.raw.request_redraw();
                            }
                        }

                        if events.is_empty()
                            && messages.is_empty()
                            && window_manager.is_idle()
                        {
                            continue;
                        }

                        let mut uis_stale = false;
                        let mut platform_keys = Vec::new();

                        for (id, window) in window_manager.iter_mut() {
                            let interact_span = debug::interact(id);
                            let mut window_events = vec![];

                            events.retain(|(window_id, event)| {
                                if *window_id == id {
                                    window_events.push(event.clone());
                                    false
                                } else {
                                    true
                                }
                            });

                            if window_events.is_empty() && messages.is_empty() {
                                continue;
                            }

                            let (ui_state, statuses) = user_interfaces
                                .get_mut(&id)
                                .expect("Get user interface")
                                .update(
                                    &window_events,
                                    window.state.cursor(),
                                    &mut window.renderer,
                                    &mut clipboard,
                                    &mut messages,
                                );

                            #[cfg(feature = "unconditional-rendering")]
                            window.request_redraw(
                                window::RedrawRequest::NextFrame,
                            );

                            match ui_state {
                                user_interface::State::Updated {
                                    redraw_request: _redraw_request,
                                    mouse_interaction,
                                    ..
                                } => {
                                    window.update_mouse(mouse_interaction);

                                    #[cfg(not(
                                        feature = "unconditional-rendering"
                                    ))]
                                    window.request_redraw(_redraw_request);
                                }
                                user_interface::State::Outdated => {
                                    uis_stale = true;
                                }
                            }

                            for (event, mut status) in window_events
                                .into_iter()
                                .zip(statuses.into_iter())
                            {
                                // Tab and Shift+Tab move the keyboard focus
                                // when no widget wanted the key
                                if shell_settings.tab_focus
                                    && status == core::event::Status::Ignored
                                    && let Some(direction) =
                                        runtime::focus::direction(&event)
                                    && let Some(ui) =
                                        user_interfaces.get_mut(&id)
                                    && runtime::focus::shift(
                                        ui,
                                        &window.renderer,
                                        direction,
                                    )
                                {
                                    status = core::event::Status::Captured;
                                    window.raw.request_redraw();
                                }

                                // The keys of the platform for windows
                                // and the program, when no widget wanted
                                // the key (macOS: Command-W, Command-Q)
                                if shell_settings.platform_keys
                                    && status == core::event::Status::Ignored
                                    && let Some(key) =
                                        runtime::shell::PlatformKey::of(
                                            cfg!(target_os = "macos"),
                                            &event,
                                        )
                                {
                                    status = core::event::Status::Captured;
                                    platform_keys.push((id, key));
                                }

                                runtime.broadcast(
                                    subscription::Event::Interaction {
                                        window: id,
                                        event,
                                        status,
                                    },
                                );
                            }

                            interact_span.finish();
                        }

                        for (id, event) in events.drain(..) {
                            runtime.broadcast(
                                subscription::Event::Interaction {
                                    window: id,
                                    event,
                                    status: core::event::Status::Ignored,
                                },
                            );
                        }

                        for (id, key) in platform_keys {
                            use runtime::shell::PlatformKey;

                            match key {
                                // As the button of the window that closes
                                // it: the window closes, or the program
                                // is asked
                                PlatformKey::CloseWindow => {
                                    let closes = window_manager
                                        .get_mut(id)
                                        .is_some_and(|window| {
                                            window.exit_on_close_request
                                        });

                                    if closes {
                                        run_action(
                                            Action::Window(
                                                runtime::window::Action::Close(
                                                    id,
                                                ),
                                            ),
                                            &program,
                                            &mut runtime,
                                            &mut compositor,
                                            &mut events,
                                            &mut messages,
                                            &mut clipboard,
                                            &mut control_sender,
                                            &mut user_interfaces,
                                            &mut window_manager,
                                            &mut ui_caches,
                                            &mut is_window_opening,
                                            &mut system_theme,
                                            &mut tray,
                                        );
                                    } else {
                                        runtime.broadcast(
                                            subscription::Event::Interaction {
                                                window: id,
                                                event: core::Event::Window(
                                                    core::window::Event::CloseRequested,
                                                ),
                                                status:
                                                    core::event::Status::Ignored,
                                            },
                                        );
                                    }
                                }
                                // The menu of the application has the
                                // key; it arrives here only when there is
                                // no such menu
                                PlatformKey::Quit => {
                                    proxy.send_action(Action::System(
                                        system::Action::RequestQuit,
                                    ));
                                }
                            }
                        }

                        if !messages.is_empty() || uis_stale {
                            let cached_interfaces: FxHashMap<_, _> =
                                ManuallyDrop::into_inner(user_interfaces)
                                    .into_iter()
                                    .map(|(id, ui)| (id, ui.into_cache()))
                                    .collect();

                            let actions = update(
                                &mut program,
                                &mut runtime,
                                &mut messages,
                            );

                            user_interfaces =
                                ManuallyDrop::new(build_user_interfaces(
                                    &program,
                                    &mut window_manager,
                                    cached_interfaces,
                                ));

                            for action in actions {
                                run_action(
                                    action,
                                    &program,
                                    &mut runtime,
                                    &mut compositor,
                                    &mut events,
                                    &mut messages,
                                    &mut clipboard,
                                    &mut control_sender,
                                    &mut user_interfaces,
                                    &mut window_manager,
                                    &mut ui_caches,
                                    &mut is_window_opening,
                                    &mut system_theme,
                                    &mut tray,
                                );
                            }

                            for (_id, window) in window_manager.iter_mut() {
                                window.raw.request_redraw();
                            }
                        }

                        if let Some(redraw_at) = window_manager.redraw_at() {
                            let _ =
                                control_sender.start_send(Control::ChangeFlow(
                                    ControlFlow::WaitUntil(redraw_at),
                                ));
                        } else {
                            let _ = control_sender.start_send(
                                Control::ChangeFlow(ControlFlow::Wait),
                            );
                        }
                    }
                    _ => {}
                }
            }
            Event::Exit => break,
        }
    }

    let _ = ManuallyDrop::into_inner(user_interfaces);
}

/// Runs a stream that yields nothing on the executor of the [`Runtime`],
/// until it ends.
fn spawn<E, Message>(
    runtime: &mut Runtime<E, Proxy<Message>, Action<Message>>,
    stream: futures::futures::stream::BoxStream<'static, ()>,
) where
    E: Executor,
    Message: Send + 'static,
{
    use futures::futures::StreamExt;

    runtime.run(
        stream
            .filter_map(|()| futures::futures::future::ready(None))
            .boxed(),
    );
}

/// Builds a window's [`UserInterface`] for the [`Program`].
fn build_user_interface<'a, P: Program>(
    program: &'a program::Instance<P>,
    cache: user_interface::Cache,
    renderer: &mut P::Renderer,
    size: Size,
    id: window::Id,
) -> UserInterface<'a, P::Message, P::Theme, P::Renderer>
where
    P::Theme: theme::Base,
{
    let view_span = debug::view(id);
    let view = program.view(id);
    view_span.finish();

    let layout_span = debug::layout(id);
    let user_interface = UserInterface::build(view, size, cache, renderer);
    layout_span.finish();

    user_interface
}

fn update<P: Program, E: Executor>(
    program: &mut program::Instance<P>,
    runtime: &mut Runtime<E, Proxy<P::Message>, Action<P::Message>>,
    messages: &mut Vec<P::Message>,
) -> Vec<Action<P::Message>>
where
    P::Theme: theme::Base,
{
    use futures::futures;

    let mut actions = Vec::new();

    for message in messages.drain(..) {
        let task = runtime.enter(|| program.update(message));

        if let Some(mut stream) = runtime::task::into_stream(task) {
            let waker = futures::task::noop_waker_ref();
            let mut context = futures::task::Context::from_waker(waker);

            // Run immediately available actions synchronously (e.g. widget operations)
            loop {
                match runtime.enter(|| stream.poll_next_unpin(&mut context)) {
                    futures::task::Poll::Ready(Some(action)) => {
                        actions.push(action);
                    }
                    futures::task::Poll::Ready(None) => {
                        break;
                    }
                    futures::task::Poll::Pending => {
                        runtime.run(stream);
                        break;
                    }
                }
            }
        }
    }

    let subscription = runtime.enter(|| program.subscription());
    let recipes = subscription::into_recipes(subscription.map(Action::Output));

    runtime.track(recipes);

    actions
}

fn run_action<'a, P, C>(
    action: Action<P::Message>,
    program: &'a program::Instance<P>,
    runtime: &mut Runtime<P::Executor, Proxy<P::Message>, Action<P::Message>>,
    compositor: &mut Option<C>,
    events: &mut Vec<(window::Id, core::Event)>,
    messages: &mut Vec<P::Message>,
    clipboard: &mut Clipboard,
    control_sender: &mut mpsc::UnboundedSender<Control>,
    interfaces: &mut FxHashMap<
        window::Id,
        UserInterface<'a, P::Message, P::Theme, P::Renderer>,
    >,
    window_manager: &mut WindowManager<P, C>,
    ui_caches: &mut FxHashMap<window::Id, user_interface::Cache>,
    is_window_opening: &mut bool,
    system_theme: &mut theme::Mode,
    tray: &mut tray::Tray,
) where
    P: Program,
    C: Compositor<Renderer = P::Renderer> + 'static,
    P::Theme: theme::Base,
{
    use crate::runtime::clipboard;
    use crate::runtime::window;

    match action {
        Action::Output(message) => {
            messages.push(message);
        }
        Action::Clipboard(action) => match action {
            clipboard::Action::Read { target, channel } => {
                let _ = channel.send(clipboard.read(target));
            }
            clipboard::Action::Write { target, contents } => {
                clipboard.write(target, contents);
            }
        },
        Action::Window(action) => match action {
            window::Action::Open(id, settings, channel) => {
                let monitor = window_manager.last_monitor();

                control_sender
                    .start_send(Control::CreateWindow {
                        id,
                        settings,
                        title: program.title(id),
                        scale_factor: program.scale_factor(id),
                        monitor,
                        on_open: channel,
                    })
                    .expect("Send control action");

                *is_window_opening = true;
            }
            window::Action::Close(id) => {
                let _ = ui_caches.remove(&id);
                let _ = interfaces.remove(&id);

                if let Some(window) = window_manager.remove(id) {
                    remember::close(&window.raw);

                    if clipboard.window_id() == Some(window.raw.id()) {
                        *clipboard = window_manager
                            .first()
                            .map(|window| window.raw.clone())
                            .map(Clipboard::connect)
                            .unwrap_or_else(Clipboard::unconnected);
                    }

                    events.push((
                        id,
                        core::Event::Window(core::window::Event::Closed),
                    ));
                }

                if window_manager.is_empty() {
                    *compositor = None;
                }
            }
            window::Action::GetOldest(channel) => {
                let id =
                    window_manager.iter_mut().next().map(|(id, _window)| id);

                let _ = channel.send(id);
            }
            window::Action::GetLatest(channel) => {
                let id =
                    window_manager.iter_mut().last().map(|(id, _window)| id);

                let _ = channel.send(id);
            }
            window::Action::Drag(id) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = window.raw.drag_window();
                }
            }
            window::Action::DragResize(id, direction) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = window.raw.drag_resize_window(
                        conversion::resize_direction(direction),
                    );
                }
            }
            window::Action::Resize(id, size) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = window.raw.request_inner_size(
                        winit::dpi::LogicalSize {
                            width: size.width,
                            height: size.height,
                        }
                        .to_physical::<f32>(f64::from(
                            window.state.scale_factor(),
                        )),
                    );
                }
            }
            window::Action::SetMinSize(id, size) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_min_inner_size(size.map(|size| {
                        winit::dpi::LogicalSize {
                            width: size.width,
                            height: size.height,
                        }
                        .to_physical::<f32>(f64::from(
                            window.state.scale_factor(),
                        ))
                    }));
                }
            }
            window::Action::SetMaxSize(id, size) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_max_inner_size(size.map(|size| {
                        winit::dpi::LogicalSize {
                            width: size.width,
                            height: size.height,
                        }
                        .to_physical::<f32>(f64::from(
                            window.state.scale_factor(),
                        ))
                    }));
                }
            }
            window::Action::SetResizeIncrements(id, increments) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_resize_increments(increments.map(|size| {
                        winit::dpi::LogicalSize {
                            width: size.width,
                            height: size.height,
                        }
                        .to_physical::<f32>(f64::from(
                            window.state.scale_factor(),
                        ))
                    }));
                }
            }
            window::Action::SetResizable(id, resizable) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_resizable(resizable);
                }
            }
            window::Action::GetSize(id, channel) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let size = window.logical_size();
                    let _ = channel.send(Size::new(size.width, size.height));
                }
            }
            window::Action::GetMaximized(id, channel) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = channel.send(window.raw.is_maximized());
                }
            }
            window::Action::Maximize(id, maximized) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_maximized(maximized);
                }
            }
            window::Action::GetMinimized(id, channel) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = channel.send(window.raw.is_minimized());
                }
            }
            window::Action::Minimize(id, minimized) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_minimized(minimized);
                }
            }
            window::Action::GetPosition(id, channel) => {
                if let Some(window) = window_manager.get(id) {
                    let position = window
                        .raw
                        .outer_position()
                        .map(|position| {
                            let position = position
                                .to_logical::<f32>(window.raw.scale_factor());

                            Point::new(position.x, position.y)
                        })
                        .ok();

                    let _ = channel.send(position);
                }
            }
            window::Action::GetScaleFactor(id, channel) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let scale_factor = window.raw.scale_factor();

                    let _ = channel.send(scale_factor as f32);
                }
            }
            window::Action::Move(id, position) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_outer_position(
                        winit::dpi::LogicalPosition {
                            x: position.x,
                            y: position.y,
                        },
                    );
                }
            }
            window::Action::SetMode(id, mode) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_visible(conversion::visible(mode));
                    window.raw.set_fullscreen(conversion::fullscreen(
                        window.raw.current_monitor(),
                        mode,
                    ));
                }
            }
            window::Action::SetIcon(id, icon) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_window_icon(conversion::icon(icon));
                }
            }
            window::Action::GetMode(id, channel) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let mode = if window.raw.is_visible().unwrap_or(true) {
                        conversion::mode(window.raw.fullscreen())
                    } else {
                        core::window::Mode::Hidden
                    };

                    let _ = channel.send(mode);
                }
            }
            window::Action::ToggleMaximize(id) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_maximized(!window.raw.is_maximized());
                }
            }
            window::Action::ToggleDecorations(id) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.set_decorations(!window.raw.is_decorated());
                }
            }
            window::Action::RequestUserAttention(id, attention_type) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.request_user_attention(
                        attention_type.map(conversion::user_attention),
                    );
                }
            }
            window::Action::GainFocus(id) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window.raw.focus_window();
                }
            }
            window::Action::Present(id) => {
                if let Some(window) = window_manager.get_mut(id) {
                    present::present(
                        &window.raw,
                        single_instance::take_activation_token(),
                    );
                }
            }
            window::Action::SetLevel(id, level) => {
                if let Some(window) = window_manager.get_mut(id) {
                    window
                        .raw
                        .set_window_level(conversion::window_level(level));
                }
            }
            window::Action::ShowSystemMenu(id) => {
                if let Some(window) = window_manager.get_mut(id)
                    && let mouse::Cursor::Available(point) =
                        window.state.cursor()
                {
                    window.raw.show_window_menu(winit::dpi::LogicalPosition {
                        x: point.x,
                        y: point.y,
                    });
                }
            }
            window::Action::GetRawId(id, channel) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = channel.send(window.raw.id().into());
                }
            }
            window::Action::Run(id, f) => {
                if let Some(window) = window_manager.get_mut(id) {
                    f(window);
                }
            }
            window::Action::Screenshot(id, channel) => {
                if let Some(window) = window_manager.get_mut(id)
                    && let Some(compositor) = compositor
                {
                    let bytes = compositor.screenshot(
                        &mut window.renderer,
                        window.state.viewport(),
                        window.state.background_color(),
                    );

                    let _ = channel.send(core::window::Screenshot::new(
                        bytes,
                        window.state.physical_size(),
                        window.state.scale_factor(),
                    ));
                }
            }
            window::Action::EnableMousePassthrough(id) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = window.raw.set_cursor_hittest(false);
                }
            }
            window::Action::DisableMousePassthrough(id) => {
                if let Some(window) = window_manager.get_mut(id) {
                    let _ = window.raw.set_cursor_hittest(true);
                }
            }
            window::Action::GetMonitorSize(id, channel) => {
                if let Some(window) = window_manager.get(id) {
                    let size = window.raw.current_monitor().map(|monitor| {
                        let scale = window.state.scale_factor();
                        let size = monitor.size().to_logical(f64::from(scale));

                        Size::new(size.width, size.height)
                    });

                    let _ = channel.send(size);
                }
            }
            window::Action::SetAllowAutomaticTabbing(enabled) => {
                control_sender
                    .start_send(Control::SetAutomaticWindowTabbing(enabled))
                    .expect("Send control action");
            }
            window::Action::RedrawAll => {
                for (_id, window) in window_manager.iter_mut() {
                    window.raw.request_redraw();
                }
            }
            window::Action::RelayoutAll => {
                for (id, window) in window_manager.iter_mut() {
                    if let Some(ui) = interfaces.remove(&id) {
                        let _ = interfaces.insert(
                            id,
                            ui.relayout(
                                window.state.logical_size(),
                                &mut window.renderer,
                            ),
                        );
                    }

                    window.raw.request_redraw();
                }
            }
        },
        Action::System(action) => match action {
            system::Action::GetInformation(_channel) => {
                #[cfg(feature = "sysinfo")]
                {
                    if let Some(compositor) = compositor {
                        let graphics_info = compositor.information();

                        let _ = std::thread::spawn(move || {
                            let information = system_information(graphics_info);

                            let _ = _channel.send(information);
                        });
                    }
                }
            }
            system::Action::GetTheme(channel) => {
                let _ = channel.send(*system_theme);
            }
            system::Action::RequestQuit => {
                runtime.broadcast(subscription::Event::QuitRequested);
            }
            system::Action::NotifyTheme(mode) => {
                if mode != *system_theme {
                    *system_theme = mode;

                    runtime.broadcast(subscription::Event::SystemThemeChanged(
                        mode,
                    ));
                }

                let Some(theme) = conversion::window_theme(mode) else {
                    return;
                };

                for (_id, window) in window_manager.iter_mut() {
                    window.state.update(
                        program,
                        &window.raw,
                        &winit::event::WindowEvent::ThemeChanged(theme),
                    );
                }
            }
        },
        Action::Widget(operation) => {
            let mut current_operation = Some(operation);

            while let Some(mut operation) = current_operation.take() {
                for (id, ui) in interfaces.iter_mut() {
                    if let Some(window) = window_manager.get_mut(*id) {
                        ui.operate(&window.renderer, operation.as_mut());
                    }
                }

                match operation.finish() {
                    operation::Outcome::None => {}
                    operation::Outcome::Some(()) => {}
                    operation::Outcome::Chain(next) => {
                        current_operation = Some(next);
                    }
                }
            }
        }
        Action::Shell(action) => match action {
            runtime::shell::Action::SetActivationPolicy(_policy) => {
                #[cfg(target_os = "macos")]
                platform::set_activation_policy(_policy);
            }
        },
        Action::Tray(action) => {
            tray.perform(action, |stream| spawn(runtime, stream));
        }
        Action::Image(action) => match action {
            image::Action::Allocate(handle, sender) => {
                use core::Renderer as _;

                // TODO: Shared image cache in compositor
                if let Some((_id, window)) = window_manager.iter_mut().next() {
                    window.renderer.allocate_image(
                        &handle,
                        move |allocation| {
                            let _ = sender.send(allocation);
                        },
                    );
                }
            }
        },
        Action::LoadFont { bytes, channel } => {
            if let Some(compositor) = compositor {
                // TODO: Error handling (?)
                compositor.load_font(bytes.clone());

                let _ = channel.send(Ok(()));
            }
        }
        Action::Reload => {
            for (id, window) in window_manager.iter_mut() {
                let Some(ui) = interfaces.remove(&id) else {
                    continue;
                };

                let cache = ui.into_cache();
                let size = window.logical_size();

                let _ = interfaces.insert(
                    id,
                    build_user_interface(
                        program,
                        cache,
                        &mut window.renderer,
                        size,
                        id,
                    ),
                );

                window.raw.request_redraw();
            }
        }
        Action::Exit => {
            control_sender
                .start_send(Control::Exit)
                .expect("Send control action");
        }
    }
}

/// Build the user interface for every window.
pub fn build_user_interfaces<'a, P: Program, C>(
    program: &'a program::Instance<P>,
    window_manager: &mut WindowManager<P, C>,
    mut cached_user_interfaces: FxHashMap<window::Id, user_interface::Cache>,
) -> FxHashMap<window::Id, UserInterface<'a, P::Message, P::Theme, P::Renderer>>
where
    C: Compositor<Renderer = P::Renderer>,
    P::Theme: theme::Base,
{
    for (id, window) in window_manager.iter_mut() {
        window.state.synchronize(program, id, &window.raw);
    }

    debug::theme_changed(|| {
        window_manager
            .first()
            .and_then(|window| theme::Base::palette(window.state.theme()))
    });

    cached_user_interfaces
        .drain()
        .filter_map(|(id, cache)| {
            let window = window_manager.get_mut(id)?;

            Some((
                id,
                build_user_interface(
                    program,
                    cache,
                    &mut window.renderer,
                    window.state.logical_size(),
                    id,
                ),
            ))
        })
        .collect()
}

/// Returns true if the provided event should cause a [`Program`] to
/// exit.
pub fn user_force_quit(
    event: &winit::event::WindowEvent,
    _modifiers: winit::keyboard::ModifiersState,
) -> bool {
    match event {
        #[cfg(target_os = "macos")]
        winit::event::WindowEvent::KeyboardInput {
            event:
                winit::event::KeyEvent {
                    logical_key: winit::keyboard::Key::Character(c),
                    state: winit::event::ElementState::Pressed,
                    ..
                },
            ..
        } if c == "q" && _modifiers.super_key() => true,
        _ => false,
    }
}

#[cfg(feature = "sysinfo")]
fn system_information(
    graphics: compositor::Information,
) -> system::Information {
    use sysinfo::{Process, System};

    let mut system = System::new_all();
    system.refresh_all();

    let cpu_brand = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().to_string())
        .unwrap_or_default();

    let memory_used = sysinfo::get_current_pid()
        .and_then(|pid| system.process(pid).ok_or("Process not found"))
        .map(Process::memory)
        .ok();

    system::Information {
        system_name: System::name(),
        system_kernel: System::kernel_version(),
        system_version: System::long_os_version(),
        system_short_version: System::os_version(),
        cpu_brand,
        cpu_cores: system.physical_core_count(),
        memory_total: system.total_memory(),
        memory_used,
        graphics_adapter: graphics.adapter,
        graphics_backend: graphics.backend,
    }
}
