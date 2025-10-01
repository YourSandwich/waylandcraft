use crate::{WaylandCraft, wlc_init};
use jni::{
    objects::JClass,
    sys::{jlong, jstring},
    JNIEnv,
};

fn jptr_to_instance(ptr: jlong) -> &'static mut WaylandCraft<'static> {
    let ptr: *mut WaylandCraft = (ptr as usize) as *mut WaylandCraft;
    unsafe { &mut *ptr }
}

#[unsafe(no_mangle)]
pub extern "system"
fn Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_init<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>
) -> jlong {
    let instance = match wlc_init() {
        Ok(i) => i,
        Err(err) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                err.to_string()
            );
            return 0;
        }
    };

    let instance_box: Box<WaylandCraft> = Box::new(instance);
    let ptr: *mut WaylandCraft = Box::into_raw(instance_box);
    let addr: u64 = ptr.addr() as u64;
    addr as i64
}

#[unsafe(no_mangle)]
pub extern "system"
fn Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_update<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong
) {
    let instance = jptr_to_instance(ptr);
    instance.update();
}

#[unsafe(no_mangle)]
pub extern "system"
fn Java_dev_evvie_waylandcraft_bridge_WaylandCraftBridge_socket<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    ptr: jlong
) -> jstring {
    let instance = jptr_to_instance(ptr);
    let socket = instance.state.socket.to_str().unwrap();
    env.new_string(socket).unwrap().into_raw()
}
