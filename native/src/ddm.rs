use crate::WLCState;
use crate::utils::{get_time, new_serial, to_fixed2};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::ClientId,
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager as wl_ddm,
        wl_data_device_manager::DndAction,
        wl_data_device_manager::WlDataDeviceManager as WlDDM,
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_surface::WlSurface,
    },
};
use smithay::wayland::selection::SelectionTarget;
use std::ops::DerefMut;
use std::os::fd::AsFd;
use std::sync::{Arc, Mutex};

// The clipboard selection's data provider. A Wayland client owns its selection
// through a WlDataSource; an X11 (Xwayland) client has no Wayland source, so its
// selection is represented by the mime types it offered, with reads serviced by
// X11Wm::send_selection. See the X11<->Wayland clipboard bridge in xwayland.rs.
#[derive(Clone)]
pub enum ClipboardSource {
    Wayland(WlDataSource),
    X11 { mime: Vec<String> },
}

impl ClipboardSource {
    pub fn mime(&self) -> Vec<String> {
        match self {
            ClipboardSource::Wayland(s) => {
                with_source_data(s, |d| d.mime.clone())
            }
            ClipboardSource::X11 { mime } => mime.clone(),
        }
    }

    // A Wayland source is gone once its WlDataSource dies; an X11 selection has
    // no resource to outlive and is dropped explicitly by the bridge.
    fn is_alive(&self) -> bool {
        match self {
            ClipboardSource::Wayland(s) => s.is_alive(),
            ClipboardSource::X11 { .. } => true,
        }
    }

    pub fn wayland(&self) -> Option<&WlDataSource> {
        match self {
            ClipboardSource::Wayland(s) => Some(s),
            ClipboardSource::X11 { .. } => None,
        }
    }
}

// A drag's data provider. A Wayland-initiated drag carries a WlDataSource; an
// X11-initiated drag (Xwayland app dragging onto a Wayland app) has no Wayland
// source, so it is represented by the offered mime types, the drag actions, and
// the X11 source window. Reads of an X11 drag are serviced by the XDND bridge
// (xdnd.rs) via convert_selection. Mirrors ClipboardSource for the DnD path.
#[derive(Clone)]
pub enum DndSource {
    Wayland(WlDataSource),
    X11 {
        mimes: Vec<String>,
        actions: DndAction,
        source_window: u32,
    },
}

impl DndSource {
    pub fn mime(&self) -> Vec<String> {
        match self {
            DndSource::Wayland(s) => with_source_data(s, |d| d.mime.clone()),
            DndSource::X11 { mimes, .. } => mimes.clone(),
        }
    }

    pub fn actions(&self) -> DndAction {
        match self {
            DndSource::Wayland(s) => with_source_data(s, |d| d.actions),
            DndSource::X11 { actions, .. } => *actions,
        }
    }

    // A Wayland source dies with its WlDataSource; an X11 drag has no resource
    // to outlive and is dropped explicitly by the XDND bridge.
    fn is_alive(&self) -> bool {
        match self {
            DndSource::Wayland(s) => s.is_alive(),
            DndSource::X11 { .. } => true,
        }
    }

    pub fn wayland(&self) -> Option<&WlDataSource> {
        match self {
            DndSource::Wayland(s) => Some(s),
            DndSource::X11 { .. } => None,
        }
    }

    // Tell the source the negotiated mime type (Wayland sources only - an X11
    // drag's accepted type is tracked in WLCDndEvent.mime, the X11 source is not
    // a Wayland resource and has no target() request).
    fn target(&self, mime: Option<String>) {
        if let DndSource::Wayland(s) = self {
            s.target(mime);
        }
    }

    // Tell the source the chosen action (Wayland sources only).
    pub fn action(&self, action: DndAction) {
        if let DndSource::Wayland(s) = self {
            s.action(action);
        }
    }

    // Signal the drop was performed (Wayland sources only).
    pub fn dnd_drop_performed(&self) {
        if let DndSource::Wayland(s) = self {
            s.dnd_drop_performed();
        }
    }

