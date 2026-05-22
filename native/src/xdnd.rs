// X11 <-> Wayland drag-and-drop: the XDND protocol, hand-implemented.
//
// smithay's X11Wm does not expose what XDND needs (sending ClientMessages,
// owning the XdndSelection, creating helper windows - all private), so this
// module opens its own second x11rb connection to the Xwayland display and
// drives XDND as a plain X11 client alongside smithay's WM connection. The two
// connections do not conflict: smithay is the WM, this one is just a client.
//
// Both drag directions are bridged. Wayland->X11: when a Wayland app drags onto
// an X11 window, WaylandCraft speaks XDND to that window as the drag source,
// owning the XdndSelection on a 1x1 helper window. X11->Wayland: when an X11 app
// starts an XDND drag, WaylandCraft maps a full-screen drop proxy so the X11
// source addresses its XDND messages here, synthesizes a Wayland wl_data_offer
// from the X11 source's types, and drives the Wayland app under the cursor while
// replying XdndStatus to the X11 source.

use crate::WLCState;
use crate::utils::new_serial;
use smithay::reexports::calloop::{
    Interest, LoopHandle, Mode, PostAction, RegistrationToken,
    channel::Event as ChannelEvent, generic::Generic,
};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::x11rb::X11Source;
use smithay::xwayland::{X11Surface, XWaylandClientData};
use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use x11rb::{
    CURRENT_TIME, NONE,
    connection::Connection as _,
    protocol::{
        Event,
        xfixes::{
            ConnectionExt as _, SelectionEventMask,
            SelectionNotifyEvent as XfixesSelectionNotifyEvent,
        },
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent,
            ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask,
            Property, PropertyNotifyEvent, PropMode, Screen,
            SelectionNotifyEvent, SelectionRequestEvent, StackMode, Window,
            WindowClass, SELECTION_NOTIFY_EVENT,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

// XDND protocol version WaylandCraft speaks. 5 is the current revision and what
// every modern toolkit (GTK, Qt) negotiates.
const XDND_VERSION: u32 = 5;
// Oldest XDND version an X11 target may advertise and still be driven. v0/v1
// lack the action protocol; modern toolkits are all >= 2.
const MIN_XDND_VERSION: u32 = 2;

// INCR chunk size for outgoing selection transfers. There is no way to query
// the X server's real maximum request size, so this is the conventional 64 KiB
// wlroots/smithay also use; payloads beyond it switch to an INCR transfer.
const INCR_CHUNK_SIZE: usize = 64 * 1024;

// get_property long_length, in 32-bit units: the whole property in one read.
// ~2 GiB, the value smithay uses - large enough for any real selection, small
// enough not to be an absurd request descriptor.
const PROP_READ_LEN: u32 = 0x1fff_ffff;

x11rb::atom_manager! {
    // Atoms for the XDND protocol and the selection transfers behind it.
    pub XdndAtoms: XdndAtomsCookie {
        // XDND protocol
        XdndSelection,
        XdndAware,
        XdndStatus,
        XdndPosition,
        XdndEnter,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndProxy,
        XdndTypeList,
        XdndActionMove,
        XdndActionCopy,
        XdndActionAsk,
        XdndActionPrivate,
        // Selection-transfer machinery
        _WL_SELECTION,
        TARGETS,
        TIMESTAMP,
        INCR,
        DELETE,
        UTF8_STRING,
        TEXT,
        WM_NAME,
        // Private: only the type of the ClientMessage X11Source sends to its
        // close window to wake the reader thread on shutdown.
        _WLC_CLOSE_CONNECTION,
    }
}

// The XDND state: WaylandCraft's own X11 client connection to Xwayland plus
// everything XDND is driven through. Created once Xwayland is up (lib.rs, the
// XWaylandEvent::Ready handler); torn down when the X server goes away
// (xwayland.rs, XwmHandler::disconnected).
pub struct XdndState {
    // The second X11 connection - a plain client connection, distinct from
    // smithay's WM connection. Arc so X11Source's reader thread can share it.
    pub conn: Arc<RustConnection>,
    pub atoms: XdndAtoms,
    pub screen: Screen,
    // 1x1, never mapped. Owns the XdndSelection during a Wayland->X11 drag and
    // is the source window in XdndEnter/Position/Drop messages.
    pub selection_window: Window,
    // Full-screen, created unmapped. Mapped and raised during an inbound
    // X11->Wayland drag so XDND messages from the X11 source land here.
    pub drop_proxy_window: Window,
    // The active Wayland->X11 drag interaction, if the in-world cursor is over
    // (or was last over) an X11 window during a Wayland drag. None outside an
    // X11 interaction.
    source: Option<XdndSource>,
    // The active X11->Wayland drag interaction: an X11 app started an XDND drag
    // and the drop proxy is mapped to catch it. None outside one.
    incoming: Option<XdndIncoming>,
    // In-flight outgoing selection transfers, keyed by the requestor window.
    // An X11 target that took the drop calls convert_selection on the dragged
    // data; the bytes come from the Wayland source over a pipe and are written
    // to the requestor's property, with INCR for large payloads.
    outgoing: HashMap<Window, OutgoingTransfer>,
    // In-flight incoming selection transfers, keyed by the property
    // convert_selection delivers into. A Wayland target reading the X11 drag's
    // data makes WaylandCraft convert_selection the X11 source; the bytes come
    // back on the selection window's property and are piped to the Wayland fd.
    incoming_transfers: HashMap<Atom, IncomingTransfer>,
    // calloop registration of the X11 event source plus the loop handle to
    // remove it with, so disconnected() can drop the source and let the
    // X11Source's reader thread shut down cleanly.
    source_token: RegistrationToken,
    loop_handle: LoopHandle<'static, WLCState>,
}

// Per-interaction state for an outbound Wayland->X11 drag. Tracks which X11
// window the cursor is over and the negotiated XDND parameters with it.
struct XdndSource {
    // The X11 window the cursor currently sits over and that XDND messages are
    // addressed to (logically). None between leaving one window and entering
    // the next.
    target: Option<XdndTarget>,
    // True once set_selection_owner(XdndSelection) succeeded for this drag.
    owns_selection: bool,
}

// One X11 target the drag has entered. Holds the real window, its XdndProxy
// delivery window (XDND messages route through the proxy when one is set), and
// the target's last XdndStatus (whether it accepts a drop, and the action it
// would perform). Copy - small scalars, copied out to keep XDND sends off the
// XdndState borrow that holds it.
#[derive(Clone, Copy)]
struct XdndTarget {
    window: Window,
    // Where XDND ClientMessages are delivered: the XdndProxy window if the
    // target declared one, else `window` itself.
    event_window: Window,
    // From the target's last XdndStatus: will it accept a drop here.
    accepted: bool,
    // From the target's last XdndStatus: the action it would perform on drop.
    action: DndAction,
}

// One outgoing selection transfer: the Wayland source's bytes on their way to
// an X11 requestor's property. Ported from smithay's xwm/selection.rs - the
// INCR state machine is identical, only the connection differs.
struct OutgoingTransfer {
    conn: Arc<RustConnection>,
    // calloop source reading the pipe fed by the Wayland source. Taken once the
    // read side is fully drained.
    token: Option<RegistrationToken>,
    // Bytes read from the Wayland source, not yet written to the property.
    source_data: Vec<u8>,
    request: SelectionRequestEvent,
    incr: bool,
    // The requestor's property currently holds a chunk awaiting its delete.
    property_set: bool,
    // A chunk is buffered and should be flushed on the next property delete.
    flush_property_on_delete: bool,
    // The terminating 0-byte INCR chunk has been written.
    sent_finished: bool,
}

// The active X11->Wayland drag. An X11 app owns the XdndSelection and addresses
// XDND ClientMessages to WaylandCraft's mapped drop proxy; this holds what is
// needed to drive the Wayland target and answer the X11 source.
struct XdndIncoming {
    // The X11 source window - the one that claimed XdndSelection. XdndStatus,
    // XdndFinished and convert_selection target this window.
    source_window: Window,
    // True between XdndEnter and XdndLeave/XdndDrop - the source has announced
    // its types and a Wayland wl_data_offer has been synthesized.
    entered: bool,
    // True once XdndDrop arrived; the Wayland target is consuming the data and
    // the interaction ends on its Finish (-> XdndFinished to the source).
    dropped: bool,
    // The X11 source timestamp from the last XdndPosition/XdndDrop, used as the
    // convert_selection timestamp. CURRENT_TIME until the first position.
    last_timestamp: u32,
}

// One incoming selection transfer: the X11 drag source's data on its way to a
// Wayland client's fd. WaylandCraft convert_selection's the source onto its
// selection window; the reply arrives as a property (INCR for large payloads)
// and is piped to `fd`. Mirrors the clipboard X11->Wayland read.
struct IncomingTransfer {
    // The Wayland client's pipe write end; bytes read off the X11 property are
    // written here. Closed (dropped) when the transfer completes.
    fd: OwnedFd,
    // True once the first INCR chunk arrived - the property is then drained
    // chunk by chunk, each delete requesting the next.
    incr: bool,
}

impl XdndState {
    // Stand up the XDND foundation against the Xwayland display `:display`.
    // Opens the connection, interns the atoms, creates both helper windows, and
    // registers the X11 event source on the calloop loop. Any X11 failure
    // returns Err so the caller can leave xdnd = None - XDND is then simply
    // unavailable and the compositor runs on without it.
    pub fn create(
        display: u32,
        loop_handle: &LoopHandle<'static, WLCState>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let (conn, screen_num) = RustConnection::connect(Some(&format!(":{}", display)))?;
        let conn = Arc::new(conn);

        let atoms = XdndAtoms::new(conn.as_ref())?.reply()?;
        // connect() parses the screen out of the display string; ":N" has none,
        // so this is screen 0 - but index by what it returned, not a constant.
        let screen = conn.setup().roots[screen_num].clone();

        let selection_window = create_helper_window(&conn, &screen, &atoms, 1, 1)?;
        let drop_proxy_window = create_helper_window(
            &conn,
            &screen,
            &atoms,
            screen.width_in_pixels,
            screen.height_in_pixels,
        )?;

        // XFixes lets WaylandCraft notice an X11 app claiming XdndSelection -
        // the start of an X11->Wayland drag. query_version both negotiates the
        // extension and registers it on the connection's extension manager, so
        // the reader thread can decode Xfixes SelectionNotify events. Then
        // watch XdndSelection's owner.
        conn.xfixes_query_version(5, 0)?.reply()?;
        conn.xfixes_select_selection_input(
            selection_window,
            atoms.XdndSelection,
            SelectionEventMask::SET_SELECTION_OWNER
                | SelectionEventMask::SELECTION_WINDOW_DESTROY
                | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?;
        conn.flush()?;

        // X11Source runs a reader thread that wait_for_event()s the connection
        // and forwards events over a calloop channel - the only race-free way
        // to integrate x11rb with calloop. The close window/atom let its Drop
        // wake that thread; the selection window serves as the close target.
        let source = X11Source::new(
            Arc::clone(&conn),
            selection_window,
            atoms._WLC_CLOSE_CONNECTION,
        );
        let source_token = loop_handle.insert_source(source, |event, _, state| {
            if let ChannelEvent::Msg(event) = event {
                state.handle_xdnd_event(event);
            }
        })?;

        Ok(XdndState {
            conn,
            atoms,
            screen,
            selection_window,
            drop_proxy_window,
            source: None,
            incoming: None,
            outgoing: HashMap::new(),
            incoming_transfers: HashMap::new(),
            source_token,
            loop_handle: loop_handle.clone(),
        })
    }

    // Tear down the XDND foundation. Remove the X11 event source first: its
    // Drop wakes the reader thread by sending a ClientMessage to the selection
    // window, so that window must still exist here - destroy the helper windows
    // only afterwards. The connection drops with self once the source is gone.
    pub fn destroy(mut self) {
        for (_, transfer) in self.outgoing.drain() {
            if let Some(token) = transfer.token {
                self.loop_handle.remove(token);
            }
        }
        self.loop_handle.remove(self.source_token);
        let _ = self.conn.destroy_window(self.selection_window);
        let _ = self.conn.destroy_window(self.drop_proxy_window);
        let _ = self.conn.flush();
    }

    // True if a Wayland->X11 drag currently has an X11 target. Used by
    // XwmHandler::disconnected to tell whether a drag in flight involves X11
    // and must be cancelled when the X server goes away.
    pub fn has_outgoing_target(&self) -> bool {
        self.source.as_ref().is_some_and(|s| s.target.is_some())
    }

    // Send one XDND ClientMessage. `target` is the real target window, named in
    // the message body so a proxy can forward it; `dest` is the window the
    // event is delivered to - the XdndProxy window if the target declared one,
    // else `target` itself. Errors are logged, never propagated: a failed send
    // must not abort the drag.
    fn send_xdnd(&self, target: Window, dest: Window, ty: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, target, ty, data);
        if let Err(err) =
            self.conn.send_event(false, dest, EventMask::NO_EVENT, event)
        {
            println!("[waylandcraft] XDND: send_event failed: {}", err);
        }
        let _ = self.conn.flush();
    }

    // Send a target-role XDND ClientMessage (XdndStatus / XdndFinished) to the
    // X11 drag source. These flow target -> source, so the message's window
    // field is the source window itself (not the proxy, which goes in data[0]).
    // Errors are logged, never propagated.
    fn send_to_source(&self, source: Window, ty: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, source, ty, data);
        if let Err(err) =
            self.conn.send_event(false, source, EventMask::NO_EVENT, event)
        {
            println!("[waylandcraft] XDND: send_event failed: {}", err);
        }
        let _ = self.conn.flush();
    }
}

// Resolve an X11 window's XDND proxy: the two-step self-verifying check from
// XDND spec / smithay's get_proxy_window. A target may delegate XDND messages
// to a proxy window declared in its XdndProxy property; the proxy must point
// back at itself for the delegation to be honored.
fn get_proxy_window(
    conn: &RustConnection,
    atoms: &XdndAtoms,
    window: Window,
) -> Option<Window> {
    let prop = conn
        .get_property(false, window, atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let proxy = prop.value32()?.next()?;
    let verify = conn
        .get_property(false, proxy, atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    if verify.value32()?.next()? == proxy {
        Some(proxy)
    } else {
        None
    }
}

// Read an X11 window's XdndAware version. Returns None if the window does not
// advertise XDND (property absent or wrong type).
fn get_xdnd_version(
    conn: &RustConnection,
    atoms: &XdndAtoms,
    window: Window,
) -> Option<u32> {
    let prop = conn
        .get_property(false, window, atoms.XdndAware, AtomEnum::ATOM, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    if prop.type_ != u32::from(AtomEnum::ATOM) {
        return None;
    }
    prop.value32()?.next()
}

// mime type -> X11 atom. text/plain and its utf-8 variant map to the
// conventional TEXT/UTF8_STRING atoms; anything else is interned by name.
// Mirrors smithay's atom_from_mime (xwm/mod.rs).
fn atom_from_mime(
    conn: &RustConnection,
    atoms: &XdndAtoms,
    mime: &str,
) -> Option<Atom> {
    match mime {
        "text/plain" => Some(atoms.TEXT),
        "text/plain;charset=utf-8" => Some(atoms.UTF8_STRING),
        other => conn
            .intern_atom(false, other.as_bytes())
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom),
    }
}

// X11 atom -> mime type. Inverse of atom_from_mime; mirrors smithay's
// mime_from_atom (xwm/mod.rs).
fn mime_from_atom(
    conn: &RustConnection,
    atoms: &XdndAtoms,
    atom: Atom,
) -> Option<String> {
    if atom == atoms.TEXT {
        return Some("text/plain".to_string());
    }
    if atom == atoms.UTF8_STRING {
        return Some("text/plain;charset=utf-8".to_string());
    }
    let reply = conn.get_atom_name(atom).ok()?.reply().ok()?;
    String::from_utf8(reply.name).ok()
}

// A single DndAction -> XDND action atom. XDND carries exactly one action;
// copy is XDND's always-supported default, so an empty/unknown action falls
// back to it rather than sending NONE (which targets read as "rejected").
fn action_to_atom(atoms: &XdndAtoms, action: DndAction) -> Atom {
    if action.contains(DndAction::Move) {
        atoms.XdndActionMove
    } else if action.contains(DndAction::Ask) {
        atoms.XdndActionAsk
    } else {
        atoms.XdndActionCopy
    }
}

// The action atom to advertise in XdndPosition for a Wayland->X11 drag. The
// Wayland source offers an action *set* (commonly Copy|Move); XDND carries one
// action, and copy is the only universally accepted one - a target that cannot
// move rejects a move-only drag outright. Mirrors smithay's preferred_action:
// echo back the action the target negotiated via XdndStatus when the source
// supports it, otherwise prefer copy. `supported` is the source's offered set,
// `negotiated` the target's last XdndStatus action (empty before the first).
fn source_action_atom(
    atoms: &XdndAtoms,
    supported: DndAction,
    negotiated: DndAction,
) -> Atom {
    if !negotiated.is_empty() && supported.contains(negotiated) {
        action_to_atom(atoms, negotiated)
    } else if supported.contains(DndAction::Copy) {
        atoms.XdndActionCopy
    } else if supported.contains(DndAction::Move) {
        atoms.XdndActionMove
    } else {
        atoms.XdndActionCopy
    }
}

// Create one XDND helper window on the second connection: override-redirect so
// smithay's WM leaves it alone (it routes to new_override_redirect_window and
// otherwise ignores it), PROPERTY_CHANGE so INCR selection transfers can be
// driven off PropertyNotify, and XdndAware set so it advertises XDND v5. Both
// helper windows are created unmapped; the caller maps the proxy when needed.
fn create_helper_window(
    conn: &RustConnection,
    screen: &Screen,
    atoms: &XdndAtoms,
    width: u16,
    height: u16,
) -> Result<Window, Box<dyn std::error::Error>> {
    let window = conn.generate_id()?;
    conn.create_window(
        screen.root_depth,
        window,
        screen.root,
        0,
        0,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.XdndAware,
        AtomEnum::ATOM,
        &[XDND_VERSION],
    )?;
    Ok(window)
}

// Locate the tracked X11Surface a Wayland surface belongs to. The focus during
// a drag is a WlSurface; for an Xwayland window that surface is mirrored by an
// X11Surface in x11_windows/x11_override_windows, whose window_id is the XDND
// target and whose geometry gives its position.
fn x11_surface_for(state: &WLCState, surface: &WlSurface) -> Option<X11Surface> {
    state
        .x11_windows
        .iter()
        .chain(state.x11_override_windows.iter())
        .find(|w| w.wl_surface().as_ref() == Some(surface))
        .cloned()
}

// True if a Wayland surface is backed by an Xwayland (X11) client.
fn is_xwayland_surface(surface: &WlSurface) -> bool {
    surface
        .client()
        .is_some_and(|c| c.get_data::<XWaylandClientData>().is_some())
}

impl WLCState {
    // Drag motion entry point for the JNI bridge (DNDGrab / X11DNDGrab).
    //
    // For a Wayland-sourced drag: if the cursor sits over an Xwayland window,
    // drive the XDND source and hide the focus from the data-device; otherwise
    // run the plain Wayland<->Wayland path (ddm::dnd_motion).
    //
    // For an X11-sourced drag: only feed the Wayland data-device, and only with
    // native Wayland surfaces - an Xwayland surface under the cursor is treated
    // as no target (re-bridging X11->X11 is out of scope).
    pub fn dnd_motion(&mut self, surface: Option<WlSurface>, x: f64, y: f64) {
        let source = self
            .data
            .dnd
            .as_ref()
            .filter(|d| !d.dropped)
            .and_then(|d| d.source.clone());

        // An X11-sourced drag never drives the XDND source - it IS the XDND
        // source's drag, re-delivered to a Wayland target via the data-device.
        if matches!(source, Some(crate::ddm::DndSource::X11 { .. })) {
            let target =
                surface.filter(|s| !is_xwayland_surface(s));
            self.data.dnd_motion(target, x, y);
            return;
        }

        // Is the focused surface an Xwayland window WaylandCraft tracks, and
        // does the drag carry a source to feed XDND with?
        let x11 = surface
            .as_ref()
            .filter(|s| is_xwayland_surface(s))
            .and_then(|s| x11_surface_for(self, s));

        match (x11, source) {
            (Some(window), Some(source)) => {
                // Wayland drag over an X11 window: speak XDND to it. The
                // Wayland data-device path must not also see this surface.
                let mimes = source.mime();
                let action = source.actions();
                let geo = window.geometry();
                let root_x = geo.loc.x + x.round() as i32;
                let root_y = geo.loc.y + y.round() as i32;
                self.xdnd_source_motion(
                    window.window_id(),
                    root_x,
                    root_y,
                    &mimes,
                    action,
                );
                self.data.dnd_motion(None, x, y);
            }
            _ => {
                // Not over a bridgeable X11 target - drop any X11 target the
                // drag had, then run the Wayland path. A sourceless drag onto
                // Xwayland also lands here: nothing to bridge, so it is simply
                // suppressed by the data-device path's own cross-client guard.
                self.xdnd_source_leave();
                self.data.dnd_motion(surface, x, y);
            }
        }
    }

    // Drop entry point for the JNI bridge. If the drag is currently over a
    // willing X11 window, hand the drop to XDND; otherwise run the plain
    // Wayland drop. An X11-sourced drag's drop is driven by the X11 source's
    // XdndDrop, not the in-world mouse - so this is a no-op for one.
    pub fn dnd_drop(&mut self) {
        if matches!(
            self.data.dnd.as_ref().and_then(|d| d.source.as_ref()),
            Some(crate::ddm::DndSource::X11 { .. })
        ) {
            return;
        }
        if let Some(action) = self.xdnd_source_drop() {
            // XdndDrop is sent. Tell the Wayland source the drop happened the
            // same way ddm::dnd_drop does for a Wayland target (action, then
            // dnd_drop_performed); dnd_finished waits for the target's
            // XdndFinished. Keep the drag alive but mark it dropped so further
            // motion is ignored.
            if let Some(dnd) = self.data.dnd.as_mut() {
                dnd.dropped = true;
                if let Some(src) = dnd.source.as_ref() {
                    src.action(action);
                    src.dnd_drop_performed();
                }
            }
            return;
        }
        // The drag was not over a willing X11 target - release the XDND
        // selection if it was claimed, then run the Wayland drop.
        self.xdnd_source_cancel();
        self.data.dnd_drop();
    }

    // Cancel entry point for the JNI bridge. Tear down any in-progress XDND
    // interaction, then run the plain Wayland cancel. For an X11-sourced drag
    // this cancels the inbound interaction (XdndStatus reject + clear).
    pub fn dnd_cancel(&mut self) {
        if matches!(
            self.data.dnd.as_ref().and_then(|d| d.source.as_ref()),
            Some(crate::ddm::DndSource::X11 { .. })
        ) {
            self.xdnd_target_cancel();
            return;
        }
        self.xdnd_source_cancel();
        self.data.dnd_cancel();
    }

    // Drive the XDND source for a Wayland drag whose cursor sits over the X11
    // window `window` at root coordinates `root_x`/`root_y`. `mimes` are the
    // Wayland source's offered types, `action` the drag's current action.
    //
    // Sends XdndEnter on first entry to a window (and XdndLeave + XdndEnter on
    // moving between X11 windows), then XdndPosition for the move. Claims the
    // XdndSelection lazily on the first call of an interaction. Every X11 call
    // is best-effort: a failure logs and the drag continues degraded.
    fn xdnd_source_motion(
        &mut self,
        window: Window,
        root_x: i32,
        root_y: i32,
        mimes: &[String],
        action: DndAction,
    ) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };

        // Claim the selection once per interaction. A Wayland app expects to be
        // the XDND source, so the selection window must own XdndSelection
        // before any target converts it.
        let source = xdnd.source.get_or_insert(XdndSource {
            target: None,
            owns_selection: false,
        });
        if !source.owns_selection {
            if let Err(err) = xdnd.conn.set_selection_owner(
                xdnd.selection_window,
                xdnd.atoms.XdndSelection,
                CURRENT_TIME,
            ) {
                println!("[waylandcraft] XDND: claiming selection failed: {}", err);
                return;
            }
            source.owns_selection = true;
        }

        // Already over this window: just send a position update. The target is
        // copied out so the send does not borrow xdnd.source.
        if let Some(target) = source.target.filter(|t| t.window == window) {
            send_position(xdnd, &target, root_x, root_y, action);
            return;
        }

        // Moved to a different X11 window (or entered the first one): leave the
        // old target, then enter the new one.
        if let Some(old) = source.target.take() {
            send_leave(xdnd, &old);
        }
        if let Some(target) = enter_target(xdnd, window, mimes) {
            send_position(xdnd, &target, root_x, root_y, action);
            xdnd.source.as_mut().unwrap().target = Some(target);
        }
        // If enter_target returned None the window does not speak a usable
        // XDND version - nothing to drive, the drag continues elsewhere.
    }

    // The Wayland drag moved off all X11 windows (onto a Wayland surface or
    // nothing). Send XdndLeave to the current X11 target, if any.
    fn xdnd_source_leave(&mut self) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        if let Some(source) = xdnd.source.as_mut()
            && let Some(target) = source.target.take()
        {
            send_leave(xdnd, &target);
        }
    }

    // The drag ended without dropping on an X11 window (the Wayland source was
    // cancelled, or the cursor was not over an X11 window at release). Leave
    // the current target and release the XdndSelection.
    fn xdnd_source_cancel(&mut self) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        let Some(source) = xdnd.source.take() else {
            return;
        };
        if let Some(target) = source.target.as_ref() {
            send_leave(xdnd, target);
        }
        if source.owns_selection {
            let _ = xdnd.conn.set_selection_owner(
                NONE,
                xdnd.atoms.XdndSelection,
                CURRENT_TIME,
            );
            let _ = xdnd.conn.flush();
        }
    }

    // The user dropped the Wayland drag over the X11 window currently under the
    // cursor. Send XdndDrop. The target then converts the selection, arriving
    // as a SelectionRequest; the drag is finished only on the target's
    // XdndFinished. Returns the action the target reported it would take, or
    // None if there is no willing X11 target to drop on - the caller then
    // falls back to cancelling the drag.
    fn xdnd_source_drop(&mut self) -> Option<DndAction> {
        let xdnd = self.xdnd.as_mut()?;
        let target = xdnd.source.as_ref()?.target.as_ref()?;
        if !target.accepted {
            // The target rejected every position - dropping would be ignored.
            return None;
        }
        // A target that accepts but reported no action falls back to copy,
        // XDND's always-available action.
        let action = if target.action.is_empty() {
            DndAction::Copy
        } else {
            target.action
        };
        let data = [xdnd.selection_window, 0, CURRENT_TIME, 0, 0];
        xdnd.send_xdnd(
            target.window,
            target.event_window,
            xdnd.atoms.XdndDrop,
            data,
        );
        Some(action)
    }

    // Entry point for every XDND event off the second X11 connection. The
    // Wayland->X11 source side handles XdndStatus/XdndFinished from the X11
    // target and SelectionRequest/PropertyNotify serving the data; the
    // X11->Wayland target side handles XfixesSelectionNotify, XdndEnter/
    // Position/Leave/Drop to the proxy, and SelectionNotify delivering the
    // data. Never panics: every X11 failure is logged and the drag continues
    // or is dropped.
    fn handle_xdnd_event(&mut self, event: Event) {
        if self.xdnd.is_none() {
            return;
        }
        match event {
            Event::ClientMessage(msg) => {
                let xdnd = self.xdnd.as_ref().unwrap();
                let data = msg.data.as_data32();
                let ty = msg.type_;
                if ty == xdnd.atoms.XdndStatus {
                    self.handle_xdnd_status(data);
                } else if ty == xdnd.atoms.XdndFinished {
                    self.handle_xdnd_finished(data);
                } else if ty == xdnd.atoms.XdndEnter {
                    self.handle_xdnd_enter(data);
                } else if ty == xdnd.atoms.XdndPosition {
                    self.handle_xdnd_position(data);
                } else if ty == xdnd.atoms.XdndLeave {
                    self.handle_xdnd_leave(data);
                } else if ty == xdnd.atoms.XdndDrop {
                    self.handle_xdnd_drop(data);
                }
            }
            Event::SelectionRequest(req) => self.handle_selection_request(req),
            Event::PropertyNotify(n) => self.handle_property_notify(n),
            Event::SelectionNotify(n) => self.handle_xdnd_selection_notify(n),
            Event::XfixesSelectionNotify(n) => {
                self.handle_xfixes_selection_notify(n)
            }
            _ => {}
        }
    }

    // XdndStatus from the X11 target: whether it accepts a drop at the last
    // reported position, and the action it would take. data[0] is the target
    // window, data[1] bit 0 the accept flag, data[4] the action atom. The
    // rectangle (data[2..3]) is a motion-suppression hint WaylandCraft does
    // not use - positions are already rate-limited upstream.
    fn handle_xdnd_status(&mut self, data: [u32; 5]) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        if let Some(source) = xdnd.source.as_mut()
            && let Some(target) = source.target.as_mut()
            && target.window == data[0]
        {
            target.accepted = (data[1] & 1) != 0;
            target.action = action_from_atom(&xdnd.atoms, data[4]);
        }
    }

    // XdndFinished from the X11 target: it consumed the drop, the drag is done.
    // Complete the Wayland source the same way a Wayland<->Wayland drop does
    // (ddm::dnd_drop): report the performed action, signal dnd_finished, then
    // drop the drag. data[0] is the target window; data[2] carries the action
    // the target performed (XDND v5).
    fn handle_xdnd_finished(&mut self, data: [u32; 5]) {
        let action = {
            let Some(xdnd) = self.xdnd.as_mut() else {
                return;
            };
            let Some(source) = xdnd.source.as_ref() else {
                return;
            };
            if source.target.as_ref().is_none_or(|t| t.window != data[0]) {
                return;
            }
            // data[1] bit 0 set means the action atom in data[2] is valid; an
            // absent or unrecognised action falls back to copy.
            let action = action_from_atom(&xdnd.atoms, data[2]);
            let action = if data[1] & 1 != 0 && !action.is_empty() {
                action
            } else {
                DndAction::Copy
            };
            xdnd.source = None;
            // Release the selection - the transfer, if any, already finished.
            let _ = xdnd.conn.set_selection_owner(
                NONE,
                xdnd.atoms.XdndSelection,
                CURRENT_TIME,
            );
            let _ = xdnd.conn.flush();
            action
        };

        if let Some(dnd) = self.data.dnd.as_ref()
            && let Some(src) = dnd.source.as_ref()
        {
            src.action(action);
            src.dnd_finished();
        }
        self.data.dnd = None;
    }

    // A SelectionRequest on the second connection: an X11 target is fetching
    // the dragged data via convert_selection(XdndSelection, ...). Serve it from
    // the Wayland source - open a pipe, hand the write end to the source, and
    // pump the read end into the requestor's property (INCR for large data).
    // Mirrors the clipboard bridge's data-serving path (smithay xwm/selection).
    fn handle_selection_request(&mut self, req: SelectionRequestEvent) {
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        let atoms = xdnd.atoms;
        let conn = Arc::clone(&xdnd.conn);

        // Only the XdndSelection is served here; anything else is not ours.
        if req.selection != atoms.XdndSelection {
            let _ = send_selection_notify(&conn, &req, false);
            return;
        }
        // A request from our own selection window is a loop - reject it.
        if req.requestor == xdnd.selection_window {
            let _ = send_selection_notify(&conn, &req, false);
            return;
        }

        // This window owns XdndSelection only on behalf of a Wayland drag
        // source. An X11-sourced drag never makes the X11 source convert
        // against our window, so anything else here has nothing to serve.
        let source = match self.data.dnd.as_ref().and_then(|d| d.source.clone())
        {
            Some(crate::ddm::DndSource::Wayland(s)) => s,
            _ => {
                let _ = send_selection_notify(&conn, &req, false);
                return;
            }
        };

        // TARGETS: the list of types the source offers, as atoms.
        if req.target == atoms.TARGETS {
            let mimes = crate::ddm::data_source_mime(&source);
            let mut targets = vec![atoms.TARGETS, atoms.TIMESTAMP];
            targets.extend(
                mimes
                    .iter()
                    .filter_map(|m| atom_from_mime(&conn, &atoms, m)),
            );
            let ok = conn
                .change_property32(
                    PropMode::REPLACE,
                    req.requestor,
                    req.property,
                    AtomEnum::ATOM,
                    &targets,
                )
                .is_ok();
            let _ = send_selection_notify(&conn, &req, ok);
            return;
        }
        if req.target == atoms.TIMESTAMP {
            let ok = conn
                .change_property32(
                    PropMode::REPLACE,
                    req.requestor,
                    req.property,
                    AtomEnum::INTEGER,
                    &[CURRENT_TIME],
                )
                .is_ok();
            let _ = send_selection_notify(&conn, &req, ok);
            return;
        }

        // A concrete type: resolve it to a mime the source offers, then start
        // the pipe transfer.
        let Some(mime) = mime_from_atom(&conn, &atoms, req.target) else {
            let _ = send_selection_notify(&conn, &req, false);
            return;
        };
        if !crate::ddm::data_source_mime(&source).contains(&mime) {
            println!("[waylandcraft] XDND: requestor asked for unoffered type {}", mime);
            let _ = send_selection_notify(&conn, &req, false);
            return;
        }

        self.start_outgoing_transfer(req, &source, &mime);
    }

    // Open the pipe, hand its write end to the Wayland source for `mime`, and
    // register a calloop reader on the read end. Reading is driven by
    // read_outgoing; the requestor's property is filled in chunks.
    fn start_outgoing_transfer(
        &mut self,
        req: SelectionRequestEvent,
        source: &WlDataSource,
        mime: &str,
    ) {
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        let conn = Arc::clone(&xdnd.conn);
        let loop_handle = xdnd.loop_handle.clone();

        let (read_fd, write_fd) = match make_pipe() {
            Some(p) => p,
            None => {
                let _ = send_selection_notify(&conn, &req, false);
                return;
            }
        };

        // A stale transfer to the same requestor would hang the new one - X11
        // clients only read from the latest reply. Drop any prior transfer.
        if let Some(xdnd) = self.xdnd.as_mut()
            && let Some(stale) = xdnd.outgoing.remove(&req.requestor)
        {
            if let Some(token) = stale.token {
                loop_handle.remove(token);
            }
            let _ = send_selection_notify(&stale.conn, &stale.request, false);
        }

        let requestor = req.requestor;
        let token = loop_handle.insert_source(
            Generic::new(read_fd, Interest::READ, Mode::Level),
            move |_, fd, state| {
                state.read_outgoing(requestor, fd.as_fd());
                Ok(PostAction::Continue)
            },
        );
        let token = match token {
            Ok(token) => token,
            Err(err) => {
                println!("[waylandcraft] XDND: transfer source insert failed: {}", err.error);
                let _ = send_selection_notify(&conn, &req, false);
                return;
            }
        };

        let transfer = OutgoingTransfer {
            conn,
            token: Some(token),
            source_data: Vec::new(),
            request: req,
            incr: false,
            property_set: false,
            flush_property_on_delete: false,
            sent_finished: false,
        };
        let Some(xdnd) = self.xdnd.as_mut() else {
            loop_handle.remove(token);
            return;
        };
        xdnd.outgoing.insert(requestor, transfer);

        // Hand the write end to the Wayland source. It writes asynchronously;
        // the bytes surface on the read fd and drive read_outgoing.
        source.send(mime.to_string(), write_fd.as_fd());
    }

    // The read side of an outgoing transfer became readable: pull a chunk from
    // the Wayland source and either write it straight to the property (small
    // payload) or drive the INCR state machine (large payload). Ported from
    // smithay's read_selection_callback. The calloop source is removed by
    // dropping its token once the transfer completes.
    fn read_outgoing(&mut self, requestor: Window, fd: BorrowedFd<'_>) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        let Some(transfer) = xdnd.outgoing.get_mut(&requestor) else {
            return;
        };

        let mut buf = [0u8; INCR_CHUNK_SIZE];
        let len = match read_fd(fd, &mut buf) {
            ReadResult::Bytes(len) => len,
            ReadResult::WouldBlock => {
                // Spurious wakeup - nothing to read yet, the level-triggered
                // source fires again when the source writes more.
                return;
            }
            ReadResult::Error => {
                // Read side closed unexpectedly - fail the transfer.
                let _ = send_selection_notify(&transfer.conn, &transfer.request, false);
                finish_outgoing(xdnd, requestor);
                return;
            }
        };
        transfer.source_data.extend_from_slice(&buf[..len]);

        if transfer.source_data.len() >= INCR_CHUNK_SIZE {
            if !transfer.incr {
                // Payload exceeds one chunk: switch to an INCR transfer. The
                // owner must watch the requestor's property for the deletes
                // that pace the chunks, so select PropertyChange on it.
                let _ = transfer.conn.change_window_attributes(
                    transfer.request.requestor,
                    &ChangeWindowAttributesAux::new()
                        .event_mask(EventMask::PROPERTY_CHANGE),
                );
                if transfer
                    .conn
                    .change_property32(
                        PropMode::REPLACE,
                        transfer.request.requestor,
                        transfer.request.property,
                        xdnd.atoms.INCR,
                        &[INCR_CHUNK_SIZE as u32],
                    )
                    .and_then(|_| transfer.conn.flush())
                    .is_err()
                {
                    finish_outgoing(xdnd, requestor);
                    return;
                }
                transfer.incr = true;
                transfer.property_set = true;
                transfer.flush_property_on_delete = true;
                let _ = send_selection_notify(&transfer.conn, &transfer.request, true);
            } else if transfer.property_set {
                // A chunk is still in the property - buffer, flush on delete.
                transfer.flush_property_on_delete = true;
            } else if flush_outgoing_chunk(transfer).is_err() {
                finish_outgoing(xdnd, requestor);
                return;
            }
        }

        if len == 0 {
            if transfer.incr {
                // Source exhausted: flush whatever is left, then the
                // terminating 0-byte chunk is sent on the next property delete.
                if !transfer.property_set && flush_outgoing_chunk(transfer).is_err() {
                    finish_outgoing(xdnd, requestor);
                    return;
                }
                transfer.flush_property_on_delete = true;
                // Reading is done; the rest is driven by PropertyNotify.
                if let Some(token) = transfer.token.take() {
                    xdnd.loop_handle.remove(token);
                }
            } else {
                // Whole payload fit in one chunk - write it and finish.
                let ok = flush_outgoing_chunk(transfer).is_ok();
                let _ = send_selection_notify(&transfer.conn, &transfer.request, ok);
                finish_outgoing(xdnd, requestor);
            }
        }
    }

    // A PropertyNotify on the second connection. NEW_VALUE on the selection
    // window paces an incoming INCR transfer - the next chunk of the X11
    // source's data has landed. DELETE on a requestor property paces an
    // outgoing INCR transfer. Anything else is not ours.
    fn handle_property_notify(&mut self, n: PropertyNotifyEvent) {
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        if n.state == Property::NEW_VALUE
            && n.window == xdnd.selection_window
            && xdnd
                .incoming_transfers
                .get(&n.atom)
                .is_some_and(|t| t.incr)
        {
            self.read_incoming_chunk(n.atom);
            return;
        }
        if n.state != Property::DELETE {
            return;
        }
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        let is_incr_target = xdnd
            .outgoing
            .get(&n.window)
            .is_some_and(|t| t.incr && t.request.property == n.atom);
        if !is_incr_target {
            return;
        }

        let transfer = xdnd.outgoing.get_mut(&n.window).unwrap();
        transfer.property_set = false;
        if !transfer.flush_property_on_delete {
            return;
        }
        transfer.flush_property_on_delete = false;

        if flush_outgoing_chunk(transfer).is_err() {
            finish_outgoing(xdnd, n.window);
            return;
        }
        let remaining = transfer.source_data.len();

        // The reader source is gone once the source is fully drained. If bytes
        // remain, or the terminating chunk has not gone out, keep flushing on
        // the next delete; otherwise the transfer is complete.
        if transfer.token.is_none() {
            if remaining > 0 || !transfer.sent_finished {
                transfer.flush_property_on_delete = true;
            } else {
                finish_outgoing(xdnd, n.window);
            }
        }
    }
}

