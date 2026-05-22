#![allow(non_snake_case)]

use crate::egl::{EGLDisplay, EGLHelper};
use crate::process::cleanup_display_processes;
use crate::svg::render_svg;
use crate::utils::get_time;
use crate::xdg_spec::RawDesktopEntry;
use crate::{WaylandCraft, wlc_init};
use jni::{
    JNIEnv,
    objects::{JClass, JObject, JString, JValue},
    signature::{Primitive, ReturnType},
    sys::{
        JNI_TRUE, jarray, jboolean, jbyte, jdouble, jint, jlong, jobject,
        jsize, jstring, jvalue,
    },
};
use smithay::{
    backend::{
        allocator::{Buffer, dmabuf::WeakDmabuf},
        renderer::utils::RendererSurfaceStateUserData,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Resource,
            protocol::{
                wl_buffer::WlBuffer,
                wl_keyboard::KeyState,
                wl_pointer::{Axis, ButtonState},
                wl_surface::WlSurface,
            },
        },
    },
    utils::{Logical, Point, Size},
    wayland::{
        compositor::{
            SubsurfaceCachedState, SurfaceAttributes, SurfaceData,
            TraversalAction, with_states,
            with_states as with_surface_data, with_surface_tree_upward,
        },
        dmabuf::get_dmabuf,
        shell::xdg::{
            PopupSurface, SurfaceCachedState, ToplevelSurface,
            XdgToplevelSurfaceData,
        },
        shm::{self, with_buffer_contents},
        single_pixel_buffer::get_single_pixel_buffer,
        viewporter::{ViewportCachedState, ensure_viewport_valid},
    },
    xwayland::X11Surface,
};
use std::ops::DerefMut;
use std::path::PathBuf;

// A managed window, either an xdg-shell toplevel or an X11 (Xwayland) window.
// Both variant types are Clone + PartialEq, so the raw-handle machinery
// (insert_get_handle / get_handle / remove_element) is generic over either.
// Always stored as Box<WlcWindow> in BridgeState, so the variant size gap
// never reaches the heap; boxing X11Surface would only add a needless deref.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq)]
pub(crate) enum WlcWindow {
    Xdg(ToplevelSurface),
    X11(X11Surface),
}

impl WlcWindow {
    fn alive(&self) -> bool {
        match self {
            WlcWindow::Xdg(t) => t.alive(),
            WlcWindow::X11(w) => w.alive(),
        }
    }

    fn wl_surface(&self) -> Option<WlSurface> {
        match self {
            WlcWindow::Xdg(t) => Some(t.wl_surface().clone()),
            WlcWindow::X11(w) => w.wl_surface(),
        }
    }
}

// A managed popup, either an xdg-shell popup or an X11 override-redirect window
// (menus, tooltips, dropdowns). Both render anchored to a parent window instead
// of as a toplevel; the X11 arm mirrors WlcWindow for X11 windows.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq)]
pub(crate) enum WlcPopup {
    Xdg(PopupSurface),
    X11(X11Surface),
}

impl WlcPopup {
    fn alive(&self) -> bool {
        match self {
            WlcPopup::Xdg(p) => p.alive(),
            WlcPopup::X11(w) => w.alive(),
        }
    }

    fn wl_surface(&self) -> Option<WlSurface> {
        match self {
            WlcPopup::Xdg(p) => Some(p.wl_surface().clone()),
            WlcPopup::X11(w) => w.wl_surface(),
        }
    }
}

#[allow(clippy::vec_box)]
pub(crate) struct BridgeState {
    /* Handle collections */
    toplevels: Vec<Box<WlcWindow>>,
    popups: Vec<Box<WlcPopup>>,
    surfaces: Vec<Box<WlSurface>>,
    dmabufs: Vec<Box<WeakDmabuf>>,
}

impl BridgeState {
    pub fn new() -> Self {
        BridgeState {
            toplevels: vec![],
            popups: vec![],
            surfaces: vec![],
            dmabufs: vec![],
        }
    }
}

