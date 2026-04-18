use crate::WLCState;
use smithay::{
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
        Resource,
        protocol::wl_output::{self, WlOutput},
    },
    utils::{Logical, Size},
};

pub struct WLCOutput {
    pub outputs: Vec<WlOutput>,
    size: Size<i32, Logical>,
    // size of content area
    bounds: Size<i32, Logical>,
    display_handle: DisplayHandle,
}

impl WLCOutput {
    pub fn new(display_handle: &DisplayHandle) -> Self {
        WLCOutput {
            outputs: vec![],
            size: Size::new(1920, 1080),
            bounds: Size::new(1920, 1080),
            display_handle: display_handle.clone(),
        }
    }

    pub fn create_global(&self) {
        self.display_handle
            .create_global::<WLCState, WlOutput, ()>(4, ());
    }

    pub fn size(&self) -> Size<i32, Logical> {
        self.size.clone()
    }

    pub fn bounds(&self) -> Size<i32, Logical> {
        self.bounds.clone()
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.size = Size::new(width, height);
        let flags = wl_output::Mode::Current;
        for output in &self.outputs {
            output.mode(flags, self.size.w, self.size.h, 0);
            if output.version() >= 2 {
                output.done();
            }
        }
    }

    pub fn set_bounds(&mut self, width: i32, height: i32) {
        self.bounds = Size::new(width, height);
    }
}

impl GlobalDispatch<WlOutput, ()> for WLCState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WlOutput>,
        _data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let output: WlOutput = data_init.init(resource, ());

        let flags = wl_output::Mode::Current;
        let size = &state.output.size;
        output.mode(flags, size.w, size.h, 0);

        let location = (0, 0);
        let physical = (0, 0);

        output.geometry(
            location.0,
            location.1,
            physical.0,
            physical.1,
            wl_output::Subpixel::None,
            "Virtual".into(),
            "Monitor".into(),
            wl_output::Transform::Normal,
        );

        if output.version() >= 4 {
            output.name("output-0".into());
            output.description("Virtual Output".into());
        }

        if output.version() >= 2 {
            output.scale(1);
            output.done();
        }
    }
}

impl Dispatch<WlOutput, ()> for WLCState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _output: &WlOutput,
        request: wl_output::Request,
        _data: &(),
        _disp: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_output::Request::Release => {}
            _ => unreachable!(),
        }
    }
}
