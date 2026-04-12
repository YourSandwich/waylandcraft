use std::sync::{Arc, Mutex};
use std::ops::DerefMut;
use std::os::fd::AsFd;
use crate::WLCState;
use crate::utils::{new_serial, get_time, to_fixed2};
use smithay::{
    reexports::{
        wayland_server::{
            protocol::{
                wl_data_device_manager::self as wl_ddm,
                wl_data_device_manager::WlDataDeviceManager as WlDDM,
                wl_data_source::{self, WlDataSource},
                wl_data_device::{self, WlDataDevice},
                wl_data_offer::{self, WlDataOffer},
                wl_surface::WlSurface,
            },
            backend::ClientId,
            DisplayHandle, DataInit, New, GlobalDispatch, Dispatch, Client,
            Resource,
        },
    },
};

pub struct WLCDataState {
    pub devices: Vec<WlDataDevice>,
    pub clipboard: Option<WlDataSource>,
    pub clipboard_focus: Option<Client>,
    pub dnd: Option<WLCDndEvent>,
    display_handle: DisplayHandle,
}

pub struct WLCDndEvent {
    pub start_serial: u32,
    pub source: WlDataSource,
    pub focus: Option<WlSurface>,
    pub mime: Option<String>,
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
}

type WLCDataOffer = Arc<Mutex<WLCDataOfferData>>;
struct WLCDataOfferData {
    // NOTE: The source is in the id space of the source client!!
    source: WlDataSource,
}

type WLCDataDevice = Arc<Mutex<WLCDataDeviceData>>;
struct WLCDataDeviceData {
    dnd_focus: Option<WlSurface>,
    last_dnd_motion: Option<(i32, i32)>,
}

fn with_source_data<F, R>(source: &WlDataSource, f: F) -> R
    where F: FnOnce(&mut WLCDataSourceData) -> R
{
    let mut guard = source
        .data::<WLCDataSource>()
        .unwrap()
        .lock()
        .unwrap();
    let data = guard.deref_mut();
    f(data)
}

fn with_offer_data<F, R>(offer: &WlDataOffer, f: F) -> R
    where F: FnOnce(&mut WLCDataOfferData) -> R
{
    let mut guard = offer
        .data::<WLCDataOffer>()
        .unwrap()
        .lock()
        .unwrap();
    let data = guard.deref_mut();
    f(data)
}

