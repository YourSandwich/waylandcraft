use crate::WLCState;
use crate::bridge::WlcWindow;
use crate::ddm::ClipboardSource;
use smithay::{
    utils::{Logical, Rectangle},
    wayland::selection::SelectionTarget,
    xwayland::{
        X11Surface, X11Wm, XwmHandler,
        xwm::{Reorder, ResizeEdge, WmInputModel, WmWindowType, X11Window, XwmId},
    },
};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    ClientMessageEvent, ConnectionExt as _, EventMask, InputFocus,
};
use x11rb::rust_connection::RustConnection;

// A write-only X11 client connection used solely to set the X server's input
// focus. WaylandCraft routes keyboard focus through its own WLCSeatState, never
// through a smithay Seat, so smithay's X11Surface KeyboardTarget impl - the only
// thing that calls XSetInputFocus - never runs. Without an XSetInputFocus call
// an X11 window never receives FocusIn, so Wine reports it as not the foreground
// window and apps that throttle background work while unfocused (osu! pausing
// map loading) stay paused. This connection issues that call directly, the same
// way xdnd.rs runs its own X11 client alongside smithay's WM connection. It is
// write-only: the WM connection already reads FocusIn/FocusOut and keeps
// _NET_ACTIVE_WINDOW in sync, so no event source is needed here.
pub struct X11FocusConn {
    conn: Arc<RustConnection>,
    wm_protocols: u32,
    wm_take_focus: u32,
}

impl X11FocusConn {
    pub fn create(display: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let (conn, _) = RustConnection::connect(Some(&format!(":{display}")))?;
        let wm_protocols = conn.intern_atom(false, b"WM_PROTOCOLS")?.reply()?.atom;
        let wm_take_focus =
            conn.intern_atom(false, b"WM_TAKE_FOCUS")?.reply()?.atom;
        Ok(X11FocusConn {
            conn: Arc::new(conn),
            wm_protocols,
            wm_take_focus,
        })
    }

    // Make `window` the X server's input focus, honoring its ICCCM input model
    // exactly as smithay's X11Surface KeyboardTarget::enter does: Passive and
    // LocallyActive windows get XSetInputFocus, LocallyActive and GloballyActive
    // also get a WM_TAKE_FOCUS message, None-model windows get nothing.
    fn focus(&self, window: &X11Surface) {
        let (set_input_focus, send_take_focus) = match window.input_model() {
            WmInputModel::None => return,
            WmInputModel::Passive => (true, false),
            WmInputModel::LocallyActive => (true, true),
            WmInputModel::GloballyActive => (false, true),
        };
        let id = window.window_id();
        if set_input_focus {
            let _ = self.conn.set_input_focus(
                InputFocus::NONE,
                id,
                x11rb::CURRENT_TIME,
            );
        }
        if send_take_focus {
            let event = ClientMessageEvent::new(
                32,
                id,
                self.wm_protocols,
                [self.wm_take_focus, x11rb::CURRENT_TIME, 0, 0, 0],
            );
            let _ =
                self.conn.send_event(false, id, EventMask::NO_EVENT, event);
        }
        let _ = self.conn.flush();
    }

    // Drop the X server's input focus. The previously focused X11 window then
    // sees FocusOut and an unfocus-throttling app pauses, which is correct when
    // WaylandCraft's keyboard focus has left every X11 window.
    fn unfocus(&self) {
        let _ = self.conn.set_input_focus(
            InputFocus::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        );
        let _ = self.conn.flush();
    }
}

// An X11 window belongs on the popup path if it is override-redirect or its
// window type is a menu/tooltip/dropdown - Ardour's tooltips are plain non-OR
// windows and would otherwise pollute the toplevel list.
fn is_popup_like(window: &X11Surface) -> bool {
    window.is_override_redirect()
        || matches!(
            window.window_type(),
            Some(
                WmWindowType::Tooltip
                    | WmWindowType::Menu
                    | WmWindowType::DropdownMenu
                    | WmWindowType::PopupMenu
            )
        )
}

// True if an X11 window is one of WaylandCraft's own XDND helper windows (the
// selection window or the drop proxy, see xdnd.rs). They live on the second X11
// connection but smithay's WM still sees them map; they must never enter the
// tracked window lists or WaylandCraft would render an invisible helper as a
// window.
fn is_xdnd_helper_window(state: &WLCState, id: u32) -> bool {
    state
        .xdnd
        .as_ref()
        .is_some_and(|x| id == x.selection_window || id == x.drop_proxy_window)
}