fn jptr_to_instance(ptr: jlong) -> &'static mut WaylandCraft<'static> {
    let ptr: *mut WaylandCraft = (ptr as usize) as *mut WaylandCraft;
    unsafe { &mut *ptr }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    init")]
pub extern "system" fn init<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    proc_addr: jlong,
    dpy_ptr: jlong,
) -> jlong {
    let dpy: EGLDisplay = (dpy_ptr as usize) as EGLDisplay;
    let egl = EGLHelper::new(dpy, proc_addr as usize);

    let instance = match wlc_init(egl) {
        Ok(i) => i,
        Err(err) => {
            let _ =
                env.throw_new("java/lang/RuntimeException", err.to_string());
            return 0;
        }
    };

    let instance_box: Box<WaylandCraft> = Box::new(instance);
    let ptr: *mut WaylandCraft = Box::into_raw(instance_box);
    let addr: u64 = ptr.addr() as u64;
    addr as i64
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    update")]
pub extern "system" fn update<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    instance.update();
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    cleanupLaunchedApps")]
pub extern "system" fn cleanupLaunchedApps<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    let killed = cleanup_display_processes(
        &instance.state.socket,
        instance.state.xdisplay,
    );
    if killed > 0 {
        println!("[waylandcraft] cleaned up {killed} launched app(s)");
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    socket")]
pub extern "system" fn socket<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jstring {
    let instance = jptr_to_instance(ptr);
    let socket = instance.state.socket.to_str().unwrap();
    env.new_string(socket).unwrap().into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    sendFrame")]
pub extern "system" fn sendFrame<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) {
    let surface = match jptr_to_wlsurface(handle) {
        Some(s) if s.is_alive() => s,
        _ => return,
    };

    with_surface_data(&surface, |data| {
        let mut attr_guard = data.cached_state.get::<SurfaceAttributes>();
        let attr = attr_guard.deref_mut().current();
        for c in attr.frame_callbacks.drain(..) {
            c.done(get_time());
        }
    });
}

// Get or insert an element and return its handle
fn insert_get_handle<T>(vec: &mut Vec<Box<T>>, elem: &T) -> jlong
where
    T: Clone + PartialEq,
{
    if !vec.iter().any(|b| **b == *elem) {
        vec.push(Box::new(elem.clone()));
    }

    let ptr: &mut T = vec.iter_mut().find(|r| ***r == *elem).unwrap();
    ((ptr as *mut T) as usize) as jlong
}

// Get an element and return its handle
// Element has to be in the list, otherwise this functions panics
fn get_handle<T>(vec: &[Box<T>], elem: &T) -> jlong
where
    T: Clone + PartialEq,
{
    let ptr: &T = vec.iter().find(|r| ***r == *elem).unwrap();
    ((ptr as *const T) as usize) as jlong
}

// Insert all elements that aren't in the list already
fn insert_all<T>(vec: &mut Vec<Box<T>>, elems: &[T])
where
    T: Clone + PartialEq,
{
    for elem in elems {
        if !vec.iter().any(|b| **b == *elem) {
            vec.push(Box::new(elem.clone()));
        }
    }
}

// Insert each tracked X11 window not already present, deduped on the stable X11
// window id. X11Surface equality is unusable here - it folds in window
// liveness, so two clones of the same window can compare unequal and produce a
// duplicate (the prior rework's bug).
#[allow(clippy::vec_box)] // Boxed for stable handle addresses, as BridgeState.
fn insert_x11_toplevels(vec: &mut Vec<Box<WlcWindow>>, windows: &[X11Surface]) {
    for window in windows {
        let id = window.window_id();
        let present = vec.iter().any(|b| match b.as_ref() {
            WlcWindow::X11(w) => w.window_id() == id,
            WlcWindow::Xdg(_) => false,
        });
        if !present {
            vec.push(Box::new(WlcWindow::X11(window.clone())));
        }
    }
}

// insert_x11_toplevels for the override/popup list.
#[allow(clippy::vec_box)] // Boxed for stable handle addresses, as BridgeState.
fn insert_x11_popups(vec: &mut Vec<Box<WlcPopup>>, windows: &[X11Surface]) {
    for window in windows {
        let id = window.window_id();
        let present = vec.iter().any(|b| match b.as_ref() {
            WlcPopup::X11(w) => w.window_id() == id,
            WlcPopup::Xdg(_) => false,
        });
        if !present {
            vec.push(Box::new(WlcPopup::X11(window.clone())));
        }
    }
}

// Get handles of all elements in the list
fn get_all_handles<T>(vec: &mut [Box<T>]) -> Vec<jlong>
where
    T: Clone + PartialEq,
{
    vec.iter_mut()
        .map(|r| ((&mut **r) as *mut T) as usize as jlong)
        .collect()
}

// Remove element from list and free it
fn remove_element<T>(vec: &mut Vec<Box<T>>, handle: jlong) {
    // Match on the boxed element's address - never dereference `handle`. By the
    // time freeToplevel/freePopup runs, toplevels()/popups()'s retain has
    // already freed that box, so `handle` is dangling; a stale handle then
    // simply matches nothing.
    vec.retain(|e| (&**e as *const T as usize as jlong) != handle);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevels")]
pub extern "system" fn toplevels<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let xdg_windows: Vec<WlcWindow> = instance
        .state
        .xdg_state
        .toplevel_surfaces()
        .iter()
        .map(|t| WlcWindow::Xdg(t.clone()))
        .collect();
    insert_all(&mut instance.bridge.toplevels, &xdg_windows);

    // The full tracked X11 set goes in - no surface filter. A window stays in
    // bridge.toplevels for its whole life, so its handle is stable and the
    // Java WLCToplevel is created once and never churned.
    let x11_ids: Vec<u32> = instance
        .state
        .x11_windows
        .iter()
        .map(|w| w.window_id())
        .collect();
    insert_x11_toplevels(
        &mut instance.bridge.toplevels,
        &instance.state.x11_windows,
    );

    // An unmapped-but-not-destroyed X11Surface stays alive(), so X11 entries
    // are dropped by X11 window id once destroyed_window removed them from
    // state.x11_windows - alive() alone is not enough.
    instance.bridge.toplevels.retain(|t| match t.as_ref() {
        WlcWindow::Xdg(_) => t.alive(),
        WlcWindow::X11(w) => x11_ids.contains(&w.window_id()),
    });

    let toplevels = get_all_handles(&mut instance.bridge.toplevels);
    let array = env.new_long_array(toplevels.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &toplevels).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    popups")]
pub extern "system" fn popups<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let xdg_popups: Vec<WlcPopup> = instance
        .state
        .xdg_state
        .popup_surfaces()
        .iter()
        .map(|p| WlcPopup::Xdg(p.clone()))
        .collect();
    insert_all(&mut instance.bridge.popups, &xdg_popups);

    // The full tracked X11 override/popup set goes in - no surface filter,
    // the same as toplevels().
    let x11_ids: Vec<u32> = instance
        .state
        .x11_override_windows
        .iter()
        .map(|w| w.window_id())
        .collect();
    insert_x11_popups(
        &mut instance.bridge.popups,
        &instance.state.x11_override_windows,
    );

    // As with toplevels(), an unmapped-but-not-destroyed X11Surface stays
    // alive(), so X11 entries are dropped by X11 window id once
    // destroyed_window removed them from state.x11_override_windows.
    instance.bridge.popups.retain(|p| match p.as_ref() {
        WlcPopup::Xdg(_) => p.alive(),
        WlcPopup::X11(w) => x11_ids.contains(&w.window_id()),
    });

    let popups = get_all_handles(&mut instance.bridge.popups);
    let array = env.new_long_array(popups.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &popups).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    minimizeReq")]
pub extern "system" fn minimizeReq<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let handles: Vec<jlong> = instance
        .state
        .requests
        .minimize
        .iter()
        .filter(|t| t.alive())
        .map(|t| {
            insert_get_handle(
                &mut instance.bridge.toplevels,
                &WlcWindow::Xdg(t.clone()),
            )
        })
        .collect();

    instance.state.requests.minimize.clear();

    let array = env.new_long_array(handles.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &handles).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    maximizeReq")]
pub extern "system" fn maximizeReq<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let handles: Vec<jlong> = instance
        .state
        .requests
        .maximize
        .iter()
        .filter(|t| t.alive())
        .map(|t| {
            insert_get_handle(
                &mut instance.bridge.toplevels,
                &WlcWindow::Xdg(t.clone()),
            )
        })
        .collect();

    instance.state.requests.maximize.clear();

    let array = env.new_long_array(handles.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &handles).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    unmaximizeReq")]
pub extern "system" fn unmaximizeReq<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let handles: Vec<jlong> = instance
        .state
        .requests
        .unmaximize
        .iter()
        .filter(|t| t.alive())
        .map(|t| {
            insert_get_handle(
                &mut instance.bridge.toplevels,
                &WlcWindow::Xdg(t.clone()),
            )
        })
        .collect();

    instance.state.requests.unmaximize.clear();

    let array = env.new_long_array(handles.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &handles).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    fullscreenReq")]
pub extern "system" fn fullscreenReq<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let handles: Vec<jlong> = instance
        .state
        .requests
        .fullscreen
        .iter()
        .filter(|t| t.alive())
        .map(|t| {
            insert_get_handle(
                &mut instance.bridge.toplevels,
                &WlcWindow::Xdg(t.clone()),
            )
        })
        .collect();

    instance.state.requests.fullscreen.clear();

    let array = env.new_long_array(handles.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &handles).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    unfullscreenReq")]
pub extern "system" fn unfullscreenReq<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let handles: Vec<jlong> = instance
        .state
        .requests
        .unfullscreen
        .iter()
        .filter(|t| t.alive())
        .map(|t| {
            insert_get_handle(
                &mut instance.bridge.toplevels,
                &WlcWindow::Xdg(t.clone()),
            )
        })
        .collect();

    instance.state.requests.unfullscreen.clear();

    let array = env.new_long_array(handles.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &handles).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    moveRequest")]
pub extern "system" fn moveRequest<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);
    let serial = instance.state.requests.move_interactive.pop();

    let serial = match serial {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let serial = Into::<u32>::into(serial) as jint;
    let array = env.new_int_array(1).unwrap();
    env.set_int_array_region(&array, 0, &[serial]).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    resizeRequest")]
pub extern "system" fn resizeRequest<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);
    let req = instance.state.requests.resize_interactive.pop();

    let (serial, edges) = match req {
        Some(r) => r,
        None => return std::ptr::null_mut(),
    };

    let serial = Into::<u32>::into(serial) as jint;
    let edges = Into::<u32>::into(edges) as jint;

    let array = env.new_int_array(2).unwrap();
    env.set_int_array_region(&array, 0, &[serial, edges])
        .unwrap();
    array.into_raw()
}

#[allow(non_upper_case_globals)]
const WLCSurface_class: &str = "dev/evvie/waylandcraft/bridge/WLCSurface";

#[allow(non_upper_case_globals)]
const WaylandCraftBridge_class: &str =
    "dev/evvie/waylandcraft/bridge/WaylandCraftBridge";

fn jptr_to_wlsurface(ptr: jlong) -> Option<WlSurface> {
    if ptr == 0 {
        return None;
    }
    let ptr: *mut WlSurface = (ptr as usize) as *mut WlSurface;
    let r = unsafe { &mut *ptr };
    Some(r.clone())
}

enum BufferAttachResult {
    Success,
    Error,
    NotManaged,
}

impl BufferAttachResult {
    fn not_managed(&self) -> bool {
        matches!(self, Self::NotManaged)
    }
}

fn try_attach_shm(
    env: &mut JNIEnv,
    obj: &JObject,
    buf: &WlBuffer,
    surf_data: &SurfaceData,
) -> BufferAttachResult {
    let r = with_buffer_contents(buf, |ptr, _len, metadata| {
        let width = metadata.width as jint;
        let height = metadata.height as jint;
        let format = (metadata.format as u32) as jint;
        let stride = metadata.stride as jint;
        ensure_viewport_valid(surf_data, Size::new(width, height));

        unsafe {
            let ptr = ptr.offset(metadata.offset as isize);
            let jptr = (ptr as usize) as jlong;
            env.call_method_unchecked(
                obj,
                (WLCSurface_class, "attachShmBuffer", "(JIIII)V"),
                ReturnType::Primitive(Primitive::Void),
                &[
                    jvalue { j: jptr },
                    jvalue { i: width },
                    jvalue { i: height },
                    jvalue { i: format },
                    jvalue { i: stride },
                ],
            )
            .unwrap();
        }
    });

    match r {
        Ok(_) => BufferAttachResult::Success,
        Err(shm::BufferAccessError::NotManaged) => {
            BufferAttachResult::NotManaged
        }
        Err(_) => BufferAttachResult::Error,
    }
}

fn try_attach_single_pixel(
    env: &mut JNIEnv,
    obj: &JObject,
    buf: &WlBuffer,
    surf_data: &SurfaceData,
) -> BufferAttachResult {
    let pix = match get_single_pixel_buffer(buf) {
        Ok(p) => p,
        Err(_) => {
            return BufferAttachResult::NotManaged;
        }
    };

    ensure_viewport_valid(surf_data, Size::new(1, 1));

    let [r, g, b, a] = pix.rgba8888();

    unsafe {
        env.call_method_unchecked(
            obj,
            (WLCSurface_class, "attachSinglePixelBuffer", "(BBBB)V"),
            ReturnType::Primitive(Primitive::Void),
            &[
                jvalue { b: r as jbyte },
                jvalue { b: g as jbyte },
                jvalue { b: b as jbyte },
                jvalue { b: a as jbyte },
            ],
        )
        .unwrap();
    }

    BufferAttachResult::Success
}

fn try_attach_dmabuf(
    instance: &mut WaylandCraft,
    env: &mut JNIEnv,
    obj: &JObject,
    buf: &WlBuffer,
    surf_data: &SurfaceData,
) -> BufferAttachResult {
    let dmabuf = match get_dmabuf(buf) {
        Ok(d) => d,
        Err(_) => return BufferAttachResult::NotManaged,
    };

    let width = dmabuf.width() as jint;
    let height = dmabuf.height() as jint;
    ensure_viewport_valid(surf_data, Size::new(width, height));

    let weak = dmabuf.weak();
    let handle = insert_get_handle(&mut instance.bridge.dmabufs, &weak);

    let already_attached = unsafe {
        env.call_method_unchecked(
            obj,
            (WLCSurface_class, "attachDmabuf", "(J)Z"),
            ReturnType::Primitive(Primitive::Boolean),
            &[jvalue { j: handle }],
        )
        .unwrap()
        .z()
        .unwrap()
    };

    if already_attached {
        return BufferAttachResult::Success;
    }

    let image = instance.egl.dmabuf_to_image(dmabuf);
    unsafe {
        env.call_method_unchecked(
            obj,
            (WLCSurface_class, "attachNewDmabuf", "(JJII)V"),
            ReturnType::Primitive(Primitive::Void),
            &[
                jvalue { j: handle },
                jvalue {
                    j: (image as usize) as jlong,
                },
                jvalue { i: width },
                jvalue { i: height },
            ],
        )
        .unwrap();
    }

    BufferAttachResult::Success
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    dmabufs")]
pub extern "system" fn dmabufs<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);
    instance.bridge.dmabufs.retain(|d| !d.is_gone());

    let dmabufs = get_all_handles(&mut instance.bridge.dmabufs);
    let array = env.new_long_array(dmabufs.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &dmabufs).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    updateSurfaceData")]
pub extern "system" fn updateSurfaceData<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    obj: JObject<'l>,
) {
    let instance = jptr_to_instance(ptr);
    let handle: jlong = env
        .get_field_unchecked(
            &obj,
            (WLCSurface_class, "handle", "J"),
            ReturnType::Primitive(Primitive::Long),
        )
        .unwrap()
        .j()
        .unwrap();

    let surface = match jptr_to_wlsurface(handle) {
        Some(s) => s,
        None => return,
    };

    with_states(&surface, |data| {
        // Read smithay's RendererSurfaceState, built by on_commit_buffer_handler
        // in CompositorHandler::commit. It owns the wl_buffer in a refcounted
        // Buffer; the buffer is never touched here, so wl_buffer.release stays
        // smithay's job (fired only when a newer buffer supersedes this one).
        // None means the surface has never had a commit processed.
        let renderer_state =
            data.data_map.get::<RendererSurfaceStateUserData>();

        // Clone the Buffer (cheap Arc) out of the locked state so the attach
        // calls below do not hold the RendererSurfaceState mutex across JNI.
        let buffer = renderer_state
            .map(|s| s.lock().unwrap().buffer().cloned());

        match buffer {
            // Has a renderer state with a current buffer: import it.
            Some(Some(buf)) => {
                // First try shm
                let mut r = try_attach_shm(&mut env, &obj, &buf, data);

                // If not managed by shm, try single pixel
                if r.not_managed() {
                    r = try_attach_single_pixel(&mut env, &obj, &buf, data);
                }

                // If not managed by single pixel, try dmabuf
                if r.not_managed() {
                    r = try_attach_dmabuf(
                        instance, &mut env, &obj, &buf, data,
                    );
                }

                let _ = r;
            }
            // Has a renderer state but no buffer: the surface was unmapped
            // (a null buffer was committed). Drop the Java-side texture.
            Some(None) => unsafe {
                env.call_method_unchecked(
                    &obj,
                    (WLCSurface_class, "removeBuffer", "()V"),
                    ReturnType::Primitive(Primitive::Void),
                    &[],
                )
                .unwrap();
            },
            // No renderer state yet: nothing committed, nothing to do.
            None => {}
        }

        let mut vp_data_guard = data.cached_state.get::<ViewportCachedState>();
        let vp_data = vp_data_guard.deref_mut().current();

        if let Some(src) = vp_data.src {
            unsafe {
                env.call_method_unchecked(
                    &obj,
                    (WLCSurface_class, "setViewportSrc", "(DDDD)V"),
                    ReturnType::Primitive(Primitive::Void),
                    &[
                        jvalue { d: src.loc.x },
                        jvalue { d: src.loc.y },
                        jvalue { d: src.size.w },
                        jvalue { d: src.size.h },
                    ],
                )
                .unwrap();
            }
        }

        if let Some(dst) = vp_data.dst {
            unsafe {
                env.call_method_unchecked(
                    &obj,
                    (WLCSurface_class, "setViewportDst", "(II)V"),
                    ReturnType::Primitive(Primitive::Void),
                    &[jvalue { i: dst.w }, jvalue { i: dst.h }],
                )
                .unwrap();
            }
        }
    });
}