// --- X11 -> Wayland: WaylandCraft is the XDND target ----------------------
impl WLCState {
    // XFixesSelectionNotify on XdndSelection: the selection owner changed. An
    // owner that is not WaylandCraft's own selection window means an X11 app
    // started an XDND drag - map the drop proxy so the source addresses its
    // XDND messages here. Owner NONE (or our own window) ends/ignores it.
    fn handle_xfixes_selection_notify(
        &mut self,
        event: XfixesSelectionNotifyEvent,
    ) {
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        // WaylandCraft itself claimed the selection (a Wayland->X11 drag) -
        // not an inbound drag.
        if event.owner == xdnd.selection_window {
            return;
        }
        // The selection went away, or a different X11 app took it: tear down
        // any inbound drag in progress, both directions.
        if self.xdnd.as_ref().unwrap().incoming.is_some() {
            self.xdnd_target_teardown();
        }
        if event.owner == NONE {
            return;
        }

        // An X11 app owns XdndSelection: an inbound drag has begun. Map the
        // full-screen proxy above everything so the X11 source finds it as the
        // XdndAware window under its pointer and sends XDND messages here.
        let xdnd = self.xdnd.as_mut().unwrap();
        if let Err(err) = map_drop_proxy(xdnd) {
            println!("[waylandcraft] XDND: mapping drop proxy failed: {}", err);
            return;
        }
        xdnd.incoming = Some(XdndIncoming {
            source_window: event.owner,
            entered: false,
            dropped: false,
            last_timestamp: CURRENT_TIME,
        });
    }