    // Signal the target finished consuming the drop (Wayland sources only).
    pub fn dnd_finished(&self) {
        if let DndSource::Wayland(s) = self {
            s.dnd_finished();
        }
    }

    // Signal the drag was cancelled (Wayland sources only).
    fn cancelled(&self) {
        if let DndSource::Wayland(s) = self {
            s.cancelled();
        }
    }
}

pub struct WLCDataState {
    pub devices: Vec<WlDataDevice>,
    pub clipboard: Option<ClipboardSource>,
    pub clipboard_focus: Option<Client>,
    pub dnd: Option<WLCDndEvent>,
    display_handle: DisplayHandle,
}

// Drag and drop session
// `dropped` is set when the user successfully performed a drop over a surface
pub struct WLCDndEvent {
    pub start_serial: u32,
    pub request_sent: bool,
    // The client that started the drag. For an X11-sourced drag there is no
    // Wayland drag client; the Xwayland client is used so the cross-client
    // suppression in dnd_motion treats every Wayland surface as a valid target.
    pub client: Client,
    pub source: Option<DndSource>,
    pub icon: Option<WlSurface>,
    pub focus: Option<WlSurface>,
    pub mime: Option<String>,
    pub action: DndAction,
    pub dropped: bool,
}

#[derive(Debug, PartialEq)]
enum SourceUsage {
    Unused,
    Selection,
    Drag,
}

type WLCDataSource = Arc<Mutex<WLCDataSourceData>>;
struct WLCDataSourceData {
    usage: SourceUsage,
    mime: Vec<String>,
    actions: DndAction,
}

// A wl_data_offer backs either a clipboard selection or a drag. The two carry
// different source types (ClipboardSource has no actions; DndSource does), so
// the offer keeps whichever applies. Both can be Wayland- or X11-backed.
enum OfferSource {
    Clipboard(ClipboardSource),
    Dnd(DndSource),
}

impl OfferSource {
    fn mime(&self) -> Vec<String> {
        match self {
            OfferSource::Clipboard(s) => s.mime(),
            OfferSource::Dnd(s) => s.mime(),
        }
    }

    fn is_alive(&self) -> bool {
        match self {
            OfferSource::Clipboard(s) => s.is_alive(),
            OfferSource::Dnd(s) => s.is_alive(),
        }
    }
}

type WLCDataOffer = Arc<Mutex<WLCDataOfferData>>;
struct WLCDataOfferData {
    // A clipboard offer or a drag offer, each either Wayland- or X11-backed.
    source: OfferSource,
    device: WlDataDevice,
}

impl WLCDataOfferData {
    // The Wayland data source behind a DnD offer, if it is Wayland-backed. An
    // X11 drag and every clipboard offer return None - those go through their
    // own bridge paths and have no WlDataSource to drive directly.
    fn wl_source(&self) -> Option<&WlDataSource> {
        match &self.source {
            OfferSource::Dnd(DndSource::Wayland(s)) => Some(s),
            _ => None,
        }
    }
}

type WLCDataDevice = Arc<Mutex<WLCDataDeviceData>>;
struct WLCDataDeviceData {
    // Device focus (see enter, leave)
    dnd_focus: Option<WlSurface>,
    last_dnd_motion: Option<(i32, i32)>,
    // Currently active data offer. May be present even when no dnd_focus set
    dnd_offer: Option<WlDataOffer>,
}

fn with_source_data<F, R>(source: &WlDataSource, f: F) -> R
where
    F: FnOnce(&mut WLCDataSourceData) -> R,
{
    let mut guard = source.data::<WLCDataSource>().unwrap().lock().unwrap();
    let data = guard.deref_mut();
    f(data)
}

// The mime types a Wayland data source offers. Used by the XDND source bridge
// (xdnd.rs) to advertise and serve a Wayland drag's types to an X11 target.
pub fn data_source_mime(source: &WlDataSource) -> Vec<String> {
    with_source_data(source, |d| d.mime.clone())
}

fn with_offer_data<F, R>(offer: &WlDataOffer, f: F) -> R
where
    F: FnOnce(&mut WLCDataOfferData) -> R,
{
    let mut guard = offer.data::<WLCDataOffer>().unwrap().lock().unwrap();
    let data = guard.deref_mut();
    f(data)
}