fn jptr_to_toplevel(ptr: jlong) -> &'static mut WlcWindow {
    let ptr: *mut WlcWindow = (ptr as usize) as *mut WlcWindow;
    unsafe { &mut *ptr }
}

fn jptr_to_popup(ptr: jlong) -> &'static mut WlcPopup {
    let ptr: *mut WlcPopup = (ptr as usize) as *mut WlcPopup;
    unsafe { &mut *ptr }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelSurface")]
pub extern "system" fn toplevelSurface<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) -> jlong {
    let instance = jptr_to_instance(ptr);

    match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => {
            let surface: &WlSurface = toplevel.wl_surface();
            insert_get_handle(&mut instance.bridge.surfaces, surface)
        }
        WlcWindow::X11(window) => match window.wl_surface() {
            Some(surface) => {
                insert_get_handle(&mut instance.bridge.surfaces, &surface)
            }
            None => 0,
        },
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    popupSurface")]
pub extern "system" fn popupSurface<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) -> jlong {
    let instance = jptr_to_instance(ptr);

    match jptr_to_popup(handle) {
        WlcPopup::Xdg(popup) => {
            let surface: &WlSurface = popup.wl_surface();
            insert_get_handle(&mut instance.bridge.surfaces, surface)
        }
        WlcPopup::X11(window) => match window.wl_surface() {
            Some(surface) => {
                insert_get_handle(&mut instance.bridge.surfaces, &surface)
            }
            None => 0,
        },
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    popupParent")]
pub extern "system" fn popupParent<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) -> jlong {
    let instance = jptr_to_instance(ptr);

    let parent_surface: WlSurface = match jptr_to_popup(handle) {
        WlcPopup::Xdg(popup) => match popup.get_parent_surface() {
            Some(s) => s,
            None => return 0,
        },
        // An X11 override-redirect window carries no parent reference. Its
        // parent is the X11 toplevel of the same Xwayland whose geometry
        // contains (or is nearest to) the OR window's position - both
        // geometries share the X11 coordinate space.
        WlcPopup::X11(window) => {
            return x11_override_parent(&instance.bridge.toplevels, window);
        }
    };

    for toplevel in &instance.bridge.toplevels {
        if toplevel.wl_surface().as_ref() == Some(&parent_surface) {
            return get_handle(&instance.bridge.toplevels, toplevel);
        }
    }

    for popup in &instance.bridge.popups {
        if popup.wl_surface().as_ref() == Some(&parent_surface) {
            return get_handle(&instance.bridge.popups, popup);
        }
    }

    0
}

// Pick the X11 toplevel that owns an override-redirect window: the one whose
// geometry contains the OR window's origin, else the one nearest to it.
fn x11_override_parent(
    toplevels: &[Box<WlcWindow>],
    window: &X11Surface,
) -> jlong {
    let loc = window.geometry().loc;

    let mut best: Option<(&Box<WlcWindow>, i64)> = None;
    for toplevel in toplevels {
        let x11 = match toplevel.as_ref() {
            WlcWindow::X11(w) => w,
            WlcWindow::Xdg(_) => continue,
        };
        let geo = x11.geometry();
        if geo.contains(loc) {
            return get_handle(toplevels, toplevel);
        }
        let center = geo.loc + geo.size.downscale(2).to_point();
        let dx = (center.x - loc.x) as i64;
        let dy = (center.y - loc.y) as i64;
        let dist = dx * dx + dy * dy;
        if best.is_none_or(|(_, d)| dist < d) {
            best = Some((toplevel, dist));
        }
    }

    match best {
        Some((toplevel, _)) => get_handle(toplevels, toplevel),
        None => 0,
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    popupOffset")]
pub extern "system" fn popupOffset<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);
    let mut offset: [jint; 2] = [0, 0];

    match jptr_to_popup(handle) {
        WlcPopup::Xdg(popup) => {
            popup.with_cached_state(|state| {
                if let Some(pos) =
                    state.last_acked.map(|c| c.state.geometry.loc)
                {
                    offset = [pos.x, pos.y];
                }
            });
        }
        // An X11 override-redirect window has only an absolute X11 geometry;
        // the parent-relative offset is its origin minus the parent's origin,
        // both in the same X11 coordinate space.
        WlcPopup::X11(window) => {
            let parent =
                x11_override_parent(&instance.bridge.toplevels, window);
            if parent != 0
                && let WlcWindow::X11(p) = jptr_to_toplevel(parent)
            {
                let off = window.geometry().loc - p.geometry().loc;
                offset = [off.x, off.y];
            }
        }
    }

    let array = env.new_int_array(2).unwrap();
    env.set_int_array_region(&array, 0, &offset).unwrap();
    array.into_raw()
}

fn get_or_create_surface<'l>(
    env: &mut JNIEnv<'l>,
    state: &mut BridgeState,
    bridge_obj: &JObject<'l>,
    surface: &WlSurface,
) -> JObject<'l> {
    let handle = insert_get_handle(&mut state.surfaces, surface);
    let sig = "(J)Ldev/evvie/waylandcraft/bridge/WLCSurface;";
    unsafe {
        env.call_method_unchecked(
            bridge_obj,
            (WaylandCraftBridge_class, "getOrCreateSurface", sig),
            ReturnType::Object,
            &[jvalue { j: handle }],
        )
        .unwrap()
        .l()
        .unwrap()
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    updateSurfaceTree")]
pub extern "system" fn updateSurfaceTree<'l>(
    mut env: JNIEnv<'l>,
    bridge_obj: JObject<'l>,
    root: JObject<'l>,
) -> jobject {
    let instance_ptr: jlong = env
        .get_field_unchecked(
            &bridge_obj,
            (WaylandCraftBridge_class, "instance", "J"),
            ReturnType::Primitive(Primitive::Long),
        )
        .unwrap()
        .j()
        .unwrap();

    let instance = jptr_to_instance(instance_ptr);

    let handle: jlong = env
        .get_field_unchecked(
            &root,
            (WLCSurface_class, "handle", "J"),
            ReturnType::Primitive(Primitive::Long),
        )
        .unwrap()
        .j()
        .unwrap();

    let surface = match jptr_to_wlsurface(handle) {
        Some(s) if s.is_alive() => s,
        _ => return JObject::null().into_raw(),
    };

    let mut last_child: JObject = JObject::null();

    with_surface_tree_upward(
        &surface,
        None,
        |surface, _data, _parent| {
            TraversalAction::DoChildren(Some(surface.clone()))
        },
        |surface, data, parent| {
            let obj = get_or_create_surface(
                &mut env,
                &mut instance.bridge,
                &bridge_obj,
                surface,
            );

            // Set the WLCSurface parentHandle
            let parent_handle = if let Some(p) = parent {
                insert_get_handle(&mut instance.bridge.surfaces, p)
            } else {
                0
            };
            env.set_field_unchecked(
                &obj,
                (WLCSurface_class, "parentHandle", "J"),
                JValue::Long(parent_handle),
            )
            .unwrap();

            // Set last child to point to this current surface
            if !last_child.as_raw().is_null() {
                env.set_field_unchecked(
                    &last_child,
                    (
                        WLCSurface_class,
                        "nextChild",
                        "Ldev/evvie/waylandcraft/bridge/WLCSurface;",
                    ),
                    JValue::Object(&obj),
                )
                .unwrap();
            }

            // Set this surfaces nextChild to null
            env.set_field_unchecked(
                &obj,
                (
                    WLCSurface_class,
                    "nextChild",
                    "Ldev/evvie/waylandcraft/bridge/WLCSurface;",
                ),
                JValue::Object(&JObject::null()),
            )
            .unwrap();

            // Set this surfaces prevChild to the last child
            env.set_field_unchecked(
                &obj,
                (
                    WLCSurface_class,
                    "prevChild",
                    "Ldev/evvie/waylandcraft/bridge/WLCSurface;",
                ),
                JValue::Object(&last_child),
            )
            .unwrap();

            // Mark this surface as visited
            env.set_field_unchecked(
                &obj,
                (WLCSurface_class, "visited", "Z"),
                JValue::Bool(1),
            )
            .unwrap();

            // Set subsurface location
            let (sx, sy) = if data.cached_state.has::<SubsurfaceCachedState>() {
                let mut subattr_guard =
                    data.cached_state.get::<SubsurfaceCachedState>();
                let subattr = subattr_guard.deref_mut().current();
                (subattr.location.x, subattr.location.y)
            } else {
                (0, 0)
            };

            env.set_field_unchecked(
                &obj,
                (WLCSurface_class, "xoff", "I"),
                JValue::Int(sx),
            )
            .unwrap();

            env.set_field_unchecked(
                &obj,
                (WLCSurface_class, "yoff", "I"),
                JValue::Int(sy),
            )
            .unwrap();

            last_child = obj;
        },
        |_surface, _data, _parent| true,
    );

    last_child.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    pointerMotion")]
pub extern "system" fn pointerMotion<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    x: jdouble,
    y: jdouble,
) {
    let instance = jptr_to_instance(ptr);
    instance.state.seat.pointer_motion(x, y);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    pointerMotionFocus")]
pub extern "system" fn pointerMotionFocus<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
    x: jdouble,
    y: jdouble,
) {
    let instance = jptr_to_instance(ptr);
    let surface = jptr_to_wlsurface(handle);

    instance.state.seat.pointer_motion_focus(surface, x, y);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    pointerRelMotion")]
pub extern "system" fn pointerRelMotion<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    dx: jdouble,
    dy: jdouble,
) {
    let instance = jptr_to_instance(ptr);

    instance.state.seat.pointer_relative_motion(dx, dy);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    maybePointerLock")]
pub extern "system" fn maybePointerLock<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) -> jboolean {
    let instance = jptr_to_instance(ptr);
    let surface = match jptr_to_wlsurface(handle) {
        Some(s) => s,
        None => return 0,
    };

    instance.state.seat.pointer_lock(&surface) as jboolean
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    pointerUnlock")]
pub extern "system" fn pointerUnlock<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);

    instance.state.seat.pointer_unlock()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    pointerLeave")]