    // XdndEnter to the drop proxy: the X11 source announced its offered types.
    // Read them (inline in the message, or from the XdndTypeList property) and
    // synthesize a Wayland drag (data.dnd) with an X11-backed DndSource so the
    // Wayland app under the cursor sees a normal wl_data_offer.
    fn handle_xdnd_enter(&mut self, data: [u32; 5]) {
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        if xdnd.incoming.is_none() {
            // XdndEnter without an inbound drag - no XFixes notify seen.
            return;
        }
        let source = data[0];
        // XdndEnter may carry a different source than XFixes reported (rare);
        // trust the message - it names the window XDND replies go to.
        let mimes = read_offered_mimes(&xdnd.conn, &xdnd.atoms, data, source);
        if mimes.is_empty() {
            println!("[waylandcraft] XDND: inbound drag offered no known types");
        }

        // The drag needs a Client for WLCDndEvent. An X11-sourced drag's client
        // field is only read by a guard that a present source disables, so any
        // live client works; a data device's client is always one.
        let Some(client) = self.data.devices.first().and_then(|d| d.client())
        else {
            // No Wayland client to deliver to at all - reject the drag.
            self.xdnd_target_teardown();
            return;
        };

        let actions = DndAction::Copy | DndAction::Move;
        let xdnd = self.xdnd.as_mut().unwrap();
        let incoming = xdnd.incoming.as_mut().unwrap();
        incoming.source_window = source;
        incoming.entered = true;

        // A drag already in flight (stale) is replaced - the new XdndEnter wins.
        self.data.dnd = Some(crate::ddm::WLCDndEvent {
            start_serial: new_serial(),
            request_sent: false,
            client,
            source: Some(crate::ddm::DndSource::X11 {
                mimes,
                actions,
                source_window: source,
            }),
            icon: None,
            focus: None,
            mime: None,
            action: DndAction::empty(),
            dropped: false,
        });
    }