// Track an X11 window if it is not already tracked, routing it to the popup
// list or the toplevel list by is_popup_like. Dedup keys on the stable X11
// window id, never X11Surface equality. WaylandCraft's own XDND helper windows
// are skipped - they are not application windows.
fn track_x11_window(state: &mut WLCState, window: X11Surface) {
    let id = window.window_id();
    if is_xdnd_helper_window(state, id) {
        return;
    }
    if state.x11_windows.iter().any(|w| w.window_id() == id)
        || state.x11_override_windows.iter().any(|w| w.window_id() == id)
    {
        return;
    }
    if is_popup_like(&window) {
        state.x11_override_windows.push(window);
    } else {
        state.x11_windows.push(window);
    }
}

// Sync X11 activation and stacking with WaylandCraft's keyboard focus. `focused`
// is the window WaylandCraft just gave keyboard focus, or None. For an X11
// window, set_activated lets the client reflect focus in its UI and raise_window
// keeps the X server's stacking order in step with focus order, so X11 menus and
// child windows do not fall behind their parent. xdg toplevels carry their own
// Activated state and are handled separately; they only matter here to drop the
// previously focused X11 window's activation when focus moves to an xdg window.
// Only ever called with toplevels - x11_windows excludes override-redirect and
// menu-type windows, which is_popup_like routes to the popup list instead.
pub(crate) fn sync_x11_focus(state: &mut WLCState, focused: Option<&WlcWindow>) {
    let new = match focused {
        Some(WlcWindow::X11(w)) => Some(w),
        _ => None,
    };
    // Identity by stable X11 window id - X11Surface equality folds in geometry.
    if new.map(|w| w.window_id())
        == state.focused_x11.as_ref().map(|w| w.window_id())
    {
        return;
    }

    let had_x11 = state.focused_x11.is_some();
    if let Some(old) = state.focused_x11.take() {
        let _ = old.set_activated(false);
    }

    if let Some(window) = new {
        let _ = window.set_activated(true);
        if let Some(xwm) = state.xwm.as_mut() {
            let _ = xwm.raise_window(window);
        }
        // Hand the X server's input focus to this window so Wine sees it as the
        // foreground window - set_activated only sets _NET_WM_STATE_FOCUSED,
        // which does not generate the FocusIn the client reacts to.
        if let Some(focus_conn) = state.x11_focus.as_ref() {
            focus_conn.focus(window);
        }
        state.focused_x11 = Some(window.clone());
    } else if had_x11 {
        // Focus moved off X11 entirely (to an xdg window or to nothing) - drop
        // the X server's input focus so the old X11 window sees FocusOut.
        if let Some(focus_conn) = state.x11_focus.as_ref() {
            focus_conn.unfocus();
        }
    }
}

// Mirror WaylandCraft's Wayland clipboard onto the X11 selection so X11
// (Xwayland) apps can paste what a Wayland app copied. Called when the Wayland
// selection changes (ddm.rs SetSelection). A Wayland-owned selection claims the
// X11 selection with its mime types; an empty selection releases it. An
// X11-owned clipboard is left alone - X11 already owns it, reclaiming would
// loop. send_selection() then services the actual X11 read from the Wayland
// source.
pub(crate) fn bridge_wayland_selection_to_x11(state: &mut WLCState) {
    let mime = match state.data.clipboard() {
        Some(ClipboardSource::X11 { .. }) => return,
        Some(src) => Some(src.mime()),
        None => None,
    };
    let Some(xwm) = state.xwm.as_mut() else {
        return;
    };
    let _ = xwm.new_selection(SelectionTarget::Clipboard, mime);
}

