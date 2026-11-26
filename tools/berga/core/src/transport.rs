//! A bulk IN/OUT pipe pair, blocking. Natively: `usb::UsbPipe` (nusb) or the fake phone. The browser does not use
//! this trait: its JavaScript calls WebUSB itself and feeds the sans-IO [`crate::client::Client`] / fastboot parser.

use std::time::Duration;

pub trait Transport {
    /// One OUT transfer.
    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()>;
    /// One IN transfer (ends at a short packet); `None` when nothing arrived within `timeout`.
    fn recv(&mut self, timeout: Duration) -> std::io::Result<Option<Vec<u8>>>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        (**self).send(bytes)
    }
    fn recv(&mut self, timeout: Duration) -> std::io::Result<Option<Vec<u8>>> {
        (**self).recv(timeout)
    }
}