    // XdndPosition from the X11 source: a heartbeat with the source's idea of
    // the cursor position. WaylandCraft's own in-world cursor (driven by the
    // X11DNDGrab) decides the Wayland target, so the position itself is unused
    // here - record the timestamp and reply XdndStatus with the current accept
    // state (whether the Wayland target took the offer, and the action).
    fn handle_xdnd_position(&mut self, data: [u32; 5]) {
        let accepted = self.data.dnd.as_ref().is_some_and(|d| d.mime.is_some());
        let action = self.data.dnd.as_ref().map(|d| d.action);

        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        let Some(incoming) = xdnd.incoming.as_mut() else {
            return;
        };
        if data[0] != incoming.source_window {
            return;
        }
        incoming.last_timestamp = data[3];
        let source = incoming.source_window;
        send_xdnd_status(xdnd, source, accepted, action);
    }

    // Re-send XdndStatus to an inbound drag's X11 source (routed from ddm.rs).
    // The Wayland target's accept/action is decided asynchronously, after the
    // XdndPosition that prompted the last XdndStatus; without this re-send a
    // source that stopped sending XdndPosition (cursor held still) would never
    // learn the drop became allowed. No-op unless an inbound drag is active.
    pub fn xdnd_target_refresh_status(&mut self) {
        let accepted = self.data.dnd.as_ref().is_some_and(|d| d.mime.is_some());
        let action = self.data.dnd.as_ref().map(|d| d.action);
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        let Some(incoming) = xdnd.incoming.as_ref() else {
            return;
        };
        // Before the source has entered the proxy there is no XDND interaction
        // to answer, and after a drop the status is frozen.
        if !incoming.entered || incoming.dropped {
            return;
        }
        let source = incoming.source_window;
        send_xdnd_status(self.xdnd.as_ref().unwrap(), source, accepted, action);
    }

