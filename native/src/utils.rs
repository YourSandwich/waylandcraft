use smithay::utils::SERIAL_COUNTER;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn new_serial() -> u32 {
    SERIAL_COUNTER.next_serial().into()
}

pub fn get_time() -> u32 {
    let time: u128 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    time as u32
}

pub fn to_fixed(v: f64) -> i32 {
    (v * 256.0) as i32
}

pub fn to_fixed2(v1: f64, v2: f64) -> (i32, i32) {
    (to_fixed(v1), to_fixed(v2))
}
