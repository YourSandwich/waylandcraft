use std::sync::Arc;
use smithay::{
    reexports::{
        calloop::{
            generic::Generic as GenericEvent,
            self, EventLoop,
        },
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            Display, DisplayHandle,
        },
    },
    wayland::{
        socket::ListeningSocketSource,
    },
};

pub struct WLCState {
    pub display_handle: DisplayHandle,
}

pub struct WLCClient {
}

impl WLCClient {
    fn new() -> Self {
        Self {}
    }
}

impl ClientData for WLCClient {
    fn initialized(&self, _id: ClientId) {
        println!("Client connected!");
    }

    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {
        println!("Client disconnected!");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Hello, world!");

    let mut event_loop: EventLoop<WLCState> = EventLoop::try_new()?;
    let display: Display<WLCState> = Display::new()?;
    let socket = ListeningSocketSource::new_auto()?;

    println!("Listening on: '{}'", socket.socket_name().to_str().unwrap());

    let mut state = WLCState {
        display_handle: display.handle(),
    };

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

    loop {
        event_loop.dispatch(None, &mut state).unwrap();
        state.display_handle.flush_clients().unwrap();
    }
}
