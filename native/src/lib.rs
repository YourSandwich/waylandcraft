use crate::bridge::BridgeState;
use crate::ddm::WLCDataState;
use crate::egl::EGLHelper;
use crate::output::WLCOutput;
use crate::seat::WLCSeatState;
use crate::xdg_spec::XDGSpecHelper;
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::utils::on_commit_buffer_handler,
    },
    delegate_compositor, delegate_dmabuf, delegate_shm,
    delegate_single_pixel_buffer, delegate_viewporter, delegate_xdg_shell,
    delegate_xwayland_shell,
    input::{SeatHandler, SeatState, dnd::DndGrabHandler},
    reexports::{
        calloop::{self, EventLoop, generic::Generic as GenericEvent},
        wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
        wayland_server::{
            self, Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{
                wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat,
                wl_surface::WlSurface,
            },
        },
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState,
        },
        dmabuf::{
            DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState,
            ImportNotifier,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
            XdgShellState,
        },
        shm::{ShmHandler, ShmState},
        single_pixel_buffer::SinglePixelBufferState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandClientData, XWaylandEvent,
        xwm::XwmId,
    },
};
use std::ffi::OsString;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

mod bridge;
mod ddm;
mod egl;
mod output;
mod process;
mod seat;
mod svg;
mod utils;
mod xdg_spec;
mod xdnd;
mod xwayland;

pub(crate) struct WaylandCraft<'a> {
    pub state: WLCState,
    pub event_loop: EventLoop<'a, WLCState>,
    pub bridge: BridgeState,
    pub egl: EGLHelper,
    pub xdg: XDGSpecHelper,
}

pub struct WLCState {
    pub display_handle: DisplayHandle,
    pub socket: OsString,
    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub xdg_state: XdgShellState,
    pub viewporter_state: ViewporterState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: DmabufGlobal,
    pub requests: WindowRequests,
    pub seat: WLCSeatState,
    pub data: WLCDataState,
    pub output: WLCOutput,
    // Smithay SeatState - only exists to satisfy the SeatHandler bound on
    // X11Wm::start_wm. No smithay Seat is created; seat.rs is the real seat.
    pub seat_state: SeatState<WLCState>,
    pub xwayland_shell_state: XWaylandShellState,
    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,
    // The XDND foundation: WaylandCraft's own X11 client connection driving
    // X11<->Wayland drag-and-drop. None until Xwayland is up, or if the second
    // connection fails to open - XDND is then simply unavailable. See xdnd.rs.
    pub xdnd: Option<xdnd::XdndState>,
    // A write-only X11 client connection used to set the X server's input
    // focus when keyboard focus moves to an X11 window. None until Xwayland is
    // up, or if the connection fails to open. See xwayland::X11FocusConn.
    pub x11_focus: Option<xwayland::X11FocusConn>,
    // X11 windows, tracked from map to unmap/destroy. toplevels() syncs
    // bridge.toplevels from this list.
    pub x11_windows: Vec<X11Surface>,
    // X11 override-redirect windows (menus, tooltips, dropdowns). Kept apart
    // from x11_windows so they reach the popup path, not the toplevel list;
    // popups() syncs bridge.popups from this list.
    pub x11_override_windows: Vec<X11Surface>,
    // The X11 toplevel currently holding WaylandCraft's keyboard focus, if any.
    // Holds the X11 Activated state so the previously focused X11 window can be
    // deactivated on a focus change; see xwayland::sync_x11_focus.
    pub focused_x11: Option<X11Surface>,
}

#[derive(Default)]
pub struct WindowRequests {
    pub minimize: Vec<ToplevelSurface>,
    pub maximize: Vec<ToplevelSurface>,
    pub unmaximize: Vec<ToplevelSurface>,
    pub fullscreen: Vec<ToplevelSurface>,
    pub unfullscreen: Vec<ToplevelSurface>,
    pub move_interactive: Vec<Serial>,
    pub resize_interactive: Vec<(Serial, ResizeEdge)>,
}