fn with_device_data<F, R>(device: &WlDataDevice, f: F) -> R
where
    F: FnOnce(&mut WLCDataDeviceData) -> R,
{
    let mut guard = device.data::<WLCDataDevice>().unwrap().lock().unwrap();
    let data = guard.deref_mut();
    f(data)
}

impl WLCDataState {
    pub fn new(display_handle: &DisplayHandle) -> Self {
        WLCDataState {
            devices: vec![],
            clipboard: None,
            clipboard_focus: None,
            dnd: None,
            display_handle: display_handle.clone(),
        }
    }

    pub fn create_global(&self) {
        self.display_handle
            .create_global::<WLCState, WlDDM, ()>(3, ());
    }

    // Replace the clipboard selection: cancel the outgoing Wayland source (if
    // any), store the new one, and re-offer to the focused client. Shared by
    // the Wayland SetSelection path and the X11 clipboard bridge.
    pub fn set_clipboard(&mut self, source: Option<ClipboardSource>) {
        if let Some(ClipboardSource::Wayland(old)) = &self.clipboard {
            old.cancelled();
        }
        self.clipboard = source;
        self.send_clipboard();
    }

    // The current clipboard selection, dropping it first if its source died.
    pub fn clipboard(&mut self) -> Option<&ClipboardSource> {
        if self.clipboard.as_ref().is_some_and(|c| !c.is_alive()) {
            self.clipboard = None;
        }
        self.clipboard.as_ref()
    }

    pub fn update_clipboard_client(&mut self, client: Option<Client>) {
        if self.clipboard_focus != client {
            self.clipboard_focus = client;
            self.send_clipboard();
        }
    }

    // Send clipboard data to client with clipboard focus
    fn send_clipboard(&self) {
        let client = match &self.clipboard_focus {
            Some(c) => c,
            None => {
                return;
            }
        };
        for device in &self.devices {
            if !device.client().is_some_and(|c| c == *client) {
                continue;
            }

            if let Some(clipboard) = &self.clipboard {
                let offer_data = WLCDataOfferData {
                    source: OfferSource::Clipboard(clipboard.clone()),
                    device: device.clone(),
                };
                let offer_data = Arc::new(Mutex::new(offer_data));
                let offer = client
                    .create_resource::<WlDataOffer, WLCDataOffer, WLCState>(
                        &self.display_handle,
                        device.version(),
                        offer_data,
                    )
                    .unwrap();

                device.data_offer(&offer);
                for m in clipboard.mime() {
                    offer.offer(m);
                }
                device.selection(Some(&offer));
            } else {
                device.selection(None);
            }
        }
    }

    fn print_dnd_debug(&self, header: &str) {
        /*
        println!("\n{}", header);
        print!("DND: ");
        if self.dnd.is_none() {
            println!("NONE");
            return;
        }
        println!();
        let dnd = self.dnd.as_ref().unwrap();
        println!("\tsource: {:?}", dnd.source);
        println!("\tfocus: {:?}", dnd.focus);
        println!("\tmime: {:?}", dnd.mime);
        println!("\taction: {:?}", dnd.action);
        println!("\tdropped: {:?}", dnd.dropped);
        */
        let _ = header;
    }

    // The serial of a freshly started Wayland drag, once, for the JNI bridge to
    // match against an implicit pointer grab. An X11-sourced drag (Stage C) is
    // not a Wayland start_drag - it has no implicit grab to match and is driven
    // by its own poll path, so it is never reported here.
    pub fn check_dnd_request(&mut self) -> Option<u32> {
        let dnd = self.dnd.as_mut()?;
        if dnd.request_sent {
            return None;
        }
        if matches!(dnd.source, Some(DndSource::X11 { .. })) {
            return None;
        }
        dnd.request_sent = true;
        Some(dnd.start_serial)
    }