    // XdndLeave from the X11 source: the drag pointer left the proxy. Cancel
    // the Wayland-side drag but keep the inbound interaction - the proxy stays
    // mapped and the source may re-enter; XFixes teardown ends it for good.
    fn handle_xdnd_leave(&mut self, data: [u32; 5]) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        let Some(incoming) = xdnd.incoming.as_mut() else {
            return;
        };
        if data[0] != incoming.source_window {
            return;
        }
        incoming.entered = false;
        self.data.dnd_cancel();
        self.data.dnd = None;
    }

    // XdndDrop from the X11 source: the user released the drag over the proxy.
    // Drive the Wayland target's drop. If no Wayland target took the offer,
    // finish the drag as failed at once; otherwise the Wayland target reads the
    // data and its Finish triggers XdndFinished (xdnd_target_finished).
    fn handle_xdnd_drop(&mut self, data: [u32; 5]) {
        let willing = {
            let Some(xdnd) = self.xdnd.as_mut() else {
                return;
            };
            let Some(incoming) = xdnd.incoming.as_mut() else {
                return;
            };
            if data[0] != incoming.source_window {
                return;
            }
            incoming.last_timestamp = data[2];
            incoming.dropped = true;
            self.data.dnd.as_ref().is_some_and(|d| {
                d.focus.is_some() && d.mime.is_some() && !d.action.is_empty()
            })
        };

        if !willing {
            // Nothing accepted the drop - tell the source it failed and end.
            self.xdnd_target_finish_failed();
            return;
        }
        // Hand the drop to the Wayland target. It consumes the data via Receive
        // and signals Finish; xdnd_target_finished then answers the X11 source.
        self.data.dnd_drop();
    }

    // SelectionNotify: the reply to a convert_selection for an inbound transfer.
    // property NONE means the conversion failed. An INCR-typed property starts a
    // chunked transfer; any other property holds the whole payload - write it to
    // the Wayland client's fd and finish.
    fn handle_xdnd_selection_notify(&mut self, n: SelectionNotifyEvent) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        // A failed conversion replies with property NONE; the transfer is still
        // keyed by the property we requested (_WL_SELECTION), so drop it by
        // that key. The fd closing gives the Wayland client EOF.
        if n.property == NONE {
            let key = xdnd.atoms._WL_SELECTION;
            xdnd.incoming_transfers.remove(&key);
            return;
        }
        let property = n.property;
        if !xdnd.incoming_transfers.contains_key(&property) {
            // A SelectionNotify we are not tracking - ignore.
            return;
        }
        let conn = Arc::clone(&xdnd.conn);
        let incr_atom = xdnd.atoms.INCR;
        let window = xdnd.selection_window;

        let reply = conn
            .get_property(true, window, property, AtomEnum::ANY, 0, PROP_READ_LEN)
            .ok()
            .and_then(|c| c.reply().ok());
        let Some(reply) = reply else {
            if let Some(xdnd) = self.xdnd.as_mut() {
                xdnd.incoming_transfers.remove(&property);
            }
            return;
        };

        if reply.type_ == incr_atom {
            // Large payload: the data arrives chunk by chunk. delete_property
            // above (delete=true) acked this notify; each further chunk lands
            // as a PropertyNotify NEW_VALUE and is drained by read_incoming.
            if let Some(xdnd) = self.xdnd.as_mut()
                && let Some(transfer) = xdnd.incoming_transfers.get_mut(&property)
            {
                transfer.incr = true;
            }
            return;
        }

        // Whole payload in one property: write it and finish.
        if let Some(xdnd) = self.xdnd.as_mut()
            && let Some(transfer) = xdnd.incoming_transfers.remove(&property)
        {
            write_all(transfer.fd.as_fd(), &reply.value);
        }
    }

    // One INCR chunk of an inbound transfer landed on the selection window's
    // property (PropertyNotify NEW_VALUE). Read it, append to the Wayland fd,
    // then delete the property to request the next chunk. An empty chunk is the
    // terminator - the transfer is complete and the fd is closed.
    fn read_incoming_chunk(&mut self, property: Atom) {
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        let conn = Arc::clone(&xdnd.conn);
        let window = xdnd.selection_window;

        // get_property with delete=true reads the chunk and deletes the
        // property in one request; reply() flushes it. The delete is the X11
        // source's signal to append the next chunk (a PropertyNotify follows).
        let reply = conn
            .get_property(true, window, property, AtomEnum::ANY, 0, PROP_READ_LEN)
            .ok()
            .and_then(|c| c.reply().ok());
        let Some(reply) = reply else {
            if let Some(xdnd) = self.xdnd.as_mut() {
                xdnd.incoming_transfers.remove(&property);
            }
            return;
        };

        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        if reply.value.is_empty() {
            // Terminating 0-byte chunk: transfer done, drop the fd (EOF).
            xdnd.incoming_transfers.remove(&property);
            return;
        }
        if let Some(transfer) = xdnd.incoming_transfers.get(&property) {
            write_all(transfer.fd.as_fd(), &reply.value);
        }
    }

    // wl_data_offer.Receive on an X11-sourced drag (routed from ddm.rs): the
    // Wayland target wants the data for `mime`. convert_selection the X11 source
    // onto WaylandCraft's selection window; the reply is delivered to `fd` by
    // handle_xdnd_selection_notify (INCR-aware). Mirrors the clipboard read.
    pub fn xdnd_target_receive(&mut self, mime: String, fd: OwnedFd) {
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        let Some(incoming) = xdnd.incoming.as_ref() else {
            return;
        };
        let Some(atom) = atom_from_mime(&xdnd.conn, &xdnd.atoms, &mime) else {
            println!("[waylandcraft] XDND: no atom for inbound mime {}", mime);
            return;
        };
        let conn = Arc::clone(&xdnd.conn);
        let property = xdnd.atoms._WL_SELECTION;
        let timestamp = incoming.last_timestamp;

        // A prior unfinished transfer to the same property would tangle with
        // this one - X11 has a single _WL_SELECTION property. Drop it; a drag
        // reads one type once, so this only guards against a misbehaving client.
        let xdnd = self.xdnd.as_mut().unwrap();
        xdnd.incoming_transfers.insert(property, IncomingTransfer {
            fd,
            incr: false,
        });

        // convert_selection(XdndSelection, ...) reaches the X11 drag source -
        // it owns that selection. The data is delivered into our selection
        // window's `property`; handle_xdnd_selection_notify pumps it to `fd`.
        if let Err(err) = conn
            .convert_selection(
                xdnd.selection_window,
                xdnd.atoms.XdndSelection,
                atom,
                property,
                timestamp,
            )
            .and_then(|_| conn.flush())
        {
            println!("[waylandcraft] XDND: convert_selection failed: {}", err);
            xdnd.incoming_transfers.remove(&property);
        }
    }

    // wl_data_offer.Finish on an X11-sourced drag (routed from ddm.rs): the
    // Wayland target finished consuming the drop. Tell the X11 source the drag
    // succeeded (XdndFinished with the performed action) and tear down.
    pub fn xdnd_target_finished(&mut self) {
        let action = self.data.dnd.as_ref().map(|d| d.action);
        let Some(xdnd) = self.xdnd.as_ref() else {
            return;
        };
        let Some(incoming) = xdnd.incoming.as_ref() else {
            return;
        };
        if !incoming.dropped {
            // Finish before the drop - ignore; XdndDrop drives completion.
            return;
        }
        let xdnd = self.xdnd.as_mut().unwrap();
        let incoming = xdnd.incoming.as_ref().unwrap();
        send_xdnd_finished(
            xdnd,
            incoming.source_window,
            true,
            action.unwrap_or(DndAction::Copy),
        );
        self.xdnd_target_teardown();
    }

    // The inbound drop was not accepted by any Wayland target: tell the X11
    // source it failed (XdndFinished with no action) and tear down.
    fn xdnd_target_finish_failed(&mut self) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        if let Some(incoming) = xdnd.incoming.as_ref() {
            let source = incoming.source_window;
            send_xdnd_finished(xdnd, source, false, DndAction::empty());
        }
        self.xdnd_target_teardown();
    }

    // Cancel an inbound X11->Wayland drag from the Wayland side (the in-world
    // X11DNDGrab was force-released). Reject it to the X11 source with a final
    // XdndStatus, drop the Wayland drag, and tear the interaction down.
    fn xdnd_target_cancel(&mut self) {
        if let Some(xdnd) = self.xdnd.as_mut()
            && let Some(incoming) = xdnd.incoming.as_ref()
        {
            let source = incoming.source_window;
            send_xdnd_status(xdnd, source, false, None);
        }
        self.data.dnd_cancel();
        self.data.dnd = None;
        self.xdnd_target_teardown();
    }

    // Tear down an inbound X11->Wayland drag: unmap the proxy, drop the inbound
    // state and any in-flight transfers, and clear the synthesized Wayland drag.
    // Safe to call with no inbound drag active. Used by XFixes teardown, drop
    // completion, cancellation, and X11 window destruction.
    pub fn xdnd_target_teardown(&mut self) {
        let Some(xdnd) = self.xdnd.as_mut() else {
            return;
        };
        if xdnd.incoming.take().is_none() {
            return;
        }
        xdnd.incoming_transfers.clear();
        let _ = xdnd.conn.unmap_window(xdnd.drop_proxy_window);
        let _ = xdnd.conn.flush();
        // Drop the synthesized Wayland drag if it is the inbound one.
        if matches!(
            self.data.dnd.as_ref().and_then(|d| d.source.as_ref()),
            Some(crate::ddm::DndSource::X11 { .. })
        ) {
            self.data.dnd_cancel();
            self.data.dnd = None;
        }
    }

    // True while an inbound X11->Wayland drag is active and a Wayland drag has
    // been synthesized for it. Polled by the JNI bridge so WaylandCraft can run
    // an in-world grab that ray-casts the cursor and drives dnd_motion.
    pub fn xdnd_target_active(&self) -> bool {
        self.xdnd
            .as_ref()
            .and_then(|x| x.incoming.as_ref())
            .is_some_and(|i| i.entered)
            && matches!(
                self.data.dnd.as_ref().and_then(|d| d.source.as_ref()),
                Some(crate::ddm::DndSource::X11 { .. })
            )
    }

    // An X11 window was destroyed (XwmHandler::destroyed_window). If it was a
    // party to an active XDND drag, cancel that drag cleanly: an inbound drag's
    // source going away tears the inbound interaction down; an outbound drag's
    // X11 target going away ends the Wayland drag that has been waiting for an
    // XdndFinished it will now never get.
    pub fn xdnd_window_destroyed(&mut self, window_id: Window) {
        let (is_incoming_source, is_outgoing_target) = {
            let Some(xdnd) = self.xdnd.as_ref() else {
                return;
            };
            let inc = xdnd
                .incoming
                .as_ref()
                .is_some_and(|i| i.source_window == window_id);
            let out = xdnd
                .source
                .as_ref()
                .and_then(|s| s.target.as_ref())
                .is_some_and(|t| t.window == window_id);
            (inc, out)
        };

        if is_incoming_source {
            // The X11 drag source crashed mid-drag - drop the inbound drag.
            self.xdnd_target_teardown();
        }
        if is_outgoing_target {
            // The X11 drag target crashed before sending XdndFinished - the
            // Wayland source would otherwise hang. Cancel it like the user
            // aborting: release the selection, tell the source it failed
            // (dnd_cancel), and drop the drag.
            self.xdnd_source_cancel();
            self.data.dnd_cancel();
            self.data.dnd = None;
        }
    }
}