// XwmHandler - the X11 window manager. WaylandCraft has no smithay Space/Window,
// so the callbacks carry no placement. An X11 window is tracked from its first
// map until destroyed_window; unmap leaves it tracked so its Java window and
// framebuffer survive the unmap/remap churn behind the subwindow flicker.
impl XwmHandler for WLCState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {
        // The window enters the model at map time, not here.
    }

    fn new_override_redirect_window(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
    ) {
        // The window enters the model at map time, not here - smithay can
        // still flip the override-redirect flag before MapNotify.
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        window.set_mapped(true).unwrap();
        // Send the post-map configure the X11 client expects (ICCCM). Without it
        // GTK clients retry-loop, unmapping and remapping - which churns the model.
        let _ = window.configure(None);
        track_x11_window(self, window);
    }

    fn mapped_override_redirect_window(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
    ) {
        // Override-redirect windows error on set_mapped/configure - just track.
        track_x11_window(self, window);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // The window stays tracked - it is removed only at destroyed_window.
        // set_mapped(false) drops its backing surface, so isMapped() goes
        // false on the Java side and the UI hides it without churning it.
        if !window.is_override_redirect() {
            window.set_mapped(false).unwrap();
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Removal keys on the stable X11 window id, never X11Surface equality.
        let id = window.window_id();
        self.x11_windows.retain(|w| w.window_id() != id);
        self.x11_override_windows.retain(|w| w.window_id() != id);
        if self.focused_x11.as_ref().is_some_and(|w| w.window_id() == id)
        {
            self.focused_x11 = None;
        }
        // If this window was a party to an XDND drag, cancel that drag - an
        // inbound drag's source, or an outbound drag's target, crashing
        // mid-drag must not leave either side hung. See xdnd.rs.
        self.xdnd_window_destroyed(id);
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        // The X server connection ended - drop the WM and the tracked windows
        // so their Java objects and framebuffers are released cleanly.
        self.xwm = None;
        // The X11 focus connection died with the X server too.
        self.x11_focus = None;
        // An XDND drag touching X11 cannot survive the X server: an inbound
        // X11-sourced drag is over, and an outbound Wayland->X11 drag would
        // hang waiting for an XdndFinished it can never get. Cancel either,
        // but leave a pure Wayland<->Wayland drag alone.
        let inbound_x11 = matches!(
            self.data.dnd.as_ref().and_then(|d| d.source.as_ref()),
            Some(crate::ddm::DndSource::X11 { .. })
        );
        let outbound_to_x11 = self
            .xdnd
            .as_ref()
            .is_some_and(|x| x.has_outgoing_target());
        if inbound_x11 || outbound_to_x11 {
            self.data.dnd_cancel();
            self.data.dnd = None;
        }
        // Tear down the XDND foundation alongside the WM - the second X11
        // connection died with the X server too.
        if let Some(xdnd) = self.xdnd.take() {
            xdnd.destroy();
        }
        self.x11_windows.clear();
        self.x11_override_windows.clear();
        self.focused_x11 = None;
        // An X11-owned clipboard selection died with the X server - drop it so
        // Wayland clients stop being offered a selection that can't be read.
        if matches!(self.data.clipboard, Some(ClipboardSource::X11 { .. })) {
            self.data.set_clipboard(None);
        }
    }

    // --- X11 <-> Wayland clipboard bridge -----------------------------------
    // The X11 side of the bridge. X11Wm owns the X11 selection plumbing; these
    // callbacks connect it to WaylandCraft's Wayland clipboard (ddm.rs). The
    // Wayland->X11 direction lives in bridge_wayland_selection_to_x11 above.
    // Only CLIPBOARD is bridged: WaylandCraft implements no Wayland primary
    // selection protocol, so SelectionTarget::Primary has nothing to bind to.

    // An X11 app requests read access to a selection (it wants to paste). Allow
    // it only while an X11 window holds keyboard focus - matches Anvil's gate.
    fn allow_selection_access(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
    ) -> bool {
        matches!(selection, SelectionTarget::Clipboard)
            && self.focused_x11.is_some()
    }

    // An X11 app pastes: write the current Wayland clipboard for `mime_type`
    // into `fd`. Only a Wayland-owned selection can be served here - an
    // X11-owned clipboard is X11's own data and smithay services X11->X11
    // transfers internally.
    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        if !matches!(selection, SelectionTarget::Clipboard) {
            return;
        }
        // This fires for the X11 selection WaylandCraft's proxy owns, which it
        // claims only on behalf of a Wayland selection - so the source is
        // normally Wayland. Forward the read to that source's client; for any
        // other state there is nothing to serve and the fd closes (EOF).
        if let Some(source) = self.data.clipboard()
            && let Some(wl) = source.wayland()
            && source.mime().contains(&mime_type)
        {
            wl.send(mime_type, fd.as_fd());
        }
    }

    // An X11 app copies: its offered mime types become the Wayland clipboard
    // selection, so Wayland apps can paste. send_selection (the other handler)
    // then services each Wayland read by writing the X11 data to the fd.
    fn new_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_types: Vec<String>,
    ) {
        if !matches!(selection, SelectionTarget::Clipboard) {
            return;
        }
        self.data
            .set_clipboard(Some(ClipboardSource::X11 { mime: mime_types }));
    }

    // An X11 app's selection was cleared. Drop the Wayland clipboard only if it
    // still holds that X11 selection - a Wayland app may have copied since.
    fn cleared_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
    ) {
        if !matches!(selection, SelectionTarget::Clipboard) {
            return;
        }
        if matches!(self.data.clipboard, Some(ClipboardSource::X11 { .. })) {
            self.data.set_clipboard(None);
        }
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        if window.is_override_redirect() {
            return;
        }
        // Honor only the requested size - WaylandCraft renders X11 windows as
        // in-world surfaces, so a client-requested screen position is
        // meaningless. Keep the current loc, drop x/y (matches Anvil).
        let mut geo = window.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = window.configure(Some(geo));
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        // X11Surface stores its own geometry; nothing extra to keep in sync.
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _resize_edge: ResizeEdge,
    ) {
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {
    }
}