impl WLCState {
    fn new(disp: DisplayHandle, egl: &EGLHelper) -> Self {
        let compositor_state = CompositorState::new::<WLCState>(&disp);
        let shm_state = ShmState::new::<WLCState>(&disp, vec![]);
        let xdg_state = XdgShellState::new::<WLCState>(&disp);
        let viewporter_state = ViewporterState::new::<WLCState>(&disp);
        let single_pixel_buffer_state =
            SinglePixelBufferState::new::<WLCState>(&disp);

        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global = init_dmabuf(&disp, &mut dmabuf_state, egl);

        let seat = WLCSeatState::new();
        seat.create_globals(&disp);

        let data = WLCDataState::new(&disp);
        data.create_global();

        let output = WLCOutput::new(&disp);
        output.create_global();

        let xwayland_shell_state = XWaylandShellState::new::<WLCState>(&disp);

        Self {
            display_handle: disp.clone(),
            socket: OsString::new(),
            compositor_state,
            shm_state,
            xdg_state,
            viewporter_state,
            single_pixel_buffer_state,
            dmabuf_state,
            dmabuf_global,
            requests: WindowRequests::default(),
            seat,
            data,
            output,
            seat_state: SeatState::new(),
            xwayland_shell_state,
            xwm: None,
            xdisplay: None,
            xdnd: None,
            x11_focus: None,
            x11_windows: vec![],
            x11_override_windows: vec![],
            focused_x11: None,
        }
    }
}

fn init_dmabuf(
    disp: &DisplayHandle,
    state: &mut DmabufState,
    egl: &EGLHelper,
) -> DmabufGlobal {
    let render_node =
        egl.get_render_node().expect("Failed to get render node!");
    let render_node_id = render_node.dev_id();
    let formats = egl.query_dmabuf_formats();

    let feedback = DmabufFeedbackBuilder::new(render_node_id, formats)
        .build()
        .unwrap();

    state.create_global_with_default_feedback::<WLCState>(disp, &feedback)
}

impl CompositorHandler for WLCState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a wayland_server::Client,
    ) -> &'a CompositorClientState {
        // The Xwayland client carries XWaylandClientData, not WLCClient.
        if let Some(data) = client.get_data::<XWaylandClientData>() {
            return &data.compositor_state;
        }
        &client.get_data::<WLCClient>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        // Hand buffer management to smithay: this builds a per-surface
        // RendererSurfaceState that holds the wl_buffer in a refcounted
        // Buffer and sends wl_buffer.release only once a newer buffer
        // supersedes it (or the surface is destroyed). The render thread
        // then reads that state read-only; see bridge::updateSurfaceData.
        on_commit_buffer_handler::<WLCState>(surface);
    }
}

impl BufferHandler for WLCState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for WLCState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for WLCState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let _ = notifier.successful::<WLCState>();
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
        positioner: PositionerState,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_configure().expect("popup initial configure");
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.requests.minimize.push(surface);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.requests.maximize.push(surface);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.requests.unmaximize.push(surface);
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<WlOutput>,
    ) {
        self.requests.fullscreen.push(surface);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.requests.unfullscreen.push(surface);
    }

    fn move_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: WlSeat,
        serial: Serial,
    ) {
        self.requests.move_interactive.push(serial);
    }

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: WlSeat,
        serial: Serial,
        edges: ResizeEdge,
    ) {
        self.requests.resize_interactive.push((serial, edges));
    }
}

// Required by X11Wm::start_wm. WaylandCraft has no smithay Seat, so the focus
// types are never actually used - X11Surface is the minimal type satisfying the
// PointerFocus: DndFocus bound without pulling in DataDeviceHandler (which a
// WlSurface focus would, colliding with the hand-rolled ddm.rs).
impl SeatHandler for WLCState {
    type KeyboardFocus = X11Surface;
    type PointerFocus = X11Surface;
    type TouchFocus = X11Surface;

    fn seat_state(&mut self) -> &mut SeatState<WLCState> {
        &mut self.seat_state
    }
}

impl DndGrabHandler for WLCState {}

impl XWaylandShellHandler for WLCState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        _xwm: XwmId,
        _surface: WlSurface,
        _window: X11Surface,
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
    fn initialized(&self, _id: ClientId) {}

    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {}
}