// Send XdndEnter to a freshly entered X11 window and build its XdndTarget.
// Resolves the proxy and version, advertises up to 3 mime atoms inline (more
// go into the XdndTypeList property on the selection window). Returns None if
// the window does not advertise a usable XDND version.
fn enter_target(
    xdnd: &XdndState,
    window: Window,
    mimes: &[String],
) -> Option<XdndTarget> {
    let proxy = get_proxy_window(&xdnd.conn, &xdnd.atoms, window);
    let event_window = proxy.unwrap_or(window);

    let client_ver = get_xdnd_version(&xdnd.conn, &xdnd.atoms, event_window)?;
    if client_ver < MIN_XDND_VERSION {
        return None;
    }
    let version = client_ver.min(XDND_VERSION);

    let type_atoms: Vec<Atom> = mimes
        .iter()
        .filter_map(|m| atom_from_mime(&xdnd.conn, &xdnd.atoms, m))
        .collect();

    let mut data = [xdnd.selection_window, version << 24, NONE, NONE, NONE];
    for (i, atom) in type_atoms.iter().take(3).enumerate() {
        data[i + 2] = *atom;
    }
    if type_atoms.len() > 3 {
        // Bit 0 of data[1]: the full type list is in XdndTypeList instead.
        data[1] |= 1;
        if let Err(err) = xdnd.conn.change_property32(
            PropMode::REPLACE,
            xdnd.selection_window,
            xdnd.atoms.XdndTypeList,
            AtomEnum::ATOM,
            &type_atoms,
        ) {
            println!("[waylandcraft] XDND: setting XdndTypeList failed: {}", err);
        }
    } else {
        let _ = xdnd
            .conn
            .delete_property(xdnd.selection_window, xdnd.atoms.XdndTypeList);
    }

    xdnd.send_xdnd(window, event_window, xdnd.atoms.XdndEnter, data);
    Some(XdndTarget {
        window,
        event_window,
        accepted: false,
        action: DndAction::empty(),
    })
}