    fn dnd_send_offer(
        &self,
        source: &DndSource,
        device: &WlDataDevice,
        data: &mut WLCDataDeviceData,
    ) -> WlDataOffer {
        let device_client = device.client().unwrap();

        // Create offer
        let offer_data = WLCDataOfferData {
            source: OfferSource::Dnd(source.clone()),
            device: device.clone(),
        };
        let offer_data = Arc::new(Mutex::new(offer_data));
        let offer = device_client
            .create_resource::<WlDataOffer, WLCDataOffer, WLCState>(
                &self.display_handle,
                device.version(),
                offer_data,
            )
            .unwrap();
        data.dnd_offer = Some(offer.clone());

        // Send offer to client
        device.data_offer(&offer);
        for m in source.mime() {
            offer.offer(m);
        }
        offer.source_actions(source.actions());

        offer
    }

    pub fn dnd_motion(
        &mut self,
        mut surface: Option<WlSurface>,
        x: f64,
        y: f64,
    ) {
        self.print_dnd_debug("dnd motion");

        if self.dnd.is_none() {
            return;
        }
        if self.dnd.as_ref().unwrap().dropped {
            return;
        }

        let client = self.dnd.as_ref().unwrap().client.clone();
        let source = self.dnd.as_ref().unwrap().source.clone();
        let focus = self.dnd.as_ref().unwrap().focus.clone();

        if source.is_none()
            && let Some(ref s) = surface
        {
            let surface_client = s.client().unwrap();
            if surface_client != client {
                // Non-source drag and focus is on a different client
                surface = None;
            }
        }

        if surface != focus {
            // Reset the accepted type when moving to different surface
            if let Some(s) = &source {
                s.target(None);
            }
            self.dnd.as_mut().unwrap().mime = None;
        }
        self.dnd.as_mut().unwrap().focus = surface.clone();

        // Unfocus devices focused on wrong surface
        self.for_all_devices(|device, data| {
            let focus = match &data.dnd_focus {
                Some(s) => s,
                None => return,
            };
            let unfocus = match &surface {
                Some(s) => s != focus,
                None => true,
            };
            if unfocus {
                device.leave();
                data.dnd_focus = None;
            }
        });

        let surface = match surface {
            Some(s) => s,
            None => return,
        };

        // Send device enter events
        self.for_all_devices(|device, data| {
            // Already focused
            if data.dnd_focus.is_some() {
                return;
            }

            // Check if client does not own surface
            let surface_client = surface.client().unwrap();
            let device_client = device.client().unwrap();
            if surface_client != device_client {
                return;
            }

            let mut offer = None;
            if let Some(ref s) = source {
                offer = Some(self.dnd_send_offer(s, device, data));
            }

            // Make device enter surface
            device.enter(new_serial(), &surface, x, y, offer.as_ref());
            data.dnd_focus = Some(surface.clone());
            data.last_dnd_motion = None;
        });

        let time = get_time();
        let pos: (i32, i32) = to_fixed2(x, y);

        // Send device motion events
        self.for_all_devices(|device, data| {
            if data.dnd_focus.is_none() {
                return;
            }
            if data.last_dnd_motion == Some(pos) {
                return;
            }

            device.motion(time, x, y);
            data.last_dnd_motion = Some(pos);
        });
    }

    pub fn dnd_drop(&mut self) {
        self.print_dnd_debug("dnd drop");
        let dnd = match &mut self.dnd {
            Some(d) => d,
            None => return,
        };
        if dnd.dropped {
            return;
        }
        dnd.dropped = true;

        let action = dnd.action;
        if dnd.focus.is_none() || dnd.mime.is_none() || action.is_empty() {
            self.dnd_cancel();
            return;
        }

        if let Some(s) = dnd.source.as_ref() {
            s.action(action);
            s.dnd_drop_performed();
        }

        self.for_all_devices(|device, data| {
            if data.dnd_focus.is_none() {
                return;
            }
            data.dnd_focus = None;
            data.dnd_offer.as_ref().unwrap().action(action);
            device.drop();
            device.leave();
        });

        if self.dnd.as_ref().unwrap().source.is_none() {
            // Immediately destroy dnd when the drag doesn't have a source
            self.dnd = None;
        }
    }

