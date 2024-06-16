use std::{
    ptr::NonNull,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
};

use rwh_06::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_buffer, wl_compositor,
        wl_display::WlDisplay,
        wl_keyboard,
        wl_pointer::{self, ButtonState},
        wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::xdg::{
    activation::v1::client::{
        xdg_activation_token_v1::XdgActivationTokenV1, xdg_activation_v1::XdgActivationV1,
    },
    decoration::zv1::client::{
        zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
    },
    shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
};
use xkbcommon::xkb;
use xkeysym::KeyCode;

use crate::{
    keyboard::{Key, KeyState},
    mouse, WindowAttribs, WindowEvent, WindowMode,
};

pub struct PlatformWindow {
    pub display: WlDisplay,
    event_queue: EventQueue<State>,
    state: State,
}

struct State {
    running: bool,
    pub width: u32,
    pub height: u32,
    base_surface: Option<wl_surface::WlSurface>,
    buffer: Option<wl_buffer::WlBuffer>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    xdg_surface: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
    xdg_decoration_manager: Option<ZxdgDecorationManagerV1>,
    xdg_toplevel_decoration: Option<ZxdgToplevelDecorationV1>,
    configured: bool,

    // Keybaord
    xkb_context: Mutex<xkb::Context>,
    xkb_state: Mutex<Option<xkb::State>>,

    event_sender: Sender<WindowEvent>,
    event_receiver: Receiver<WindowEvent>,
}

delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);

impl State {
    fn init_xdg_surface(&mut self, qh: &QueueHandle<State>, attris: &WindowAttribs) {
        let wm_base = self.wm_base.as_ref().unwrap();
        let base_surface = self.base_surface.as_ref().unwrap();

        let xdg_surface = wm_base.get_xdg_surface(base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        toplevel.set_title(attris.title.clone());
        toplevel.set_app_id("com.ventengine.VentEngine".into());

        match attris.mode {
            WindowMode::FullScreen => toplevel.set_fullscreen(None),
            WindowMode::Maximized => toplevel.set_maximized(),
            WindowMode::Minimized => toplevel.set_minimized(),
            _ => {}
        }
        if let Some(max_size) = attris.max_size {
            toplevel.set_max_size(max_size.0 as i32, max_size.1 as i32)
        }

        if let Some(min_size) = attris.min_size {
            toplevel.set_min_size(min_size.0 as i32, min_size.1 as i32)
        }

        if let Some(manager) = &self.xdg_decoration_manager {
            // if supported, let the compositor render titlebars for us
            self.xdg_toplevel_decoration = Some(manager.get_toplevel_decoration(&toplevel, qh, ()));
            self.xdg_toplevel_decoration.as_ref().unwrap().set_mode(wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1::Mode::ServerSide);
        }

        self.xdg_surface = Some((xdg_surface, toplevel));
    }

    fn init_xdg_activation(&mut self, qh: &QueueHandle<State>, xdg_activation_v1: XdgActivationV1) {
        let token = xdg_activation_v1.get_activation_token(qh, ());
        token.set_app_id("com.ventengine.VentEngine".into());
        token.set_surface(self.base_surface.as_ref().unwrap())
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key,
            serial,
            time,
            state: key_state,
        } = event
        {
            match key_state {
                WEnum::Value(key_state) => {
                    let state_guard = state.xkb_state.lock().unwrap();

                    if let Some(guard) = state_guard.as_ref() {
                        let keycode = KeyCode::new(key + 8);
                        let keysym = guard.key_get_one_sym(keycode);
                        let key_state = match key_state {
                            wl_keyboard::KeyState::Pressed => KeyState::Pressed,
                            wl_keyboard::KeyState::Released => KeyState::Released,
                            _ => unreachable!(),
                        };

                        state
                            .event_sender
                            .send(WindowEvent::Key {
                                key: convert_key(keysym.raw()),
                                state: key_state,
                            })
                            .expect("Failed to send key event");
                    }
                }
                WEnum::Unknown(u) => log::error!("Invalid key state {}", u),
            }
        } else if let wl_keyboard::Event::Keymap { format, fd, size } = event {
            match format {
                WEnum::Value(format) => match format {
                    wl_keyboard::KeymapFormat::NoKeymap => {
                        log::error!("no keymap")
                    }

                    wl_keyboard::KeymapFormat::XkbV1 => {
                        match unsafe {
                            let context = state.xkb_context.lock().unwrap();

                            xkb::Keymap::new_from_fd(
                                &context,
                                fd,
                                size as usize,
                                xkb::KEYMAP_FORMAT_TEXT_V1,
                                xkb::COMPILE_NO_FLAGS,
                            )
                        } {
                            Ok(Some(keymap)) => {
                                let xkb_state = xkb::State::new(&keymap);
                                {
                                    let mut state_guard = state.xkb_state.lock().unwrap();
                                    *state_guard = Some(xkb_state);
                                }
                            }

                            Ok(None) => {
                                log::error!("invalid keymap");
                            }

                            Err(err) => {
                                log::error!("{}", err);
                            }
                        }
                    }
                    _ => unreachable!(),
                },
                WEnum::Unknown(_) => todo!(),
            }
        }
    }
}

