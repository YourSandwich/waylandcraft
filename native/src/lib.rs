use std::sync::Arc;
use std::ops::DerefMut;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::ffi::OsString;
use smithay::{
    reexports::{
        calloop::{
            generic::Generic as GenericEvent,
            self, EventLoop,
        },
        wayland_server::{
            self,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{
                wl_surface::WlSurface,
                wl_buffer::WlBuffer,
                wl_seat::WlSeat,
            },
            Display, DisplayHandle,
        },
    },
    wayland::{
        socket::ListeningSocketSource,
        compositor::{
            CompositorState, CompositorClientState, CompositorHandler,
            with_states, SurfaceAttributes, BufferAssignment,
            with_surface_tree_downward, TraversalAction
        },
        buffer::BufferHandler,
        shm::{ShmState, ShmHandler},
        output::OutputHandler,
        shell::xdg::{
            XdgShellState, XdgShellHandler, ToplevelSurface, PopupSurface,
            PositionerState
        },
    },
    output::{self, Output, PhysicalProperties, Subpixel},
    input::{
        pointer::PointerHandle,
        keyboard::{KeyboardHandle, XkbConfig},
        SeatState, SeatHandler
    },
    utils::Serial,
    delegate_compositor, delegate_shm, delegate_output, delegate_seat,
    delegate_xdg_shell,
};

mod bridge;

pub(crate) struct WaylandCraft<'a> {
    pub state: WLCState,
    pub event_loop: EventLoop<'a, WLCState>,
}

pub(crate) struct WLCState {
    pub display_handle: DisplayHandle,
    pub socket: OsString,
    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub xdg_state: XdgShellState,
    pub pointer: PointerHandle<Self>,
    pub keyboard: KeyboardHandle<Self>,
}

impl WLCState {
    fn new(disp: DisplayHandle) -> Self {
        let compositor_state = CompositorState::new::<WLCState>(&disp);
        let shm_state = ShmState::new::<WLCState>(&disp, vec![]);

        let mut seat_state = SeatState::<WLCState>::new();
        let mut seat = seat_state.new_wl_seat(&disp, "seat-0");
        let pointer = seat.add_pointer();
        let keyboard = seat.add_keyboard(XkbConfig::default(), 200, 25)
            .expect("Keyboard create");

        let xdg_state = XdgShellState::new::<WLCState>(&disp);

        Self {
            display_handle: disp.clone(),
            socket: OsString::new(),
            compositor_state,
            shm_state,
            seat_state,
            xdg_state,
            pointer,
            keyboard,
        }
    }
}

impl CompositorHandler for WLCState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a wayland_server::Client,
    ) -> &'a CompositorClientState {
        &client.get_data::<WLCClient>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        with_states(surface, |data| {
            let mut attr_guard = data
                .cached_state
                .get::<SurfaceAttributes>();
            let attr = attr_guard
                .deref_mut()
                .current();
            let maybe_buf = if let Some(assign) = &attr.buffer {
                match assign {
                    BufferAssignment::NewBuffer(b) => Some(b),
                    BufferAssignment::Removed => None,
                }
            } else {
                None
            };
            if let Some(buf) = maybe_buf {
                buf.release();
            }
            attr.buffer = None;
        });
    }
}

impl BufferHandler for WLCState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {
    }
}

impl ShmHandler for WLCState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl OutputHandler for WLCState {}

impl SeatHandler for WLCState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
}

impl XdgShellHandler for WLCState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.send_configure();
    }

    fn new_popup(
        &mut self,
        surface: PopupSurface,
        _positioner: PositionerState
    ) {
        surface.send_configure().expect("Initial popup configure");
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32
    ) {
    }
}

pub(crate) struct WLCClient {
    compositor_state: CompositorClientState,
}

impl WLCClient {
    fn new() -> Self {
        Self {
            compositor_state: CompositorClientState::default(),
        }
    }
}

impl ClientData for WLCClient {
    fn initialized(&self, _id: ClientId) {
    }

    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {
    }
}

fn send_frame(state: &mut WLCState) {
    let toplevels = state.xdg_state.toplevel_surfaces();
    let time: u128 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    for toplevel in toplevels {
        let toplevel_surface = toplevel.wl_surface();

        with_surface_tree_downward(
            toplevel_surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |_, data, _| {
                let mut attr_guard = data
                    .cached_state
                    .get::<SurfaceAttributes>();
                let attr = attr_guard
                    .deref_mut()
                    .current();
                for c in attr.frame_callbacks.drain(..) {
                    c.done(time as u32);
                }
            },
            |_, _, _| true,
        );
    }
}

fn register_virtual_output(state: &mut WLCState) {
    let output = Output::new(
        "output-0".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Virtual".into(),
            model: "Monitor".into(),
        },
    );
    output.change_current_state(
        Some(output::Mode { size: (1920, 1080).into(), refresh: 60000 }),
        None,
        None,
        Some((0, 0).into())
    );
    output.create_global::<WLCState>(&state.display_handle);
}

pub(crate) fn wlc_init(
) -> Result<WaylandCraft<'static>, Box<dyn std::error::Error>> {
    let event_loop: EventLoop<WLCState> = EventLoop::try_new()?;
    let display: Display<WLCState> = Display::new()?;
    let socket = ListeningSocketSource::new_auto()?;

    let mut state = WLCState::new(display.handle());
    state.socket = socket.socket_name().to_os_string();

    register_virtual_output(&mut state);

    let ev_handle = event_loop.handle();

    ev_handle.insert_source(socket, |stream, _, state| {
        let client = WLCClient::new();
        state.display_handle.insert_client(stream, Arc::new(client)).unwrap();
    }).unwrap();

    let display_source = GenericEvent::new(
        display, calloop::Interest::READ, calloop::Mode::Level
    );
    ev_handle.insert_source(display_source, |_, display_io, state| {
        unsafe {
            display_io.get_mut().dispatch_clients(state).unwrap();
        }
        Ok(calloop::PostAction::Continue)
    }).unwrap();

    Ok(WaylandCraft { state, event_loop })
}

impl<'a> WaylandCraft<'a> {
    pub fn update(&mut self) {
        let state = &mut self.state;
        let event_loop = &mut self.event_loop;
        event_loop.dispatch(Some(Duration::ZERO), state).unwrap();
        send_frame(state);
        state.display_handle.flush_clients().unwrap();
    }
}

delegate_compositor!(WLCState);
delegate_shm!(WLCState);
delegate_output!(WLCState);
delegate_seat!(WLCState);
delegate_xdg_shell!(WLCState);
