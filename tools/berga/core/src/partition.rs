//! The only partitions berga will ever write. Same list as the Bergamota app (UserdataFlash.partitions), the web
//! flasher (src/lib/fastboot.ts) and Enxerto itself. Never modem, persist, bluetooth, dsp or a bootloader.

use crate::{Error, Result};

pub const ALLOWED: [&str; 3] = ["boot", "userdata", "logo"];

/// Hard refusal of every other name.
pub fn check(name: &str) -> Result<&'static str> {
    ALLOWED.iter().copied().find(|p| *p == name).ok_or_else(|| Error::Refused(format!("partition {name:?}: berga only writes {}", ALLOWED.join(", "))))
}

/// Validates an image for `partition` from its size and first bytes (app rules: boot = Android boot image of
/// 1-64 MiB, logo 1-32 MiB, userdata at least 64 MiB).
pub fn validate_image(partition: &str, size: u64, first: &[u8]) -> Result<()> {
    check(partition)?;
    const MIB: u64 = 1 << 20;
    match partition {
        "boot" => {
            if !(MIB..=64 * MIB).contains(&size) {
                return Err(Error::Refused("a boot image must be 1-64 MiB".into()));
            }
            if !first.starts_with(b"ANDROID!") {
                return Err(Error::Refused("not an Android boot image (no ANDROID! header)".into()));
            }
        }
        "logo" if !(MIB..=32 * MIB).contains(&size) => return Err(Error::Refused("a logo image must be 1-32 MiB".into())),
        "userdata" if size < 64 * MIB => return Err(Error::Refused(format!("{size} bytes (< 64 MiB): not a root image"))),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_three() {
        for p in ALLOWED {
            assert!(check(p).is_ok());
        }
        for p in ["modem", "persist", "bluetooth", "dsp", "abl", "xbl", "Boot", "boot ", "userdata\0", "", "recovery", "vbmeta"] {
            assert!(check(p).is_err(), "{p:?}");
        }
        assert!(validate_image("boot", 2 << 20, b"ANDROID!....").is_ok());
        assert!(validate_image("boot", 2 << 20, b"\x7fELF").is_err());
        assert!(validate_image("userdata", 1 << 20, b"").is_err());
        assert!(validate_image("modem", 100 << 20, b"").is_err());
    }
}