fn with_device_data<F, R>(device: &WlDataDevice, f: F) -> R
    where F: FnOnce(&mut WLCDataDeviceData) -> R
{
    let mut guard = device
        .data::<WLCDataDevice>()
        .unwrap()
        .lock()
        .unwrap();
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
        self.display_handle.create_global::<WLCState, WlDDM, ()>(3, ());
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
            None => {return;},
        };
        for device in &self.devices {
            if !device.client().is_some_and(|c| c == *client) {
                continue;
            }

            if let Some(clipboard) = &self.clipboard {
                let offer_data = WLCDataOfferData {
                    source: clipboard.clone(),
                };
                let offer_data = Arc::new(Mutex::new(offer_data));
                let offer = client.create_resource::<
                    WlDataOffer, WLCDataOffer, WLCState
                >(&self.display_handle, device.version(), offer_data).unwrap();

                device.data_offer(&offer);
                with_source_data(&clipboard, |data| {
                    for m in data.mime.iter().cloned() {
                        offer.offer(m);
                    }
                });
                device.selection(Some(&offer));
            } else {
                device.selection(None);
            }
        }
    }

    fn unfocus_devices(&mut self) {
        self.for_all_devices(|device, data| {
            match data.dnd_focus.take() {
                Some(_) => (),
                None => { return }
            };
            device.leave();
        });
    }

    pub fn dnd_motion(&mut self, surface: Option<WlSurface>, x: f64, y: f64) {
        if self.dnd.is_none() { return }
        let source = self.dnd.as_ref().unwrap().source.clone();
        let focus = self.dnd.as_ref().unwrap().focus.clone();

        if surface != focus {
            // Reset the accepted type when moving to different surface
            source.target(None);
            self.dnd.as_mut().unwrap().mime = None;
        }
        self.dnd.as_mut().unwrap().focus = surface.clone();

        // Unfocus devices focused on wrong surface
        self.for_all_devices(|device, data| {
            let focus = match &data.dnd_focus {
                Some(s) => s,
                None => { return }
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
            None => { return }
        };

        // Send device enter events
        self.for_all_devices(|device, data| {
            // Already focused
            if data.dnd_focus.is_some() { return }

            // Check if client does not own surface
            let surface_client = surface.client().unwrap();
            let device_client = device.client().unwrap();
            if surface_client != device_client { return }

            // Create offer
            let offer_data = WLCDataOfferData {
                source: source.clone(),
            };
            let offer_data = Arc::new(Mutex::new(offer_data));
            let offer = device_client.create_resource::<
                WlDataOffer, WLCDataOffer, WLCState
            >(&self.display_handle, device.version(), offer_data).unwrap();

            // Send offer to client
            device.data_offer(&offer);
            with_source_data(&source, |data| {
                for m in data.mime.iter().cloned() {
                    offer.offer(m);
                }
            });

            // Make device enter surface
            device.enter(new_serial(), &surface, x, y, Some(&offer));
            data.dnd_focus = Some(surface.clone());
            data.last_dnd_motion = None;
        });

        let time = get_time();
        let pos: (i32, i32) = to_fixed2(x, y);

        // Send device motion events
        self.for_all_devices(|device, data| {
            if !data.dnd_focus.is_some() { return }
            if data.last_dnd_motion == Some(pos) { return }

            device.motion(time, x, y);
            data.last_dnd_motion = Some(pos);
        });
    }

    pub fn dnd_cancel(&mut self) {
        let dnd = match self.dnd.take() {
            Some(d) => d,
            None => { return }
        };
        dnd.source.cancelled();
        self.unfocus_devices();
    }

    fn for_all_devices<F>(&self, mut f: F)
        where F: FnMut(&WlDataDevice, &mut WLCDataDeviceData)
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
                };
                let source_data = Arc::new(Mutex::new(source_data));
                let _source = data_init.init(id, source_data.clone());
            },
            wl_ddm::Request::GetDataDevice { id, .. } => {
                let device_data = WLCDataDeviceData {
                    dnd_focus: None,
                    last_dnd_motion: None,
                };
                let device_data = Arc::new(Mutex::new(device_data));
                let device = data_init.init(id, device_data.clone());

                state.data.devices.push(device);
            },
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
            },
            wl_data_source::Request::Destroy => {
                let dnd = match &state.data.dnd {
                    Some(d) => d,
                    None => { return }
                };
                if *source == dnd.source {
                    state.data.dnd_cancel();
                }
            },
            wl_data_source::Request::SetActions { .. } => {},
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        resource: &WlDataSource,
        _data: &WLCDataSource,
    ) {
        if state.data.clipboard.as_ref().is_some_and(|c| c == resource) {
            state.data.clipboard = None;
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
            wl_data_device::Request::StartDrag { source, serial, .. } => {
                let source = match &source {
                    Some(s) => s,
                    None => { return }
                };

                with_source_data(source, |data| {
                    if data.usage != SourceUsage::Unused {
                        device.post_error(
                            wl_data_device::Error::UsedSource,
                            "reused data source",
                        );
                        return;
                    }
                    data.usage = SourceUsage::Drag;
                });

                // Cancel if drag is already active
                if state.data.dnd.is_some() {
                    source.cancelled();
                    return;
                }

                state.data.dnd = Some(WLCDndEvent {
                    start_serial: serial,
                    source: source.clone(),
                    focus: None,
                    mime: None,
                });
            },
            wl_data_device::Request::SetSelection { source, serial: _ } => {
                let focus = state.data.clipboard_focus.as_ref();
                if !focus.is_some_and(|c| c == client) {
                    return;
                }

                if let Some(source) = &source {
                    let mime = with_source_data(source, |data| {
                        data.mime.clone()
                    });

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

                if let Some(old_clipboard) = &state.data.clipboard {
                    old_clipboard.cancelled();
                }
                state.data.clipboard = source;
                state.data.send_clipboard();
            },
            wl_data_device::Request::Release => {},
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
                with_offer_data(offer, |data| {
                    if !data.source.is_alive() {
                        return;
                    }

                    let mime = with_source_data(&data.source, |source_data| {
                        source_data.mime.clone()
                    });

                    if !mime.iter().any(|m| *m == mime_type) {
                        return;
                    }

                    data.source.send(mime_type, fd.as_fd());
                });
            },
            wl_data_offer::Request::Accept { mime_type, .. } => {
                let dnd = match &mut state.data.dnd {
                    Some(d) => d,
                    None => { return }
                };
                with_offer_data(offer, |data| {
                    if data.source != dnd.source { return }
                    data.source.target(mime_type.clone());
                    dnd.mime = mime_type;
                });
            },
            wl_data_offer::Request::Destroy { .. } => {},
            wl_data_offer::Request::Finish { .. } => {},
            wl_data_offer::Request::SetActions { .. } => {},
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
