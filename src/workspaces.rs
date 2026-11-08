// SPDX-License-Identifier: GPL-3.0-only

//! Tiling of the active workspace on this panel's output, through COSMIC's
//! workspace protocol, as the tiling applet does it (adapted from
//! cosmic-ext-applet-quick-settings); and the keyboard layout in use, through
//! its keyboard layout protocol, as the input sources applet does it. One
//! connection: the privileged socket can be used once.
use std::os::{
    fd::{FromRawFd, RawFd},
    unix::net::UnixStream,
};

use cosmic::{
    cctk::{
        self,
        cosmic_protocols::{
            keyboard_layout::v1::client::zcosmic_keyboard_layout_v1::ZcosmicKeyboardLayoutV1,
            workspace::v2::client::zcosmic_workspace_handle_v2::TilingState,
        },
        keyboard_layout::{KeyboardLayoutHandler, KeyboardLayoutState},
        sctk::{
            self,
            output::{OutputHandler, OutputState},
            reexports::{
                calloop, calloop_wayland_source::WaylandSource,
                protocols::ext::workspace::v1::client::ext_workspace_handle_v1,
            },
            registry::{ProvidesRegistryState, RegistryState},
            seat::{Capability, SeatHandler, SeatState},
        },
        wayland_client::{
            Connection, QueueHandle, WEnum, delegate_noop, globals::registry_queue_init,
            protocol::{wl_keyboard::WlKeyboard, wl_output::WlOutput, wl_seat::WlSeat},
        },
        workspace::{Workspace, WorkspaceHandler, WorkspaceState},
    },
    iced::{
        Subscription,
        futures::{self, SinkExt, channel::mpsc},
        stream,
    },
};

#[derive(Clone, Debug)]
pub enum Event {
    Ready(calloop::channel::Sender<Command>),
    /// Whether the active workspace tiles its windows.
    Tiling(bool),
    /// The keyboard layout in use, by its place in the list.
    Layout(usize),
}

#[derive(Clone, Debug)]
pub enum Command {
    Tiling(bool),
    /// Switch to the layout at this place in the list.
    Layout(u32),
}

pub fn subscription() -> Subscription<Event> {
    Subscription::run_with(std::any::TypeId::of::<Event>(), |_| {
        stream::channel(4, |mut output: mpsc::Sender<Event>| async move {
            let (tx, rx) = calloop::channel::channel();
            _ = output.send(Event::Ready(tx)).await;
            std::thread::spawn(move || {
                if let Err(err) = run(output, rx) {
                    tracing::warn!("workspace tracking stopped: {err}");
                }
            });
            futures::future::pending::<()>().await;
        })
    })
}

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    workspace_state: WorkspaceState,
    seat_state: SeatState,
    keyboard_layout_state: KeyboardLayoutState,
    keyboard: Option<(WlSeat, WlKeyboard, ZcosmicKeyboardLayoutV1)>,
    /// The output this panel is on, by name, and once known.
    output_name: String,
    output: Option<WlOutput>,
    tx: mpsc::Sender<Event>,
    last: Option<bool>,
}

impl State {
    fn active(&self) -> Option<&Workspace> {
        self.workspace_state.workspace_groups().find_map(|group| {
            if self.output.as_ref().is_some_and(|o| !group.outputs.contains(o)) {
                return None;
            }
            group
                .workspaces
                .iter()
                .filter_map(|handle| self.workspace_state.workspace_info(handle))
                .find(|w| w.state.contains(ext_workspace_handle_v1::State::Active))
        })
    }

    fn report(&mut self) {
        let tiling = self.active().and_then(|w| w.tiling).map(|t| {
            matches!(t, WEnum::Value(TilingState::TilingEnabled))
        });
        if let Some(tiling) = tiling.filter(|t| Some(*t) != self.last) {
            self.last = Some(tiling);
            _ = futures::executor::block_on(self.tx.send(Event::Tiling(tiling)));
        }
    }

