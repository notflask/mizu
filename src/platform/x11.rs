//! X11: touchpad pinch (XInput 2.4 gesture events) and pen pressure.
//!
//! mizu opens its own connection and selects XI2 events on the window.
//! Button events cannot be selected twice on a window (winit already has
//! them), so the pen keeps working through winit's mouse events and this
//! backend only adds its pressure (and whether the eraser end is used),
//! read from the tablet's "Abs Pressure" valuator on motion.

use std::any::Any;
use std::collections::HashMap;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;
use x11rb::connection::Connection;
use x11rb::protocol::xinput::{self, ConnectionExt as _, DeviceClassData, EventMask, XIEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::Event;

use super::{Emit, PlatformEvent};

/// XI 2.4 gesture event masks (not among x11rb's named constants).
const GESTURE_PINCH_BEGIN: u32 = 1 << 27;
const GESTURE_PINCH_UPDATE: u32 = 1 << 28;
const GESTURE_PINCH_END: u32 = 1 << 29;

/// A tablet tool's pressure axis.
#[derive(Clone, Copy, Debug)]
struct PressureAxis {
    number: u16,
    min: f64,
    max: f64,
    eraser: bool,
}

fn fp3232(v: xinput::Fp3232) -> f64 {
    v.integral as f64 + v.frac as f64 / 4_294_967_296.0
}

fn fp1616(v: i32) -> f32 {
    v as f32 / 65536.0
}

/// Value of valuator `number` in an event, if it is present.
fn valuator(mask: &[u32], values: &[xinput::Fp3232], number: u16) -> Option<f64> {
    let (word, bit) = (number as usize / 32, number as u32 % 32);
    if mask.get(word)? & (1 << bit) == 0 {
        return None;
    }
    // Values are packed in the order of the set bits.
    let before: u32 = mask[..word].iter().map(|m| m.count_ones()).sum::<u32>()
        + (mask[word] & ((1u32 << bit) - 1)).count_ones();
    values.get(before as usize).copied().map(fp3232)
}

pub fn init(window: &Window, emit: Emit) -> Option<Box<dyn Any>> {
    let win = match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => h.window as u32,
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => return None,
    };
    let scale = window.scale_factor() as f32;
    let (conn, _) = match x11rb::connect(None) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("x11 input: cannot connect: {e}");
            return None;
        }
    };
    let version = conn.xinput_xi_query_version(2, 4).ok()?.reply().ok()?;
    let gestures = (version.major_version, version.minor_version) >= (2, 4);

    // Tablet tools: devices with an "Abs Pressure" valuator.
    let pressure_atom = conn
        .intern_atom(true, b"Abs Pressure")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let devices = conn
        .xinput_xi_query_device(xinput::Device::ALL)
        .ok()?
        .reply()
        .ok()?;
    let mut axes: HashMap<u16, PressureAxis> = HashMap::new();
    for d in &devices.infos {
        let name = String::from_utf8_lossy(&d.name).to_lowercase();
        for c in &d.classes {
            if let DeviceClassData::Valuator(v) = &c.data {
                if pressure_atom != 0 && v.label == pressure_atom {
                    axes.insert(
                        d.deviceid,
                        PressureAxis {
                            number: v.number,
                            min: fp3232(v.min),
                            max: fp3232(v.max),
                            eraser: name.contains("eraser"),
                        },
                    );
                }
            }
        }
    }
    if axes.is_empty() && !gestures {
        log::info!("x11: no tablet and no XInput 2.4 gestures");
        return None;
    }

    let mut masks = Vec::new();
    if !axes.is_empty() {
        masks.push(EventMask {
            deviceid: xinput::Device::ALL.into(),
            mask: vec![XIEventMask::MOTION],
        });
    }
    if gestures {
        masks.push(EventMask {
            deviceid: xinput::Device::ALL_MASTER.into(),
            mask: vec![(GESTURE_PINCH_BEGIN | GESTURE_PINCH_UPDATE | GESTURE_PINCH_END).into()],
        });
    }
    if let Err(e) = conn.xinput_xi_select_events(win, &masks).map(|c| c.check()) {
        log::warn!("x11 input: cannot select events: {e}");
        return None;
    }
    let _ = conn.flush();
    log::info!(
        "x11: {} tablet tool(s), gestures {}",
        axes.len(),
        if gestures { "yes" } else { "no" }
    );

    let thread = std::thread::Builder::new()
        .name("mizu-x11-input".into())
        .spawn(move || {
            let mut pinch_last = 1.0f32;
            while let Ok(ev) = conn.wait_for_event() {
                match ev {
                    Event::XinputMotion(m) => {
                        let Some(axis) = axes.get(&m.deviceid) else {
                            continue;
                        };
                        let Some(v) = valuator(&m.valuator_mask, &m.axisvalues, axis.number) else {
                            continue;
                        };
                        let range = (axis.max - axis.min).max(1e-6);
                        let p = ((v - axis.min) / range).clamp(0.0, 1.0) as f32;
                        emit(PlatformEvent::PenPressure {
                            pressure: p,
                            eraser: axis.eraser,
                        });
                    }
                    Event::XinputGesturePinchBegin(g) => {
                        pinch_last = 1.0;
                        emit(PlatformEvent::PinchBegin {
                            pos: [fp1616(g.event_x) / scale, fp1616(g.event_y) / scale],
                        });
                    }
                    Event::XinputGesturePinchUpdate(g) => {
                        let s = fp1616(g.scale).max(0.01);
                        let delta = s / pinch_last - 1.0;
                        pinch_last = s;
                        emit(PlatformEvent::PinchUpdate {
                            delta,
                            pos: [fp1616(g.event_x) / scale, fp1616(g.event_y) / scale],
                        });
                    }
                    Event::XinputGesturePinchEnd(_) => emit(PlatformEvent::PinchEnd),
                    _ => {}
                }
            }
        })
        .ok()?;
    Some(Box::new(thread))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(v: f64) -> xinput::Fp3232 {
        xinput::Fp3232 {
            integral: v.floor() as i32,
            frac: ((v - v.floor()) * 4_294_967_296.0) as u32,
        }
    }

    #[test]
    fn valuators_are_packed_by_mask_bits() {
        // Axes 0, 1 and 4 present; pressure is axis 4 -> third value.
        let mask = [0b1_0011];
        let values = [fp(10.0), fp(20.0), fp(512.5)];
        assert_eq!(valuator(&mask, &values, 4), Some(512.5));
        assert_eq!(valuator(&mask, &values, 1), Some(20.0));
        assert_eq!(valuator(&mask, &values, 2), None);
        assert_eq!(valuator(&mask, &values, 40), None);
    }
}
