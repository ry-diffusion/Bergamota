//! berga-core: everything the Bergamota host tools share, with no UI and no I/O of its own (sans-IO).
//!
//! * [`proto`]: the berga USB protocol frames (docs/BERGA_USB.md); [`client`]: request/response/event matching
//!   with the 15 s timeout; [`session`]: a blocking driver of the client over any [`transport::Transport`].
//! * [`server`]: the phone side of the daemon ops (shared by the phone daemon `bergad` and [`fake`]).
//! * [`fake`]: a complete fake phone (kernel ops + daemon ops) for tests, `berga --fake` and the web demo.
//! * [`fastboot`], [`sparse`], [`bootimg`], [`partition`]: flashing and booting, with the hard partition allowlist.
//! * [`queue`], [`appfiles`], [`presets`], [`metrics`]: the Bergamota app's queue and JSON files, kept compatible.
//! * `usb` (feature `usb`, native only): nusb transports for the berga interface and fastboot.
//!
//! A UI (CLI, GPUI app, web page) only ever sees plain structs, `serde_json::Value`s and [`client::Incoming`] events.

pub mod appfiles;
pub mod bootimg;
pub mod client;
pub mod fake;
pub mod fastboot;
pub mod metrics;
pub mod partition;
pub mod presets;
pub mod proto;
pub mod queue;
pub mod server;
pub mod session;
pub mod sparse;
pub mod transport;
#[cfg(feature = "usb")]
pub mod usb;

/// USB identity of the phone's berga interface (kernel patch 1037).
pub mod ids {
    /// PLACEHOLDER: pid.codes VID with its *test* PID 0x0001, which the kernel has presented since the
    /// USB-Ethernet bring-up. Not ours to ship; a dedicated pid.codes PID is proposed in docs/BERGA_USB.md.
    /// Matching goes by the interface class triple below, so changing the PID later only touches these two lines.
    pub const VID: u16 = 0x1209;
    pub const PID: u16 = 0x0001;
    pub const IFACE_CLASS: u8 = 0xff;
    pub const IFACE_SUBCLASS: u8 = 0x62; // 'b'
    pub const IFACE_PROTOCOL: u8 = 0x01;
    /// Enxerto / ABL fastboot: interface ff/42/03 (VID/PID differ between ABL and Enxerto).
    pub const FASTBOOT_CLASS: (u8, u8, u8) = (0xff, 0x42, 0x03);
}

/// Every phone command times out after this long (project rule).
pub const PHONE_TIMEOUT_MS: u64 = 15_000;

#[derive(Debug)]
pub enum Error {
    Proto(proto::ProtoError),
    Io(std::io::Error),
    /// The phone answered with an error frame / fastboot FAIL.
    Device(String),
    /// No answer within [`PHONE_TIMEOUT_MS`] (or the fastboot step's own limit).
    Timeout(String),
    /// Refused here before anything was sent (allowlists, validation).
    Refused(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Proto(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Device(e) => write!(f, "phone: {e}"),
            Error::Timeout(e) => write!(f, "timed out: {e}"),
            Error::Refused(e) => write!(f, "refused: {e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<proto::ProtoError> for Error {
    fn from(e: proto::ProtoError) -> Error {
        Error::Proto(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