pub extern "system" fn pointerLeave<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    instance.state.seat.pointer_motion_focus(None, 0.0, 0.0);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    pointerButton")]
pub extern "system" fn pointerButton<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    button: jint,
    state: jint,
) -> jint {
    let instance = jptr_to_instance(ptr);

    let state = match state {
        0 => ButtonState::Released,
        1 => ButtonState::Pressed,
        _ => unreachable!(),
    };

    instance.state.seat.pointer_button(button as u32, state) as jint
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    pointerAxis")]
pub extern "system" fn pointerAxis<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    axis: jint,
    value: jdouble,
) {
    let instance = jptr_to_instance(ptr);

    let axis = match axis {
        0 => Axis::VerticalScroll,
        1 => Axis::HorizontalScroll,
        _ => {
            return;
        }
    };

    instance.state.seat.pointer_axis(axis, value);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    cursorShape")]
pub extern "system" fn cursorShape<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jint {
    let instance = jptr_to_instance(ptr);

    match instance.state.seat.cursor_shape {
        Some(shape) => shape as jint,
        None => -1,
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    keyboardFocus")]
pub extern "system" fn keyboardFocus<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) {
    let instance = jptr_to_instance(ptr);
    let window: Option<WlcWindow> = if handle != 0 {
        Some(jptr_to_toplevel(handle).clone())
    } else {
        None
    };

    let surface = window.as_ref().and_then(|w| w.wl_surface());

    // Only xdg toplevels carry the xdg Activated state; an X11 window is
    // activated and raised separately, below, via sync_x11_focus.
    let toplevel: Option<ToplevelSurface> = match &window {
        Some(WlcWindow::Xdg(t)) => Some(t.clone()),
        _ => None,
    };

    // Update the client gaining keyboard focus with the clipboard contents
    let client = surface.as_ref().and_then(|s| s.client());
    instance.state.data.update_clipboard_client(client);

    match surface {
        Some(s) => instance.state.seat.keyboard_focus(s),
        None => instance.state.seat.keyboard_unfocus(),
    };

    instance
        .state
        .xdg_state
        .toplevel_surfaces()
        .iter()
        .for_each(|t| {
            t.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Activated);
            });
        });

    if let Some(t) = toplevel {
        t.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        })
    }

    instance
        .state
        .xdg_state
        .toplevel_surfaces()
        .iter()
        .for_each(|t| {
            t.send_pending_configure();
        });

    // X11 equivalent of the xdg Activated handling above: activate and raise
    // the focused X11 window, deactivate the previously focused one. No-op when
    // focus stays put or never involves an X11 window.
    crate::xwayland::sync_x11_focus(&mut instance.state, window.as_ref());
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    keyboardActivate")]
pub extern "system" fn keyboardActivate<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    instance.state.seat.activate_keyboard();
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    keyboardDeactivate")]
pub extern "system" fn keyboardDeactivate<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    instance.state.seat.deactivate_keyboard();
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    keyboardInput")]
pub extern "system" fn keyboardInput<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    scancode: jint,
    action: jint,
) {
    let instance = jptr_to_instance(ptr);

    let scancode = scancode as u32;
    let action = match action {
        0 => KeyState::Released,
        1 => KeyState::Pressed,
        _ => {
            return;
        }
    };

    instance.state.seat.keyboard_key(scancode, action);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    keyboardUpdate")]
pub extern "system" fn keyboardUpdate<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    scancode: jint,
    press: jboolean,
) {
    let instance = jptr_to_instance(ptr);
    instance
        .state
        .seat
        .keyboard_update_xkb(scancode as u32, press == JNI_TRUE);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    fullscreened")]
pub extern "system" fn fullscreened<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);

    let mut handles: Vec<jlong> = vec![];
    for toplevel in instance.state.xdg_state.toplevel_surfaces() {
        let fullscreen = toplevel.with_committed_state(|state| {
            state
                .map(|s| s.states.contains(xdg_toplevel::State::Fullscreen))
                .unwrap_or(false)
        });
        if !fullscreen {
            continue;
        }

        let handle = insert_get_handle(
            &mut instance.bridge.toplevels,
            &WlcWindow::Xdg(toplevel.clone()),
        );
        handles.push(handle);
    }

    let array = env.new_long_array(handles.len() as jsize).unwrap();
    env.set_long_array_region(&array, 0, &handles).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    outputSize")]