// Send XdndPosition to a target with the root-relative cursor position packed
// into data[2] (x in the high 16 bits, y in the low 16).
fn send_position(
    xdnd: &XdndState,
    target: &XdndTarget,
    root_x: i32,
    root_y: i32,
    action: DndAction,
) {
    let packed = (((root_x & 0xffff) << 16) | (root_y & 0xffff)) as u32;
    let data = [
        xdnd.selection_window,
        0,
        packed,
        CURRENT_TIME,
        source_action_atom(&xdnd.atoms, action, target.action),
    ];
    xdnd.send_xdnd(
        target.window,
        target.event_window,
        xdnd.atoms.XdndPosition,
        data,
    );
}

// Send XdndLeave to a target the drag has left.
fn send_leave(xdnd: &XdndState, target: &XdndTarget) {
    let data = [xdnd.selection_window, 0, 0, 0, 0];
    xdnd.send_xdnd(
        target.window,
        target.event_window,
        xdnd.atoms.XdndLeave,
        data,
    );
}

// XDND action atom -> DndAction. Inverse of action_to_atom; an unknown atom
// reads as no action.
fn action_from_atom(atoms: &XdndAtoms, atom: Atom) -> DndAction {
    if atom == atoms.XdndActionMove {
        DndAction::Move
    } else if atom == atoms.XdndActionCopy {
        DndAction::Copy
    } else if atom == atoms.XdndActionAsk {
        DndAction::Ask
    } else {
        DndAction::empty()
    }
}