// Taken from <linux/input-event-codes.h>.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;
const BTN_FORWARD: u32 = 0x115;
const BTN_BACK: u32 = 0x116;

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_pointer::WlPointer,
        event: <wl_pointer::WlPointer as Proxy>::Event,
        data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            time,
            button,
            state: mouse_state,
        } = event
        {
            let mouse_state = match mouse_state {
                wayland_client::WEnum::Value(ButtonState::Pressed) => mouse::ButtonState::Pressed,
                wayland_client::WEnum::Value(ButtonState::Released) => mouse::ButtonState::Released,
                WEnum::Value(_) => mouse::ButtonState::Released,
                WEnum::Unknown(_) => mouse::ButtonState::Released,
            };
            match button {
                BTN_LEFT => state
                    .event_sender
                    .send(WindowEvent::Mouse {
                        key: crate::mouse::Key::LEFT,
                        state: mouse_state,
                    })
                    .unwrap(),
                BTN_RIGHT => state
                    .event_sender
                    .send(WindowEvent::Mouse {
                        key: crate::mouse::Key::RIGHT,
                        state: mouse_state,
                    })
                    .unwrap(),
                BTN_MIDDLE => state
                    .event_sender
                    .send(WindowEvent::Mouse {
                        key: crate::mouse::Key::MIDDLE,
                        state: mouse_state,
                    })
                    .unwrap(),
                BTN_SIDE => state
                    .event_sender
                    .send(WindowEvent::Mouse {
                        key: crate::mouse::Key::SIDE,
                        state: mouse_state,
                    })
                    .unwrap(),
                BTN_EXTRA => state
                    .event_sender
                    .send(WindowEvent::Mouse {
                        key: crate::mouse::Key::EXTRA,
                        state: mouse_state,
                    })
                    .unwrap(),
                BTN_FORWARD => state
                    .event_sender
                    .send(WindowEvent::Mouse {
                        key: crate::mouse::Key::FORWARD,
                        state: mouse_state,
                    })
                    .unwrap(),
                BTN_BACK => state
                    .event_sender
                    .send(WindowEvent::Mouse {
                        key: crate::mouse::Key::BACK,
                        state: mouse_state,
                    })
                    .unwrap(),
                _ => (),
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        data: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
            let surface = state.base_surface.as_ref().unwrap();
            if let Some(ref buffer) = state.buffer {
                surface.attach(Some(buffer), 0, 0);
                surface.commit();
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close {} = event {
            state
                .event_sender
                .send(WindowEvent::Close)
                .expect("Failed to send Close Event");
        } else if let xdg_toplevel::Event::ConfigureBounds { width, height } = event {
            state.width = width as u32;
            state.height = height as u32;
        } else if let xdg_toplevel::Event::Configure {
            width,
            height,
            states,
        } = event
        {
            state.width = width as u32;
            state.height = height as u32;
        }
    }
}

impl wayland_client::Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: <wl_registry::WlRegistry as Proxy>::Event,
        data: &GlobalListContents,
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_compositor::WlCompositor,
        event: <wl_compositor::WlCompositor as Proxy>::Event,
        data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgDecorationManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZxdgDecorationManagerV1,
        event: <ZxdgDecorationManagerV1 as Proxy>::Event,
        data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        todo!()
    }
}
impl Dispatch<ZxdgToplevelDecorationV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZxdgToplevelDecorationV1,
        event: <ZxdgToplevelDecorationV1 as Proxy>::Event,
        data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        todo!()
    }
}
impl Dispatch<XdgActivationV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &XdgActivationV1,
        event: <XdgActivationV1 as Proxy>::Event,
        data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        todo!()
    }
}
impl Dispatch<XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &XdgActivationTokenV1,
        event: <XdgActivationTokenV1 as Proxy>::Event,
        data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        todo!()
    }
}