pub extern "system" fn outputSize<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) -> jarray {
    let instance = jptr_to_instance(handle);

    let size = instance.state.output.size();
    let size: [jint; 2] = [size.w, size.h];

    let array = env.new_int_array(2).unwrap();
    env.set_int_array_region(&array, 0, &size).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    outputBounds")]
pub extern "system" fn outputBounds<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) -> jarray {
    let instance = jptr_to_instance(handle);

    let bounds = instance.state.output.bounds();
    let bounds: [jint; 2] = [bounds.w, bounds.h];

    let array = env.new_int_array(2).unwrap();
    env.set_int_array_region(&array, 0, &bounds).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    outputResize")]
pub extern "system" fn outputResize<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    width: jint,
    height: jint,
) {
    let instance = jptr_to_instance(handle);
    let size = instance.state.output.size();
    let width_changed = size.w != width;
    let height_changed = size.h != height;

    if width <= 0 || height <= 0 {
        return;
    }

    if !width_changed && !height_changed {
        return;
    }

    instance.state.output.resize(width, height);

    for toplevel in instance.state.xdg_state.toplevel_surfaces() {
        toplevel.with_pending_state(|state| {
            let fullscreen =
                state.states.contains(xdg_toplevel::State::Fullscreen);
            if fullscreen {
                state.size = Some(Size::new(width, height));
            }
        });
        toplevel.send_pending_configure();
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    outputSetBounds")]
pub extern "system" fn outputSetBounds<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    width: jint,
    height: jint,
) {
    let instance = jptr_to_instance(handle);
    let bounds = instance.state.output.bounds();
    let width_changed = bounds.w != width;
    let height_changed = bounds.h != height;

    if width <= 0 || height <= 0 {
        return;
    }

    if !width_changed && !height_changed {
        return;
    }

    instance.state.output.set_bounds(width, height);

    for toplevel in instance.state.xdg_state.toplevel_surfaces() {
        toplevel.with_pending_state(|state| {
            let maximized =
                state.states.contains(xdg_toplevel::State::Maximized);
            if maximized {
                state.size = Some(Size::new(width, height));
            }
        });
        toplevel.send_pending_configure();
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    checkInputRegion")]
pub extern "system" fn checkInputRegion<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    x: jdouble,
    y: jdouble,
) -> jboolean {
    let surface = match jptr_to_wlsurface(handle) {
        Some(s) => s,
        None => return 0,
    };

    let point: Point<f64, Logical> = Point::new(x, y);

    with_states(&surface, |data| {
        let mut attr_guard = data.cached_state.get::<SurfaceAttributes>();
        let attr = attr_guard.deref_mut().current();
        if let Some(r) = &attr.input_region {
            r.contains(point.to_i32_floor())
        } else {
            true
        }
    }) as jboolean
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    surfaceXDGGeometry")]
pub extern "system" fn surfaceXDGGeometry<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) -> jarray {
    let surface = match jptr_to_wlsurface(handle) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let geometry: Option<[jint; 4]> = with_states(&surface, |states| {
        let mut guard = states.cached_state.get::<SurfaceCachedState>();
        guard
            .current()
            .geometry
            .map(|r| [r.loc.x, r.loc.y, r.size.w, r.size.h])
    });

    if let Some(geometry) = geometry {
        let array = env.new_int_array(4).unwrap();
        env.set_int_array_region(&array, 0, &geometry).unwrap();
        array.into_raw()
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelTitle")]
pub extern "system" fn toplevelTitle<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) -> jstring {
    let title = match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => {
            let surface = toplevel.wl_surface();
            with_states(surface, |states| {
                let attr_guard = states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap();
                attr_guard.title.clone()
            })
        }
        WlcWindow::X11(window) => {
            let title = window.title();
            (!title.is_empty()).then_some(title)
        }
    };

    if let Some(title) = title {
        env.new_string(&title).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelAppID")]
pub extern "system" fn toplevelAppID<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) -> jstring {
    let app_id = match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => {
            let surface = toplevel.wl_surface();
            with_states(surface, |states| {
                let attr_guard = states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap();
                attr_guard.app_id.clone()
            })
        }
        WlcWindow::X11(window) => {
            let class = window.class();
            (!class.is_empty()).then_some(class)
        }
    };

    if let Some(app_id) = app_id {
        env.new_string(&app_id).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelResize")]
pub extern "system" fn toplevelResize<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    width: jint,
    height: jint,
    interactive: jboolean,
) {
    match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => {
            toplevel.with_pending_state(|state| {
                state.size = Some(Size::new(width, height));
                state.states.unset(xdg_toplevel::State::Maximized);
                state.states.unset(xdg_toplevel::State::Fullscreen);
                if interactive == JNI_TRUE {
                    state.states.set(xdg_toplevel::State::Resizing);
                } else {
                    state.states.unset(xdg_toplevel::State::Resizing);
                }
            });
            toplevel.send_pending_configure();
        }
        WlcWindow::X11(window) => {
            let mut rect = window.geometry();
            rect.size = Size::new(width, height);
            let _ = window.configure(Some(rect));
        }
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelResizeOvr")]
pub extern "system" fn toplevelResizeOvr<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    width: jint,
    height: jint,
) {
    match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => {
            toplevel.with_pending_state(|state| {
                state.size = Some(Size::new(width, height));
                state.states.unset(xdg_toplevel::State::Resizing);
            });
            toplevel.send_pending_configure();
        }
        WlcWindow::X11(window) => {
            let mut rect = window.geometry();
            rect.size = Size::new(width, height);
            let _ = window.configure(Some(rect));
        }
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelClose")]
pub extern "system" fn toplevelClose<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) {
    match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => toplevel.send_close(),
        WlcWindow::X11(window) => {
            let _ = window.close();
        }
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelMaximize")]
pub extern "system" fn toplevelMaximize<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) {
    let instance = jptr_to_instance(ptr);

    match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => {
            toplevel.with_pending_state(|state| {
                if state.states.contains(xdg_toplevel::State::Fullscreen) {
                    return;
                }
                let output = &instance.state.output;
                state.size = Some(output.bounds());
                state.states.set(xdg_toplevel::State::Maximized);
            });
            toplevel.send_configure();
        }
        WlcWindow::X11(window) => {
            if window.is_fullscreen() {
                return;
            }
            let _ = window.set_maximized(true);
        }
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    toplevelFullscreen")]
pub extern "system" fn toplevelFullscreen<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    _ptr: jlong,
    handle: jlong,
    width: jint,
    height: jint,
) {
    match jptr_to_toplevel(handle) {
        WlcWindow::Xdg(toplevel) => {
            // Fullscreen state, but keep the window at its current size instead
            // of expanding to the output - so it stays normal-sized and
            // resizable.
            toplevel.with_pending_state(|state| {
                state.size = Some(Size::new(width, height));
                state.states.set(xdg_toplevel::State::Fullscreen);
            });
            toplevel.send_configure();
        }
        WlcWindow::X11(window) => {
            // Match the Xdg arm: set the fullscreen state but keep the window
            // at its current size instead of expanding to the output.
            let _ = window.set_fullscreen(true);
            let mut rect = window.geometry();
            rect.size = Size::new(width, height);
            let _ = window.configure(Some(rect));
        }
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    freeSurface")]
pub extern "system" fn freeSurface<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) {
    let instance = jptr_to_instance(ptr);
    remove_element(&mut instance.bridge.surfaces, handle);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    freeToplevel")]
pub extern "system" fn freeToplevel<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) {
    let instance = jptr_to_instance(ptr);
    remove_element(&mut instance.bridge.toplevels, handle);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    freePopup")]
pub extern "system" fn freePopup<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
) {
    let instance = jptr_to_instance(ptr);
    remove_element(&mut instance.bridge.popups, handle);
}

#[allow(non_upper_case_globals)]
const RawDesktopEntry_class: &str =
    "dev/evvie/waylandcraft/desktop/RawDesktopEntry";

fn raw_desktop_entry_to_java<'l>(
    env: &mut JNIEnv<'l>,
    entry: &RawDesktopEntry,
) -> JObject<'l> {
    macro_rules! to_jstr {
        ($s:expr) => {
            env.new_string($s).unwrap()
        };
    }
    macro_rules! nullstr {
        () => {
            unsafe { JString::from_raw(std::ptr::null_mut()) }
        };
    }
    macro_rules! to_jstr_opt {
        ($s:expr) => {
            match $s {
                Some(s) => to_jstr!(s),
                None => nullstr!(),
            }
        };
    }

    let app_id: JString<'l> = to_jstr!(&entry.app_id);
    let name: JString<'l> = to_jstr_opt!(&entry.name);
    let generic_name: JString<'l> = to_jstr_opt!(&entry.generic_name);
    let exec: JString<'l> = to_jstr_opt!(&entry.exec);
    let exec_terminal: jboolean = entry.exec_terminal as jboolean;
    let comment: JString<'l> = to_jstr_opt!(&entry.comment);
    let visible: jboolean = entry.visible as jboolean;
    let icon_path: JString<'l> = to_jstr_opt!(&entry.icon_path);

    let keywords: Vec<JString<'l>> =
        entry.keywords.iter().map(|k| to_jstr!(k)).collect();
    let kw_array = env
        .new_object_array(
            keywords.len() as jsize,
            "java/lang/String",
            JObject::null(),
        )
        .unwrap();
    for (i, kw) in keywords.iter().enumerate() {
        env.set_object_array_element(&kw_array, i as jsize, kw)
            .unwrap();
    }

    let categories: Vec<JString<'l>> =
        entry.categories.iter().map(|c| to_jstr!(c)).collect();
    let cat_array = env
        .new_object_array(
            categories.len() as jsize,
            "java/lang/String",
            JObject::null(),
        )
        .unwrap();
    for (i, cat) in categories.iter().enumerate() {
        env.set_object_array_element(&cat_array, i as jsize, cat)
            .unwrap();
    }

    let str_sig = "Ljava/lang/String;";
    let str_arr_sig = "[Ljava/lang/String;";
    let mut ctor_sig = String::new();
    ctor_sig += "(";
    ctor_sig += str_sig; // appId
    ctor_sig += str_sig; // name
    ctor_sig += str_sig; // genericName
    ctor_sig += str_sig; // exec
    ctor_sig += "Z"; // execTerminal
    ctor_sig += str_sig; // comment
    ctor_sig += str_arr_sig; // keywords
    ctor_sig += str_arr_sig; // categories
    ctor_sig += "Z"; // visible
    ctor_sig += str_sig; // iconPath
    ctor_sig += ")V";

    let ctor_args = [
        JValue::Object(&app_id),
        JValue::Object(&name),
        JValue::Object(&generic_name),
        JValue::Object(&exec),
        JValue::Bool(exec_terminal),
        JValue::Object(&comment),
        JValue::Object(&kw_array),
        JValue::Object(&cat_array),
        JValue::Bool(visible),
        JValue::Object(&icon_path),
    ];

    env.new_object(RawDesktopEntry_class, ctor_sig, &ctor_args)
        .unwrap()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    loadDesktopEntry")]
pub extern "system" fn loadDesktopEntry<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    jpath: JString<'l>,
) -> jobject {
    let instance = jptr_to_instance(ptr);
    let path: String =
        unsafe { env.get_string_unchecked(&jpath).unwrap() }.into();
    let path: PathBuf = path.into();
    let entry = match instance.xdg.load_entry(path) {
        Some(e) => e,
        None => return std::ptr::null_mut(),
    };

    raw_desktop_entry_to_java(&mut env, &entry).into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    resolveAppID")]
pub extern "system" fn resolveAppID<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    app_id: JString<'l>,
) -> jstring {
    let instance = jptr_to_instance(ptr);
    let app_id: String =
        unsafe { env.get_string_unchecked(&app_id).unwrap() }.into();

    match instance.xdg.resolve_app_id(&app_id) {
        Some(id) => env.new_string(&id).unwrap().into_raw(),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    loadDesktopEntries")]
pub extern "system" fn loadDesktopEntries<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);
    let entries = instance.xdg.get_raw_entries();
    let entries: Vec<JObject<'l>> = entries
        .iter()
        .map(|e| raw_desktop_entry_to_java(&mut env, e))
        .collect();

    let array = env
        .new_object_array(
            entries.len() as jsize,
            RawDesktopEntry_class,
            JObject::null(),
        )
        .unwrap();

    for (i, ent) in entries.iter().enumerate() {
        env.set_object_array_element(&array, i as jsize, ent)
            .unwrap();
    }

    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    renderSVG")]
pub extern "system" fn renderSVG<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    path: JString<'l>,
    width: jint,
    height: jint,
    ptr: jlong,
) -> jboolean {
    let path: String =
        unsafe { env.get_string_unchecked(&path).unwrap() }.into();
    let path: PathBuf = path.into();
    let data = (ptr as usize) as *mut u8;
    let width = width as u32;
    let height = height as u32;

    render_svg(path, width, height, data).is_some() as jboolean
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    execApp")]
pub extern "system" fn execApp<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    app_id: JString<'l>,
) -> jboolean {
    let instance = jptr_to_instance(ptr);
    let app_id: String =
        unsafe { env.get_string_unchecked(&app_id).unwrap() }.into();

    let mut env_vars = vec![
        ("WAYLAND_DISPLAY".into(), instance.state.socket.clone()),
        ("QT_QPA_PLATFORM".into(), "wayland".into()),
        ("ELECTRON_OZONE_PLATFORM_HINT".into(), "auto".into()),
        ("GDK_BACKEND".into(), "wayland".into()),
    ];

    // process::spawn strips DISPLAY then applies the caller's env, so X11 apps
    // only reach Xwayland if DISPLAY is in this vec.
    if let Some(display) = instance.state.xdisplay {
        env_vars.push(("DISPLAY".into(), format!(":{display}").into()));
    }

    instance.xdg.exec_app(app_id, env_vars) as jboolean
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    setKeymapDefault")]
pub extern "system" fn setKeymapDefault<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    instance.state.seat.change_keymap_to_default();
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    exportKeymap")]
pub extern "system" fn exportKeymap<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jstring {
    let instance = jptr_to_instance(ptr);
    let keymap_str = instance.state.seat.export_keymap();
    env.new_string(keymap_str).unwrap().into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    setKeymapFromStr")]
pub extern "system" fn setKeymapFromStr<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    keymap_str: JString<'l>,
) -> jboolean {
    let instance = jptr_to_instance(ptr);
    let keymap_str: String =
        unsafe { env.get_string_unchecked(&keymap_str).unwrap() }.into();

    instance.state.seat.change_keymap_from_str(keymap_str) as jboolean
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    checkDndRequest")]
pub extern "system" fn checkDndRequest<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jarray {
    let instance = jptr_to_instance(ptr);
    let request = match instance.state.data.check_dnd_request() {
        Some(r) => r,
        None => return std::ptr::null_mut(),
    };

    let serial = request as jint;
    let array = env.new_int_array(1).unwrap();
    env.set_int_array_region(&array, 0, &[serial]).unwrap();
    array.into_raw()
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    checkDndActive")]
pub extern "system" fn checkDndActive<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jboolean {
    let instance = jptr_to_instance(ptr);
    instance.state.data.dnd.is_some() as jboolean
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    dndCancel")]
pub extern "system" fn dndCancel<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    instance.state.dnd_cancel();
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    dndDrop")]
pub extern "system" fn dndDrop<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) {
    let instance = jptr_to_instance(ptr);
    instance.state.dnd_drop();
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    dndMotion")]
pub extern "system" fn dndMotion<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
    handle: jlong,
    x: jdouble,
    y: jdouble,
) {
    let instance = jptr_to_instance(ptr);
    let surface = jptr_to_wlsurface(handle);
    instance.state.dnd_motion(surface, x, y);
}

#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    dndIcon")]
pub extern "system" fn dndIcon<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jlong {
    let instance = jptr_to_instance(ptr);
    let dnd = match &instance.state.data.dnd {
        Some(d) => d,
        None => return 0,
    };
    match dnd.icon.as_ref() {
        Some(icon) => insert_get_handle(&mut instance.bridge.surfaces, icon),
        None => 0,
    }
}

// True while an X11 app's XDND drag is in flight onto WaylandCraft (Stage C).
// Polled each tick by WaylandCraft.java: a rising edge starts an in-world
// X11DNDGrab that ray-casts the cursor and drives the Wayland target; a falling
// edge ends it. The X11 source, not the in-world mouse, owns drop/cancel.
#[unsafe(export_name = "Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_\
    checkX11Dnd")]
pub extern "system" fn checkX11Dnd<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong,
) -> jboolean {
    let instance = jptr_to_instance(ptr);
    instance.state.xdnd_target_active() as jboolean
}