// Write the next INCR_CHUNK_SIZE bytes (or the remaining tail, or an empty
// terminating chunk) of an outgoing transfer to the requestor's property.
// Ported from smithay's OutgoingTransfer::flush_data.
fn flush_outgoing_chunk(
    transfer: &mut OutgoingTransfer,
) -> Result<(), x11rb::errors::ConnectionError> {
    let len = transfer.source_data.len().min(INCR_CHUNK_SIZE);
    if len == 0 {
        transfer.sent_finished = true;
    }
    let chunk: Vec<u8> = transfer.source_data.drain(..len).collect();
    transfer.conn.change_property8(
        PropMode::REPLACE,
        transfer.request.requestor,
        transfer.request.property,
        transfer.request.target,
        &chunk,
    )?;
    transfer.conn.flush()?;
    transfer.property_set = true;
    Ok(())
}

// Drop an outgoing transfer: remove its calloop reader source (if still
// registered) and forget the transfer.
fn finish_outgoing(xdnd: &mut XdndState, requestor: Window) {
    if let Some(transfer) = xdnd.outgoing.remove(&requestor)
        && let Some(token) = transfer.token
    {
        xdnd.loop_handle.remove(token);
    }
}

// Send a SelectionNotify telling the requestor whether the transfer succeeded.
// On failure the property field is NONE, the X11 convention for "denied".
fn send_selection_notify(
    conn: &RustConnection,
    req: &SelectionRequestEvent,
    success: bool,
) -> Result<(), x11rb::errors::ConnectionError> {
    let event = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property: if success { req.property } else { NONE },
    };
    conn.send_event(false, req.requestor, EventMask::NO_EVENT, event)?;
    conn.flush()?;
    Ok(())
}

// Create a CLOEXEC | NONBLOCK pipe. The write end is handed to the Wayland
// source; the read end drives an outgoing transfer through calloop. Returns
// (read, write) or None if pipe2 fails.
fn make_pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    // SAFETY: pipe2 writes exactly two fds into the array on success; the fds
    // are wrapped in OwnedFd, which owns and closes them.
    let ret = unsafe {
        libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK)
    };
    if ret != 0 {
        return None;
    }
    // SAFETY: fds[0]/fds[1] are fresh, owned pipe ends from a successful pipe2.
    unsafe {
        Some((
            OwnedFd::from_raw_fd(fds[0]),
            OwnedFd::from_raw_fd(fds[1]),
        ))
    }
}

// Outcome of one read() on the transfer pipe. Bytes(0) is a genuine EOF (the
// Wayland source closed the write end); WouldBlock keeps EAGAIN distinct from
// EOF so a spurious wakeup is not mistaken for end-of-data.
enum ReadResult {
    Bytes(usize),
    WouldBlock,
    Error,
}

// Read from the non-blocking transfer pipe into `buf`.
fn read_fd(fd: BorrowedFd<'_>, buf: &mut [u8]) -> ReadResult {
    // SAFETY: read() fills at most buf.len() bytes into buf's backing storage.
    let ret = unsafe {
        libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len())
    };
    if ret >= 0 {
        return ReadResult::Bytes(ret as usize);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EAGAIN) | Some(libc::EINTR) => ReadResult::WouldBlock,
        _ => ReadResult::Error,
    }
}

// --- X11 -> Wayland target helpers ----------------------------------------

// Map the full-screen drop proxy and stack it above every other window, so the
// X11 drag source finds it as the topmost XdndAware window under its pointer
// and addresses its XDND ClientMessages here. Resizes it to the current screen
// first - the screen could have changed since the proxy was created.
fn map_drop_proxy(xdnd: &XdndState) -> Result<(), x11rb::errors::ConnectionError> {
    xdnd.conn.configure_window(
        xdnd.drop_proxy_window,
        &ConfigureWindowAux::new()
            .width(xdnd.screen.width_in_pixels as u32)
            .height(xdnd.screen.height_in_pixels as u32),
    )?;
    xdnd.conn.map_window(xdnd.drop_proxy_window)?;
    xdnd.conn.configure_window(
        xdnd.drop_proxy_window,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    )?;
    xdnd.conn.flush()?;
    Ok(())
}

// Read an inbound drag's offered types as mime strings. XDND v5 packs up to
// three type atoms inline in the XdndEnter message (data[2..5]); a fourth set
// bit in data[1] means the full list is in the source's XdndTypeList property.
// Unknown atoms are dropped. Mirrors smithay's handle_enter type extraction.
fn read_offered_mimes(
    conn: &RustConnection,
    atoms: &XdndAtoms,
    data: [u32; 5],
    source: Window,
) -> Vec<String> {
    if data[1] & 1 == 0 {
        return data[2..5]
            .iter()
            .filter(|&&a| a != NONE)
            .filter_map(|&a| mime_from_atom(conn, atoms, a))
            .collect();
    }
    let reply = conn
        .get_property(
            false,
            source,
            atoms.XdndTypeList,
            AtomEnum::ATOM,
            0,
            PROP_READ_LEN,
        )
        .ok()
        .and_then(|c| c.reply().ok());
    let Some(reply) = reply else {
        return Vec::new();
    };
    reply
        .value32()
        .into_iter()
        .flatten()
        .filter_map(|a| mime_from_atom(conn, atoms, a))
        .collect()
}

// Send XdndStatus to an inbound drag's X11 source: whether a Wayland target
// accepts a drop here (data[1] bit 0) and the action it would take (data[4]).
// An accepted drag with no action falls back to copy; a rejected one sends the
// NONE action atom, which the source reads as "do not drop here".
fn send_xdnd_status(
    xdnd: &XdndState,
    source: Window,
    accepted: bool,
    action: Option<DndAction>,
) {
    let action = action.unwrap_or(DndAction::empty());
    let accept = accepted && !action.is_empty();
    // data[1] bit 0: accept. bit 1 (always set here): send XdndPosition for
    // every motion - WaylandCraft does not use the rectangle suppression hint.
    let flags = if accept { 0b11 } else { 0b10 };
    let action_atom = if accept {
        action_to_atom(&xdnd.atoms, action)
    } else {
        NONE
    };
    let data = [
        xdnd.drop_proxy_window,
        flags,
        0,
        0,
        action_atom,
    ];
    xdnd.send_to_source(source, xdnd.atoms.XdndStatus, data);
}

// Send XdndFinished to an inbound drag's X11 source: the drag is over. `success`
// (data[1] bit 0, XDND v5) tells the source whether the drop was consumed;
// data[2] carries the performed action. A failed drop sends no action.
fn send_xdnd_finished(
    xdnd: &XdndState,
    source: Window,
    success: bool,
    action: DndAction,
) {
    let data = [
        xdnd.drop_proxy_window,
        if success { 1 } else { 0 },
        if success {
            action_to_atom(&xdnd.atoms, action)
        } else {
            NONE
        },
        0,
        0,
    ];
    xdnd.send_to_source(source, xdnd.atoms.XdndFinished, data);
}

// Write a full buffer to a (possibly blocking) pipe fd, retrying short writes
// and EINTR. The Wayland client's read end is the other side; a closed pipe or
// hard error ends the write - the transfer is best-effort, never panics.
fn write_all(fd: BorrowedFd<'_>, mut buf: &[u8]) {
    while !buf.is_empty() {
        // SAFETY: write() reads at most buf.len() bytes from buf's storage.
        let ret = unsafe {
            libc::write(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len())
        };
        if ret > 0 {
            buf = &buf[ret as usize..];
            continue;
        }
        if ret == 0 {
            return;
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EINTR) => continue,
            _ => return,
        }
    }
}
