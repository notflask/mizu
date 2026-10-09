//! macOS: pen pressure and the eraser end (an `NSEvent` monitor), opening
//! documents from Finder, and the native parts of the title bar.
//!
//! Everything here runs on the main thread (winit's event loop thread).

use std::any::Any;
use std::cell::Cell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplicationWillFinishLaunchingNotification, NSColor, NSEvent, NSEventMask, NSEventSubtype,
    NSEventType, NSPointingDeviceType, NSView, NSWindow,
};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSObject,
    NSString, NSUserDefaults,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::{Emit, PlatformEvent};

/// The window's `NSWindow`.
fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: winit hands out a valid NSView for as long as the window lives,
    // and we are on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    view.window()
}

// ---------------------------------------------------------------------------
// title bar

/// Height of the title bar area (in points, = logical pixels) that the
/// content runs under with a full-size content view. 0 in full screen.
pub fn titlebar_height(window: &Window) -> f32 {
    let Some(w) = ns_window(window) else {
        return 28.0;
    };
    let frame = w.frame();
    let content = w.contentLayoutRect();
    (frame.size.height - content.size.height).max(0.0) as f32
}

/// Double-click on the title bar: what System Settings says (zoom by
/// default, minimise, or nothing).
pub fn title_double_click(window: &Window) {
    let Some(w) = ns_window(window) else { return };
    let action = NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str("AppleActionOnDoubleClick"))
        .map(|s| s.to_string());
    match action.as_deref() {
        Some("Minimize") => w.performMiniaturize(None),
        Some("None") | Some("Nothing") => {}
        _ => w.performZoom(None),
    }
}

/// Background of the window (shows while resizing), as sRGB.
pub fn set_window_background(window: &Window, rgb: [u8; 3]) {
    let Some(w) = ns_window(window) else { return };
    let c = |v: u8| v as f64 / 255.0;
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(c(rgb[0]), c(rgb[1]), c(rgb[2]), 1.0);
    w.setBackgroundColor(Some(&color));
}

// ---------------------------------------------------------------------------
// opening documents from Finder

static OPEN_SINK: OnceLock<Box<dyn Fn(PathBuf) + Send + Sync>> = OnceLock::new();

const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const K_AE_OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[name = "MizuOpenDocumentsHandler"]
    struct OpenHandler;

    impl OpenHandler {
        #[unsafe(method(handleOpen:withReply:))]
        fn handle_open(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            // SAFETY: documented method; AEKeyword is a 32-bit code.
            let list: Option<Retained<NSAppleEventDescriptor>> =
                unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
            let Some(list) = list else {
                return;
            };
            let n = list.numberOfItems();
            // A single file is sent as itself, not as a list.
            let items: Vec<Retained<NSAppleEventDescriptor>> = if n == 0 {
                vec![list]
            } else {
                (1..=n).filter_map(|i| list.descriptorAtIndex(i)).collect()
            };
            for item in items {
                let path = item
                    .fileURLValue()
                    .and_then(|u| u.path())
                    .map(|p| PathBuf::from(p.to_string()));
                if let (Some(p), Some(sink)) = (path, OPEN_SINK.get()) {
                    log::info!("open from Finder: {}", p.display());
                    sink(p);
                }
            }
        }
    }
);

/// Route Finder's "open document" requests (double-click, Open With, drop
/// on the Dock icon) to `sink`. Call before the event loop runs: the handler
/// is installed when the app is about to finish launching, which is when
/// AppKit expects it, so files that launched the app arrive too.
pub fn install_open_handler(sink: Box<dyn Fn(PathBuf) + Send + Sync>) {
    if OPEN_SINK.set(sink).is_err() {
        return;
    }
    let block = RcBlock::new(|_note: NonNull<NSNotification>| install_ae_handler());
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: the block is 'static and the observer token is kept alive by
    // the notification center for the life of the app.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
            None,
            &block,
        )
    };
    std::mem::forget(token);
}

fn install_ae_handler() {
    let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::alloc(), init] };
    let manager = NSAppleEventManager::sharedAppleEventManager();
    // SAFETY: the selector matches the handler's method; the handler is
    // leaked below, so it outlives the registration.
    unsafe {
        let _: () = msg_send![
            &manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleOpen:withReply:),
            forEventClass: K_CORE_EVENT_CLASS,
            andEventID: K_AE_OPEN_DOCUMENTS
        ];
    }
    std::mem::forget(handler);
}

// ---------------------------------------------------------------------------
// pen

struct PenMonitor {
    token: Retained<AnyObject>,
}

impl Drop for PenMonitor {
    fn drop(&mut self) {
        // SAFETY: the token came from addLocalMonitorForEventsMatchingMask.
        unsafe { NSEvent::removeMonitor(&self.token) };
    }
}

#[derive(Default)]
struct PenState {
    eraser: Cell<bool>,
    down: Cell<bool>,
}

/// Watch tablet events of this window: pressure and the eraser end. Pen
/// strokes are taken out of the normal mouse stream (they arrive as
/// `PlatformEvent`s instead); a mouse is not affected.
pub fn init(window: &Window, emit: Emit) -> Option<Box<dyn Any>> {
    MainThreadMarker::new()?;
    let ns_win = ns_window(window)?;
    let win_number = ns_win.windowNumber();
    let state = Rc::new(PenState::default());
    let mask = NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseDragged
        | NSEventMask::LeftMouseUp
        | NSEventMask::TabletPoint
        | NSEventMask::TabletProximity;
    let block = RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid event.
        let event: &NSEvent = unsafe { ev.as_ref() };
        let ty = event.r#type();
        if ty == NSEventType::TabletProximity {
            if event.isEnteringProximity() {
                state
                    .eraser
                    .set(event.pointingDeviceType() == NSPointingDeviceType::Eraser);
            }
            return ev.as_ptr();
        }
        let from_pen =
            ty == NSEventType::TabletPoint || event.subtype() == NSEventSubtype::TabletPoint;
        if !from_pen || event.windowNumber() != win_number {
            return ev.as_ptr();
        }
        let Some(view) = ns_win.contentView() else {
            return ev.as_ptr();
        };
        let loc = event.locationInWindow();
        let h = view.frame().size.height;
        let pos = [loc.x as f32, (h - loc.y) as f32];
        let pressure = event.pressure().clamp(0.0, 1.0);
        let out = match ty {
            NSEventType::LeftMouseDown => {
                state.down.set(true);
                PlatformEvent::PenDown {
                    pos,
                    pressure,
                    eraser: state.eraser.get(),
                }
            }
            NSEventType::LeftMouseUp => {
                state.down.set(false);
                PlatformEvent::PenUp
            }
            NSEventType::LeftMouseDragged => PlatformEvent::PenMove { pos, pressure },
            // Pure tablet points come in between; they matter only while
            // the pen touches.
            _ if state.down.get() => PlatformEvent::PenMove { pos, pressure },
            _ => return ev.as_ptr(),
        };
        emit(out);
        std::ptr::null_mut()
    });
    // SAFETY: the block lives as long as the monitor (the token holds it).
    let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) }?;
    log::info!("macOS: tablet monitor installed");
    Some(Box::new(PenMonitor { token }))
}
