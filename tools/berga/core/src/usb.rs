//! Native USB (nusb, no libusb): find the phone's berga interface or a fastboot interface and use it as a
//! [`Transport`]. Interfaces are matched by class triple, so the (placeholder) VID/PID do not matter here.

use crate::ids;
use crate::transport::Transport;
use crate::{Error, Result};
use nusb::MaybeFuture;
use nusb::transfer::{Buffer, Bulk, In, Out, TransferError};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// The iOS kernel's berga vendor interface (kernel patch 1037).
    Berga,
    /// Fastboot; `bergamota` = Enxerto (VID 18d1 PID 4ee0 product "bergamota"), else Qualcomm ABL.
    Fastboot,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub kind: Kind,
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    pub product: Option<String>,
    pub interface: u8,
    info: nusb::DeviceInfo,
}

fn kind_of(c: (u8, u8, u8)) -> Option<Kind> {
    if c == (ids::IFACE_CLASS, ids::IFACE_SUBCLASS, ids::IFACE_PROTOCOL) {
        Some(Kind::Berga)
    } else if c == ids::FASTBOOT_CLASS {
        Some(Kind::Fastboot)
    } else {
        None
    }
}

fn usb_err(e: impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::other(format!("USB: {e}")))
}

/// Every berga or fastboot interface currently on the bus.
pub fn list() -> Result<Vec<Found>> {
    let mut out = Vec::new();
    for d in nusb::list_devices().wait().map_err(usb_err)? {
        for i in d.interfaces() {
            if let Some(kind) = kind_of((i.class(), i.subclass(), i.protocol())) {
                out.push(Found {
                    kind,
                    vid: d.vendor_id(),
                    pid: d.product_id(),
                    serial: d.serial_number().map(str::to_string),
                    product: d.product_string().map(str::to_string),
                    interface: i.interface_number(),
                    info: d.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// A claimed interface's bulk pipe pair.
pub struct UsbPipe {
    out: nusb::Endpoint<Bulk, Out>,
    inn: nusb::Endpoint<Bulk, In>,
    _iface: nusb::Interface,
    max_in: usize,
    pub found: Found,
}

impl UsbPipe {
    pub fn open(f: &Found, max_in: usize) -> Result<UsbPipe> {
        let dev = f.info.open().wait().map_err(usb_err)?;
        let iface = dev.claim_interface(f.interface).wait().map_err(|e| usb_err(format!("claim interface {}: {e} (another program using the phone? close the Bergamota app's fastboot / other berga)", f.interface)))?;
        let desc = iface.descriptor().ok_or_else(|| usb_err("no interface descriptor"))?;
        let mut a_in = None;
        let mut a_out = None;
        for ep in desc.endpoints() {
            if ep.transfer_type() == nusb::descriptors::TransferType::Bulk {
                if ep.address() & 0x80 != 0 { a_in.get_or_insert(ep.address()); } else { a_out.get_or_insert(ep.address()); }
            }
        }
        let (a_in, a_out) = a_in.zip(a_out).ok_or_else(|| usb_err("interface has no bulk IN/OUT pair"))?;
        Ok(UsbPipe {
            out: iface.endpoint::<Bulk, Out>(a_out).map_err(usb_err)?,
            inn: iface.endpoint::<Bulk, In>(a_in).map_err(usb_err)?,
            _iface: iface,
            max_in,
            found: f.clone(),
        })
    }

    /// The first berga interface, or a helpful error.
    pub fn berga() -> Result<UsbPipe> {
        let f = list()?.into_iter().find(|f| f.kind == Kind::Berga).ok_or_else(|| {
            Error::Device("no phone with the berga USB interface (needs an iOS boot with kernel patch 1037; try --fake to try berga without it)".into())
        })?;
        UsbPipe::open(&f, crate::proto::MAX_FRAME)
    }

    pub fn fastboot() -> Result<UsbPipe> {
        let f = list()?.into_iter().find(|f| f.kind == Kind::Fastboot).ok_or_else(|| Error::Device("no phone in fastboot".into()))?;
        UsbPipe::open(&f, 512)
    }
}

impl Transport for UsbPipe {
    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let c = self.out.transfer_blocking(Buffer::from(bytes.to_vec()), Duration::from_secs(15));
        c.status.map_err(|e| std::io::Error::other(format!("USB write: {e}")))
    }

    fn recv(&mut self, timeout: Duration) -> std::io::Result<Option<Vec<u8>>> {
        let mut buf = Buffer::new(self.max_in.div_ceil(512) * 512);
        buf.set_requested_len(buf.capacity());
        let c = self.inn.transfer_blocking(buf, timeout);
        match c.status {
            Ok(()) => {
                let mut v = c.buffer.into_vec();
                v.truncate(c.actual_len);
                Ok(Some(v))
            }
            Err(TransferError::Cancelled) => Ok(None),
            Err(e) => Err(std::io::Error::other(format!("USB read: {e}"))),
        }
    }
}