impl PlatformWindow {
    pub fn create_window(attribs: &WindowAttribs) -> Self {
        let conn = wayland_client::Connection::connect_to_env().expect("Failed to get connection");
        println!("Connected to Wayland Server");

        let (event_sender, event_receiver) = channel::<WindowEvent>();

        let mut state = State {
            running: true,
            width: attribs.width,
            height: attribs.height,
            base_surface: None,
            buffer: None,
            wm_base: None,
            xdg_surface: None,
            configured: false,
            xdg_toplevel_decoration: None,
            xdg_decoration_manager: None,
            event_receiver,
            event_sender,
            xkb_context: Mutex::new(xkb::Context::new(xkb::CONTEXT_NO_FLAGS)),
            xkb_state: Mutex::new(None),
        };

        let display = conn.display();

        let (globals, event_queue) = registry_queue_init::<State>(&conn).unwrap();
        let qhandle = event_queue.handle();

        let wm_base: xdg_wm_base::XdgWmBase =
            globals.bind(&event_queue.handle(), 1..=6, ()).unwrap();
        state.wm_base = Some(wm_base);

        let compositor: wl_compositor::WlCompositor =
            globals.bind(&event_queue.handle(), 1..=6, ()).unwrap();
        let surface = compositor.create_surface(&qhandle, ());
        state.base_surface = Some(surface);

        let wl_seat: wl_seat::WlSeat = globals.bind(&event_queue.handle(), 1..=6, ()).unwrap();
        // let xdg_decoration_manager: ZxdgDecorationManagerV1 =
        //     globals.bind(&event_queue.handle(), 1..=1, ()).unwrap();
        // state.xdg_decoration_manager = Some(xdg_decoration_manager);

        if state.wm_base.is_some() && state.xdg_surface.is_none() {
            state.init_xdg_surface(&qhandle, attribs);
        }
        state.base_surface.as_ref().unwrap().commit();

        let xdg_activation: XdgActivationV1 =
            globals.bind(&event_queue.handle(), 1..=1, ()).unwrap();

        state.init_xdg_activation(&qhandle, xdg_activation);

        PlatformWindow {
            display,
            state,
            event_queue,
        }
    }

    pub fn poll<F>(mut self, mut event_handler: F)
    where
        F: FnMut(WindowEvent),
    {
        while self.state.running {
            self.event_queue
                .dispatch_pending(&mut self.state)
                .expect("Failed to dispatch pending");

            while let Ok(event) = self.state.event_receiver.try_recv() {
                event_handler(event);
            }

            event_handler(WindowEvent::Draw);
        }
    }

    pub fn width(&self) -> u32 {
        self.state.width
    }

    pub fn height(&self) -> u32 {
        self.state.height
    }

    pub fn raw_display_handle(&self) -> RawDisplayHandle {
        RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(self.display.id().as_ptr().cast()).unwrap(),
        ))
    }

    pub fn raw_window_handle(&self) -> RawWindowHandle {
        let ptr = self.state.base_surface.as_ref().unwrap().id().as_ptr();
        RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(ptr as *mut _).unwrap(),
        ))
    }

    pub fn close(&mut self) {
        self.event_queue
            .flush()
            .expect("Failed to flush Event Queue");
        self.state.running = false;
    }
}

impl Drop for PlatformWindow {
    fn drop(&mut self) {
        self.close()
    }
}

fn convert_key(raw_key: xkeysym::RawKeysym) -> Key {
    match raw_key {
        xkeysym::key::A | xkeysym::key::a => Key::A,
        xkeysym::key::B | xkeysym::key::b => Key::B,
        xkeysym::key::C | xkeysym::key::c => Key::C,
        xkeysym::key::D | xkeysym::key::d => Key::D,
        xkeysym::key::E | xkeysym::key::e => Key::E,
        xkeysym::key::F | xkeysym::key::f => Key::F,
        xkeysym::key::G | xkeysym::key::g => Key::G,
        xkeysym::key::H | xkeysym::key::h => Key::H,
        xkeysym::key::I | xkeysym::key::i => Key::I,
        xkeysym::key::J | xkeysym::key::j => Key::J,
        xkeysym::key::K | xkeysym::key::k => Key::K,
        xkeysym::key::L | xkeysym::key::l => Key::L,
        xkeysym::key::M | xkeysym::key::m => Key::M,
        xkeysym::key::N | xkeysym::key::n => Key::N,
        xkeysym::key::O | xkeysym::key::o => Key::O,
        xkeysym::key::P | xkeysym::key::p => Key::P,
        xkeysym::key::Q | xkeysym::key::q => Key::Q,
        xkeysym::key::R | xkeysym::key::r => Key::R,
        xkeysym::key::S | xkeysym::key::s => Key::S,
        xkeysym::key::T | xkeysym::key::t => Key::T,
        xkeysym::key::U | xkeysym::key::u => Key::U,
        xkeysym::key::V | xkeysym::key::v => Key::V,
        xkeysym::key::W | xkeysym::key::w => Key::W,
        xkeysym::key::X | xkeysym::key::x => Key::X,
        xkeysym::key::Y | xkeysym::key::y => Key::Y,
        xkeysym::key::Z | xkeysym::key::z => Key::Z,

        xkeysym::key::space => Key::Space,
        xkeysym::key::Shift_L => Key::ShiftL,
        xkeysym::key::Shift_R => Key::ShiftR,
        xkeysym::key::leftarrow => Key::Leftarrow,
        xkeysym::key::uparrow => Key::Uparrow,
        xkeysym::key::rightarrow => Key::Rightarrow,
        xkeysym::key::downarrow => Key::Downarrow,

        _ => {
            log::warn!("Unknown key {}", raw_key);
            Key::Unknown
        }
    }
}