    fn set_layout(&self, group: u32) {
        if let Some((_, _, layout)) = &self.keyboard {
            layout.set_group(group);
        }
    }

    fn set_tiling(&mut self, tiling: bool) {
        let Some(handle) = self.active().and_then(|w| w.cosmic_handle.clone()) else {
            return;
        };
        handle.set_tiling_state(if tiling {
            TilingState::TilingEnabled
        } else {
            TilingState::FloatingOnly
        });
        if let Ok(manager) = self.workspace_state.workspace_manager().get() {
            manager.commit();
        }
    }
}

fn run(
    tx: mpsc::Sender<Event>,
    rx: calloop::channel::Channel<Command>,
) -> Result<(), Box<dyn std::error::Error>> {
    // The real compositor, not the panel's, which applets live in.
    let socket = std::env::var("X_PRIVILEGED_WAYLAND_SOCKET")
        .ok()
        .and_then(|fd| fd.parse::<RawFd>().ok())
        .map(|fd| unsafe { UnixStream::from_raw_fd(fd) });
    let conn = match socket {
        Some(socket) => Connection::from_socket(socket)?,
        None => Connection::connect_to_env()?,
    };
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();
    let mut event_loop = calloop::EventLoop::<State>::try_new()?;
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .map_err(|err| err.error)?;
    event_loop
        .handle()
        .insert_source(rx, |event, (), state| {
            match event {
                calloop::channel::Event::Msg(Command::Tiling(tiling)) => state.set_tiling(tiling),
                calloop::channel::Event::Msg(Command::Layout(group)) => state.set_layout(group),
                calloop::channel::Event::Closed => {}
            }
        })
        .map_err(|err| err.error)?;

    let registry_state = RegistryState::new(&globals);
    let mut state = State {
        output_state: OutputState::new(&globals, &qh),
        workspace_state: WorkspaceState::new(&registry_state, &qh),
        seat_state: SeatState::new(&globals, &qh),
        keyboard_layout_state: KeyboardLayoutState::new(&registry_state, &qh),
        keyboard: None,
        registry_state,
        output_name: std::env::var("COSMIC_PANEL_OUTPUT").unwrap_or_default(),
        output: None,
        tx,
        last: None,
    };
    loop {
        event_loop.dispatch(None, &mut state)?;
    }
}

impl WorkspaceHandler for State {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }

    fn done(&mut self) {
        self.report();
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: WlOutput) {
        let name = self.output_state.info(&output).and_then(|i| i.name);
        if name.as_deref() == Some(self.output_name.as_str()) {
            self.output = Some(output);
            self.report();
        }
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    sctk::registry_handlers![OutputState, SeatState];
}

impl KeyboardLayoutHandler for State {
    fn group(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: &ZcosmicKeyboardLayoutV1, group: u32) {
        _ = futures::executor::block_on(self.tx.send(Event::Layout(group as usize)));
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, capability: Capability) {
        if capability != Capability::Keyboard || self.keyboard.is_some() {
            return;
        }
        let keyboard = seat.get_keyboard(qh, ());
        match self.keyboard_layout_state.get_keyboard_layout(&keyboard, qh) {
            Some(layout) => self.keyboard = Some((seat, keyboard, layout)),
            None => keyboard.release(),
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            self.drop_keyboard(&seat);
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat) {
        self.drop_keyboard(&seat);
    }
}

impl State {
    fn drop_keyboard(&mut self, seat: &WlSeat) {
        if let Some((_, keyboard, layout)) = self.keyboard.take_if(|(s, _, _)| s == seat) {
            layout.destroy();
            keyboard.release();
        }
    }
}

sctk::delegate_output!(State);
sctk::delegate_registry!(State);
cctk::delegate_workspace!(State);
sctk::delegate_seat!(State);
cctk::delegate_keyboard_layout!(State);
// Keys go to windows, not here: only the layout's group is wanted.
delegate_noop!(State: ignore WlKeyboard);
