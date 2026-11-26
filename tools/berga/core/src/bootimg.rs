//! Boot image repack: change the xnu boot-args of a built Bergamota boot.img without rebuilding Enxerto.
//! The Android header (v0-v2, what enxerto/build.sh makes) locates the ramdisk; the ramdisk is the
//! tools/mkpayload.py payload whose header holds the cmdline (608 bytes at 0x48, never compressed).

use crate::{Error, Result};

pub const PAYLOAD_MAGIC: &[u8; 8] = b"BERGAMOT"; // "BERGAMOTAXNU" truncated to char[8] by mkpayload.py
pub const CMDLINE_OFF: usize = 0x48;
pub const CMDLINE_LEN: usize = 608; // Enxerto's BOOT_LINE_LENGTH, NUL included

#[derive(Clone, Debug, PartialEq)]
pub struct BootImage {
    pub header_version: u32,
    pub page_size: u32,
    pub kernel_size: u32,
    pub ramdisk_offset: usize,
    pub ramdisk_size: u32,
    /// The Android header's own cmdline (Enxerto reads flags such as `bergamota=1` from it).
    pub header_cmdline: String,
    /// The xnu boot-args in the payload, if the ramdisk is a Bergamota payload.
    pub xnu_cmdline: Option<String>,
    pub payload_version: Option<u32>,
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn cstr(b: &[u8]) -> String {
    String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).to_string()
}

pub fn parse(img: &[u8]) -> Result<BootImage> {
    if img.len() < 1660 || !img.starts_with(b"ANDROID!") {
        return Err(Error::Refused("not an Android boot image (no ANDROID! header)".into()));
    }
    let header_version = le32(img, 40);
    if header_version > 2 {
        return Err(Error::Refused(format!("boot image header v{header_version}: berga handles v0-v2 (what enxerto/build.sh makes)")));
    }
    let (kernel_size, ramdisk_size, page_size) = (le32(img, 8), le32(img, 16), le32(img, 36));
    if !page_size.is_power_of_two() || !(2048..=65536).contains(&page_size) {
        return Err(Error::Refused(format!("bad page size {page_size}")));
    }
    let ps = page_size as usize;
    let ramdisk_offset = ps + (kernel_size as usize).div_ceil(ps) * ps;
    if ramdisk_offset + ramdisk_size as usize > img.len() {
        return Err(Error::Refused("ramdisk runs past the end of the image".into()));
    }
    let rd = &img[ramdisk_offset..ramdisk_offset + ramdisk_size as usize];
    let is_payload = rd.len() >= CMDLINE_OFF + CMDLINE_LEN && rd.starts_with(PAYLOAD_MAGIC);
    Ok(BootImage {
        header_version,
        page_size,
        kernel_size,
        ramdisk_offset,
        ramdisk_size,
        header_cmdline: cstr(&img[64..576]),
        xnu_cmdline: is_payload.then(|| cstr(&rd[CMDLINE_OFF..CMDLINE_OFF + CMDLINE_LEN])),
        payload_version: is_payload.then(|| le32(rd, 8)),
    })
}

/// A copy of `img` with the payload's xnu boot-args replaced (sizes and offsets unchanged).
pub fn with_xnu_cmdline(img: &[u8], cmdline: &str) -> Result<Vec<u8>> {
    let b = parse(img)?;
    if b.xnu_cmdline.is_none() {
        return Err(Error::Refused("this boot image has no Bergamota payload (empty ramdisk: Bergamota fastboot image?)".into()));
    }
    let c = cmdline.trim();
    if c.len() >= CMDLINE_LEN || c.contains('\0') {
        return Err(Error::Refused(format!("cmdline is {} bytes, Enxerto takes at most {}", c.len(), CMDLINE_LEN - 1)));
    }
    let mut out = img.to_vec();
    let at = b.ramdisk_offset + CMDLINE_OFF;
    out[at..at + CMDLINE_LEN].fill(0);
    out[at..at + c.len()].copy_from_slice(c.as_bytes());
    Ok(out)
}

/// A minimal v2-shaped boot image around `ramdisk` (tests, fixtures).
pub fn synth(ramdisk: &[u8]) -> Vec<u8> {
    let ps = 4096usize;
    let kernel = vec![0x11u8; 5000];
    let mut img = vec![0u8; ps];
    img[..8].copy_from_slice(b"ANDROID!");
    img[8..12].copy_from_slice(&(kernel.len() as u32).to_le_bytes());
    img[16..20].copy_from_slice(&(ramdisk.len() as u32).to_le_bytes());
    img[36..40].copy_from_slice(&(ps as u32).to_le_bytes());
    img[40..44].copy_from_slice(&2u32.to_le_bytes());
    img[64..74].copy_from_slice(b"bergamota=");
    img.extend_from_slice(&kernel);
    img.resize(ps + kernel.len().div_ceil(ps) * ps, 0);
    img.extend_from_slice(ramdisk);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repack_cmdline() {
        let mut payload = vec![0u8; 0x2a8 + 64];
        payload[..12].copy_from_slice(b"BERGAMOTAXNU"[..8].iter().chain(&1u32.to_le_bytes()).copied().collect::<Vec<_>>().as_slice());
        payload[CMDLINE_OFF..CMDLINE_OFF + 7].copy_from_slice(b"debug=0");
        let img = synth(&payload);
        let b = parse(&img).unwrap();
        assert_eq!(b.xnu_cmdline.as_deref(), Some("debug=0"));
        assert_eq!(b.payload_version, Some(1));
        let new = with_xnu_cmdline(&img, "  debug=0xc bergamotausb=2 ").unwrap();
        assert_eq!(new.len(), img.len());
        assert_eq!(parse(&new).unwrap().xnu_cmdline.as_deref(), Some("debug=0xc bergamotausb=2"));
        assert!(with_xnu_cmdline(&img, &"x".repeat(CMDLINE_LEN)).is_err());
        assert!(with_xnu_cmdline(&synth(&[]), "a").is_err());
        assert!(parse(b"ANDROID?").is_err());
    }
}
