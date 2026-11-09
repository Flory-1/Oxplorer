//! Phones and cameras on a USB cable (MTP), for the sidebar and the Home
//! page: looked for in the background every few seconds, so a phone shows
//! up soon after it is plugged in (and unlocked).

use super::*;
use ox_core::remote::mtp::Device;

/// How often the devices are looked for again.
const PHONES_REFRESH: Duration = Duration::from_secs(4);

#[derive(Default)]
pub struct Phones {
    pub devices: Vec<Device>,
    rx: Option<Receiver<Vec<Device>>>,
    at: Option<Instant>,
}

impl OxApp {
    pub(super) fn poll_phones(&mut self) {
        let phones = &mut self.phones;
        if let Some(rx) = &phones.rx {
            match rx.try_recv() {
                Ok(list) => {
                    phones.devices = list;
                    phones.rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(_) => phones.rx = None,
            }
        }
        let due = phones.at.is_none_or(|at| at.elapsed() >= PHONES_REFRESH);
        if due && phones.rx.is_none() && cfg!(windows) {
            phones.at = Some(Instant::now());
            let (tx, rx) = crossbeam_channel::bounded(1);
            let ctx = self.ctx.clone();
            let before = phones.devices.clone();
            std::thread::spawn(move || {
                let list = ox_core::remote::mtp::devices();
                // Only a change needs drawing again.
                if list != before {
                    ctx.request_repaint();
                }
                let _ = tx.send(list);
            });
            phones.rx = Some(rx);
        }
        if cfg!(windows) {
            // Look again even when nothing else wakes the app up.
            self.ctx.request_repaint_after(PHONES_REFRESH);
        }
    }
}
