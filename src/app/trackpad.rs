use iced::Subscription;

#[cfg(target_os = "macos")]
mod platform {
    use std::ptr::NonNull;
    use std::sync::{Mutex, Once};

    use block2::RcBlock;
    use iced::Subscription;
    use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
    use objc2_app_kit::{NSEvent, NSEventMask};

    static INSTALL: Once = Once::new();
    static OUTPUT: Mutex<Option<UnboundedSender<f64>>> = Mutex::new(None);

    pub(super) fn install() {
        INSTALL.call_once(|| {
            let handler = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
                let pointer = event.as_ptr();
                let delta = unsafe { event.as_ref() }.magnification();
                publish(delta);
                pointer
            });
            let monitor = unsafe {
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                    NSEventMask::Magnify,
                    &handler,
                )
            };
            if let Some(monitor) = monitor {
                std::mem::forget(monitor);
            }
        });
    }

    pub(super) fn subscription() -> Subscription<f64> {
        Subscription::run(events)
    }

    fn publish(delta: f64) {
        if !delta.is_finite() || delta == 0.0 {
            return;
        }
        if let Some(output) = OUTPUT
            .lock()
            .expect("trackpad output mutex poisoned")
            .as_mut()
        {
            let _ = output.unbounded_send(delta);
        }
    }

    fn events() -> UnboundedReceiver<f64> {
        let (output, events) = mpsc::unbounded();
        *OUTPUT.lock().expect("trackpad output mutex poisoned") = Some(output);
        events
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use iced::Subscription;

    pub(super) fn install() {}

    pub(super) fn subscription() -> Subscription<f64> {
        Subscription::none()
    }
}

pub(crate) fn install() {
    platform::install();
}

pub(crate) fn subscription() -> Subscription<f64> {
    platform::subscription()
}

pub(crate) fn magnification_factor(delta: f64) -> Option<f64> {
    (delta.is_finite() && delta != 0.0).then(|| delta.exp())
}

#[cfg(test)]
mod tests {
    use super::magnification_factor;

    #[test]
    fn native_magnification_maps_to_stable_zoom_factors() {
        assert!(magnification_factor(0.1).unwrap() > 1.0);
        assert!(magnification_factor(-0.1).unwrap() < 1.0);
        assert_eq!(magnification_factor(0.0), None);
        assert_eq!(magnification_factor(f64::NAN), None);
    }
}