pub(crate) fn wlc_init(
    egl: EGLHelper,
) -> Result<WaylandCraft<'static>, Box<dyn std::error::Error>> {
    let event_loop: EventLoop<WLCState> = EventLoop::try_new()?;
    let display: Display<WLCState> = Display::new()?;
    let socket = ListeningSocketSource::new_auto()?;

    let mut state = WLCState::new(display.handle(), &egl);
    state.socket = socket.socket_name().to_os_string();

    let ev_handle = event_loop.handle();

    ev_handle
        .insert_source(socket, |stream, _, state| {
            let client = WLCClient::new();
            state
                .display_handle
                .insert_client(stream, Arc::new(client))
                .unwrap();
        })
        .unwrap();

    let display_source = GenericEvent::new(
        display,
        calloop::Interest::READ,
        calloop::Mode::Level,
    );
    ev_handle
        .insert_source(display_source, |_, display_io, state| {
            unsafe {
                display_io.get_mut().dispatch_clients(state).unwrap();
            }
            Ok(calloop::PostAction::Continue)
        })
        .unwrap();

    spawn_xwayland(&ev_handle, &state.display_handle);

    let xdg = XDGSpecHelper::init();

    let instance = WaylandCraft {
        state,
        event_loop,
        bridge: BridgeState::new(),
        egl,
        xdg,
    };
    Ok(instance)
}

fn spawn_xwayland(
    handle: &calloop::LoopHandle<'static, WLCState>,
    display: &DisplayHandle,
) {
    let (xwayland, client) = match XWayland::spawn(
        display,
        None,
        std::iter::empty::<(String, String)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    ) {
        Ok(xwayland) => xwayland,
        Err(err) => {
            println!("[waylandcraft] failed to spawn Xwayland: {}", err);
            return;
        }
    };

    let display = display.clone();
    let loop_handle = handle.clone();
    let ret = handle.insert_source(xwayland, move |event, _, state| match event {
        XWaylandEvent::Ready {
            x11_socket,
            display_number,
        } => {
            let wm = match X11Wm::start_wm(
                loop_handle.clone(),
                &display,
                x11_socket,
                client.clone(),
            ) {
                Ok(wm) => wm,
                Err(err) => {
                    println!("[waylandcraft] failed to start X11 WM: {}", err);
                    return;
                }
            };
            state.xwm = Some(wm);
            state.xdisplay = Some(display_number);
            println!("[waylandcraft] Xwayland ready on display :{}", display_number);

            // Open the X11 input-focus connection. A failure here is non-fatal:
            // X11 windows just will not receive X11 input focus.
            match xwayland::X11FocusConn::create(display_number) {
                Ok(focus_conn) => state.x11_focus = Some(focus_conn),
                Err(err) => println!(
                    "[waylandcraft] X11 focus connection failed: {}",
                    err
                ),
            }

            // Stand up the XDND foundation on its own X11 connection. A failure
            // here is non-fatal: X11<->Wayland DnD is just unavailable.
            match xdnd::XdndState::create(display_number, &loop_handle) {
                Ok(xdnd) => {
                    state.xdnd = Some(xdnd);
                }
                Err(err) => {
                    println!("[waylandcraft] XDND init failed, X11 DnD disabled: {}", err)
                }
            }
        }
        XWaylandEvent::Error => {
            println!("[waylandcraft] Xwayland crashed on startup");
        }
    });
    if let Err(err) = ret {
        println!("[waylandcraft] failed to insert Xwayland source: {}", err);
    }
}

impl<'a> WaylandCraft<'a> {
    pub fn update(&mut self) {
        let state = &mut self.state;
        let event_loop = &mut self.event_loop;
        event_loop.dispatch(Some(Duration::ZERO), state).unwrap();
        state.display_handle.flush_clients().unwrap();
    }
}

delegate_compositor!(WLCState);
delegate_shm!(WLCState);
delegate_xdg_shell!(WLCState);
delegate_viewporter!(WLCState);
delegate_single_pixel_buffer!(WLCState);
delegate_dmabuf!(WLCState);
delegate_xwayland_shell!(WLCState);