    fn unfocus_devices(&self) {
        self.for_all_devices(|device, data| {
            match data.dnd_focus.take() {
                Some(_) => (),
                None => return,
            };
            device.leave();
        });
    }

    pub fn dnd_cancel(&mut self) {
        self.print_dnd_debug("dnd cancel");
        let dnd = match &self.dnd {
            Some(d) => d,
            None => return,
        };
        self.unfocus_devices();

        if let Some(s) = dnd.source.as_ref() {
            s.cancelled();
        } else {
            // Immediately destroy dnd when the drag doesn't have a source
            self.dnd = None;
        }
    }

    fn for_all_devices<F>(&self, mut f: F)
    where
        F: FnMut(&WlDataDevice, &mut WLCDataDeviceData),
    {
        for device in &self.devices {
            with_device_data(device, |data| f(device, data));
        }
    }
}

impl GlobalDispatch<WlDDM, ()> for WLCState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WlDDM>,
        _data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let _ddm: WlDDM = data_init.init(resource, ());
    }
}

impl Dispatch<WlDDM, ()> for WLCState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _ddm: &WlDDM,
        request: wl_ddm::Request,
        _data: &(),
        _disp: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_ddm::Request::CreateDataSource { id } => {
                let source_data = WLCDataSourceData {
                    usage: SourceUsage::Unused,
                    mime: vec![],
                    actions: DndAction::None,
                };
                let source_data = Arc::new(Mutex::new(source_data));
                let _source = data_init.init(id, source_data.clone());
            }
            wl_ddm::Request::GetDataDevice { id, .. } => {
                let device_data = WLCDataDeviceData {
                    dnd_focus: None,
                    last_dnd_motion: None,
                    dnd_offer: None,
                };
                let device_data = Arc::new(Mutex::new(device_data));
                let device = data_init.init(id, device_data.clone());

                state.data.devices.push(device);
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<WlDataSource, WLCDataSource> for WLCState {
    fn request(
        state: &mut Self,
        _client: &Client,
        source: &WlDataSource,
        request: wl_data_source::Request,
        _data: &WLCDataSource,
        _disp: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_source::Request::Offer { mime_type } => {
                with_source_data(source, |data| {
                    data.mime.push(mime_type);
                });
            }
            wl_data_source::Request::Destroy => {
                let dnd = match &state.data.dnd {
                    Some(d) => d,
                    None => return,
                };
                if dnd.source.as_ref().and_then(|s| s.wayland())
                    == Some(source)
                {
                    state.data.print_dnd_debug("data source destroy");
                    state.data.unfocus_devices();
                    state.data.dnd = None;
                }
            }
            wl_data_source::Request::SetActions { dnd_actions } => {
                let actions = match dnd_actions.into_result() {
                    Ok(a) => a,
                    Err(_) => return,
                };

                with_source_data(source, |data| {
                    data.actions = actions;
                });

                state.data.for_all_devices(|_device, data| {
                    if data.dnd_offer.is_none() {
                        return;
                    }
                    let offer = data.dnd_offer.as_ref().unwrap();
                    let matches_source = with_offer_data(offer, |data| {
                        data.wl_source() == Some(source)
                    });
                    if matches_source {
                        offer.source_actions(actions);
                    }
                });
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        source: &WlDataSource,
        _data: &WLCDataSource,
    ) {
        if matches!(
            &state.data.clipboard,
            Some(ClipboardSource::Wayland(c)) if c == source
        ) {
            state.data.clipboard = None;
        }
        if let Some(dnd) = &state.data.dnd
            && dnd.source.as_ref().and_then(|s| s.wayland()) == Some(source)
        {
            state.data.unfocus_devices();
            state.data.dnd = None;
        }
    }
}

impl Dispatch<WlDataDevice, WLCDataDevice> for WLCState {
    fn request(
        state: &mut Self,
        client: &Client,
        device: &WlDataDevice,
        request: wl_data_device::Request,
        _data: &WLCDataDevice,
        _disp: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_device::Request::StartDrag {
                source,
                serial,
                icon,
                ..
            } => {
                if let Some(ref s) = source {
                    with_source_data(s, |data| {
                        if data.usage != SourceUsage::Unused {
                            device.post_error(
                                wl_data_device::Error::UsedSource,
                                "reused data source",
                            );
                            return;
                        }
                        data.usage = SourceUsage::Drag;
                    });
                }

                state.data.print_dnd_debug("drag start");

                // Cancel if drag is already active
                if state.data.dnd.is_some() {
                    if let Some(s) = source {
                        s.cancelled();
                    }
                    return;
                }

                state.data.dnd = Some(WLCDndEvent {
                    start_serial: serial,
                    request_sent: false,
                    client: client.clone(),
                    source: source.clone().map(DndSource::Wayland),
                    icon: icon.clone(),
                    focus: None,
                    mime: None,
                    action: DndAction::None,
                    dropped: false,
                });
            }
            wl_data_device::Request::SetSelection { source, serial: _ } => {
                let focus = state.data.clipboard_focus.as_ref();
                if !focus.is_some_and(|c| c == client) {
                    return;
                }

                if let Some(source) = &source {
                    let mime =
                        with_source_data(source, |data| data.mime.clone());

                    // STOP SENDING ME EMPTY CLIPBOARD SELECTIONS WITH THE
                    // SAVE_TARGETS MIME. I HAVE NO CLUE WHAT THAT IS.
                    // WHYYYYYYYY. I blame X11.
                    if mime.iter().any(|m| m == "SAVE_TARGETS") {
                        return;
                    }

                    with_source_data(source, |data| {
                        if data.usage != SourceUsage::Unused {
                            device.post_error(
                                wl_data_device::Error::UsedSource,
                                "reused data source",
                            );
                            return;
                        }
                        data.usage = SourceUsage::Selection;
                    });
                }

                state.data.set_clipboard(
                    source.map(ClipboardSource::Wayland),
                );
                // Mirror the new Wayland selection onto the X11 side so X11
                // (Xwayland) apps can paste it.
                crate::xwayland::bridge_wayland_selection_to_x11(state);
            }
            wl_data_device::Request::Release => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        device: &WlDataDevice,
        _data: &WLCDataDevice,
    ) {
        state.data.devices.retain(|d| d != device);
    }
}

impl Dispatch<WlDataOffer, WLCDataOffer> for WLCState {
    fn request(
        state: &mut Self,
        _client: &Client,
        offer: &WlDataOffer,
        request: wl_data_offer::Request,
        _data: &WLCDataOffer,
        _disp: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_offer::Request::Receive { mime_type, fd } => {
                enum Routed {
                    Wayland(WlDataSource),
                    X11Clipboard,
                    X11Dnd,
                    None,
                }
                let routed = with_offer_data(offer, |data| {
                    if !data.source.is_alive()
                        || !data.source.mime().contains(&mime_type)
                    {
                        return Routed::None;
                    }
                    match &data.source {
                        OfferSource::Clipboard(ClipboardSource::Wayland(s))
                        | OfferSource::Dnd(DndSource::Wayland(s)) => {
                            Routed::Wayland(s.clone())
                        }
                        OfferSource::Clipboard(ClipboardSource::X11 { .. }) => {
                            Routed::X11Clipboard
                        }
                        OfferSource::Dnd(DndSource::X11 { .. }) => {
                            Routed::X11Dnd
                        }
                    }
                });
                match routed {
                    Routed::Wayland(s) => s.send(mime_type, fd.as_fd()),
                    // The selection is X11-owned: an X11 app copied. Hand the
                    // Wayland client's read fd to Xwayland, which writes the
                    // requested mime type into it.
                    Routed::X11Clipboard => {
                        if let Some(xwm) = state.xwm.as_mut() {
                            let _ = xwm.send_selection(
                                SelectionTarget::Clipboard,
                                mime_type,
                                fd,
                            );
                        }
                    }
                    // The drag is X11-sourced: convert the XdndSelection from
                    // the X11 source into the Wayland client's fd. See xdnd.rs.
                    Routed::X11Dnd => {
                        state.xdnd_target_receive(mime_type, fd);
                    }
                    Routed::None => {}
                }
            }
            wl_data_offer::Request::Accept { mime_type, .. } => {
                let dnd = match &mut state.data.dnd {
                    Some(d) => d,
                    None => return,
                };
                // The accepted type belongs to the offer's own drag. A Wayland
                // source is told via target(); an X11 source has no Wayland
                // resource, so only WLCDndEvent.mime is recorded (the XDND
                // bridge reads it for the XdndStatus accept flag).
                let owns_drag = with_offer_data(offer, |data| {
                    match (&data.source, dnd.source.as_ref()) {
                        (
                            OfferSource::Dnd(DndSource::Wayland(src)),
                            Some(DndSource::Wayland(cur)),
                        ) => src == cur,
                        (
                            OfferSource::Dnd(DndSource::X11 { .. }),
                            Some(DndSource::X11 { .. }),
                        ) => true,
                        _ => false,
                    }
                });
                if !owns_drag {
                    return;
                }
                if let Some(src) = dnd.source.as_ref() {
                    src.target(mime_type.clone());
                }
                dnd.mime = mime_type;
                // An X11-sourced drag's accept state just changed - push a
                // fresh XdndStatus to the X11 source so it learns the drop
                // became allowed even if its cursor is held still.
                let is_x11 = matches!(
                    dnd.source.as_ref(),
                    Some(DndSource::X11 { .. })
                );
                if is_x11 {
                    state.xdnd_target_refresh_status();
                }
            }
            wl_data_offer::Request::Destroy => {
                with_offer_data(offer, |data| {
                    with_device_data(&data.device, |dev| {
                        if dev.dnd_offer.as_ref() == Some(offer) {
                            dev.dnd_offer = None;
                        }
                    });
                });
            }
            wl_data_offer::Request::Finish => {
                // A Wayland drag source is told directly; an X11-sourced drag
                // has its XdndFinished sent and is torn down by the bridge.
                let is_x11 = with_offer_data(offer, |data| {
                    match &data.source {
                        OfferSource::Dnd(DndSource::Wayland(s)) => {
                            s.dnd_finished();
                            false
                        }
                        OfferSource::Dnd(DndSource::X11 { .. }) => true,
                        OfferSource::Clipboard(_) => false,
                    }
                });
                if is_x11 {
                    state.xdnd_target_finished();
                }
            }
            wl_data_offer::Request::SetActions {
                dnd_actions,
                preferred_action,
            } => {
                let dnd_actions = match dnd_actions.into_result() {
                    Ok(a) => a,
                    Err(_) => return,
                };
                let preferred_action = match preferred_action.into_result() {
                    Ok(a) => a,
                    Err(_) => return,
                };

                let source_actions = with_offer_data(offer, |data| {
                    match &data.source {
                        OfferSource::Dnd(s) => s.actions(),
                        OfferSource::Clipboard(_) => DndAction::None,
                    }
                });

                let actions = dnd_actions & source_actions;
                let action = if actions.contains(preferred_action)
                    && preferred_action != DndAction::Ask
                {
                    preferred_action
                } else {
                    actions
                        .iter()
                        .find(|a| *a != DndAction::Ask)
                        .unwrap_or(DndAction::None)
                };

                assert!(action.iter().count() <= 1);
                assert_eq!(action.iter().find(|a| *a == DndAction::Ask), None);
                assert!(actions.contains(action));

                let dnd = match state.data.dnd.as_mut() {
                    Some(d) => d,
                    None => return,
                };
                dnd.action = action;

                let source = match dnd.source.as_ref() {
                    Some(s) => s,
                    None => return,
                };

                offer.action(dnd.action);
                // No-op for an X11 source - the chosen action surfaces to the
                // X11 source as the XdndStatus action atom, not here.
                source.action(dnd.action);

                // An X11-sourced drag's action just changed - refresh the
                // XdndStatus so the X11 source sees the new action.
                if matches!(source, DndSource::X11 { .. }) {
                    state.xdnd_target_refresh_status();
                }
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        _state: &mut Self,
        _client: ClientId,
        _offer: &WlDataOffer,
        _data: &WLCDataOffer,
    ) {
    }
}
