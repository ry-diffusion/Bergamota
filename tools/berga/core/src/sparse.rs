//! Android sparse images, streamed (port of apps/web/src/lib/sparse.ts): parse the chunk table without reading
//! the data, split the image into pieces that fit the device's download buffer, and produce each piece's bytes.
//! A piece is itself a complete sparse image covering the whole partition (leading DONT_CARE, its chunks,
//! trailing DONT_CARE), which is what `fastboot -S` sends and what ABL and Enxerto accept.

use crate::{Error, Result};

pub const MAGIC: u32 = 0xed26_ff3a;
pub const RAW: u16 = 0xcac1;
pub const FILL: u16 = 0xcac2;
pub const SKIP: u16 = 0xcac3;
pub const CRC: u16 = 0xcac4;
pub const FILE_HDR: u64 = 28;
pub const CHUNK_HDR: u64 = 12;

/// Random access to the image (a file natively; a buffer in tests and in the browser).
pub trait Source {
    fn size(&self) -> u64;
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()>;
}

impl Source for &[u8] {
    fn size(&self) -> u64 {
        self.len() as u64
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let o = offset as usize;
        let src = self.get(o..o + buf.len()).ok_or_else(|| Error::Refused(format!("image truncated at {o}")))?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

impl Source for std::fs::File {
    fn size(&self) -> u64 {
        self.metadata().map(|m| m.len()).unwrap_or(0)
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buf)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    /// `blocks` blocks from `block`, data at `offset` in the source.
    Raw { block: u32, blocks: u32, offset: u64 },
    Fill { block: u32, blocks: u32, value: u32 },
}

impl Segment {
    pub fn block(&self) -> u32 {
        match *self {
            Segment::Raw { block, .. } | Segment::Fill { block, .. } => block,
        }
    }
    pub fn blocks(&self) -> u32 {
        match *self {
            Segment::Raw { blocks, .. } | Segment::Fill { blocks, .. } => blocks,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub sparse: bool,
    pub block_size: u32,
    pub total_blocks: u32,
    pub source_bytes: u64,
    pub segments: Vec<Segment>,
    /// Raw image only: zero padding of the last block.
    pub tail_pad: u64,
}

impl Image {
    pub fn expanded_bytes(&self) -> u64 {
        self.total_blocks as u64 * self.block_size as u64
    }
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// Reads the chunk table (never the data) and validates it completely.
pub fn parse(src: &mut dyn Source, raw_block_size: u32) -> Result<Image> {
    let size = src.size();
    let bad = |m: String| Error::Refused(format!("sparse image: {m}"));
    let mut h = [0u8; FILE_HDR as usize];
    if size < FILE_HDR || {
        src.read_at(0, &mut h)?;
        le32(&h, 0) != MAGIC
    } {
        let bs = raw_block_size as u64;
        let total = size.div_ceil(bs);
        return Ok(Image {
            sparse: false,
            block_size: raw_block_size,
            total_blocks: u32::try_from(total).map_err(|_| bad("raw image too large".into()))?,
            source_bytes: size,
            segments: if total > 0 { vec![Segment::Raw { block: 0, blocks: total as u32, offset: 0 }] } else { vec![] },
            tail_pad: total * bs - size,
        });
    }
    let (major, file_hdr, chunk_hdr) = (le16(&h, 4), le16(&h, 8) as u64, le16(&h, 10) as u64);
    let (block_size, total_blocks, total_chunks) = (le32(&h, 12), le32(&h, 16), le32(&h, 20));
    if major != 1 {
        return Err(bad(format!("unsupported version {major}")));
    }
    if file_hdr < FILE_HDR || chunk_hdr < CHUNK_HDR {
        return Err(bad("bad header sizes".into()));
    }
    if block_size == 0 || block_size % 4 != 0 {
        return Err(bad(format!("bad block size {block_size}")));
    }
    let bs = block_size as u64;
    let mut segments = Vec::new();
    let (mut off, mut block) = (file_hdr, 0u64);
    for i in 0..total_chunks {
        let mut c = [0u8; 16];
        src.read_at(off, &mut c[..CHUNK_HDR as usize])?;
        let (ty, blocks, total) = (le16(&c, 0), le32(&c, 4) as u64, le32(&c, 8) as u64);
        let data = off + chunk_hdr;
        let at = format!("chunk {i} (byte {off})");
        match ty {
            RAW => {
                if total != chunk_hdr + blocks * bs {
                    return Err(bad(format!("{at}: RAW size mismatch")));
                }
                if blocks > 0 {
                    segments.push(Segment::Raw { block: block as u32, blocks: blocks as u32, offset: data });
                }
            }
            FILL => {
                if total != chunk_hdr + 4 {
                    return Err(bad(format!("{at}: FILL size mismatch")));
                }
                src.read_at(data, &mut c[12..16])?;
                if blocks > 0 {
                    segments.push(Segment::Fill { block: block as u32, blocks: blocks as u32, value: le32(&c, 12) });
                }
            }
            SKIP if total != chunk_hdr => return Err(bad(format!("{at}: DONT_CARE size mismatch"))),
            SKIP => {}
            CRC if total != chunk_hdr + 4 || blocks != 0 => return Err(bad(format!("{at}: CRC32 chunk malformed"))),
            CRC => {}
            other => return Err(bad(format!("{at}: unknown chunk type 0x{other:x}"))),
        }
        block += blocks;
        off += total;
        if off > size {
            return Err(bad(format!("{at}: runs past the end of the file")));
        }
        if block > total_blocks as u64 {
            return Err(bad(format!("{at}: more blocks than the header declares")));
        }
    }
    if block != total_blocks as u64 {
        return Err(bad(format!("chunks cover {block} blocks, header says {total_blocks}")));
    }
    Ok(Image { sparse: true, block_size, total_blocks, source_bytes: size, segments, tail_pad: 0 })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Exact bytes this piece sends (the fastboot download size).
    pub bytes: u64,
    /// Send the source unchanged (it fits in one download).
    pub verbatim: bool,
    pub segments: Vec<Segment>,
}

fn count_chunks(img: &Image, segs: &[Segment]) -> u64 {
    let (mut n, mut at) = (0u64, 0u32);
    for s in segs {
        if s.block() > at {
            n += 1;
        }
        n += 1;
        at = s.block() + s.blocks();
    }
    n + (at < img.total_blocks) as u64
}

fn sparse_bytes(img: &Image, segs: &[Segment]) -> u64 {
    FILE_HDR + count_chunks(img, segs) * CHUNK_HDR + segs.iter().map(|s| if let Segment::Raw { blocks, .. } = s { *blocks as u64 * img.block_size as u64 } else { 4 }).sum::<u64>()
}

/// Splits the image into pieces of at most `max_bytes` each.
pub fn plan(img: &Image, max_bytes: u64) -> Result<Vec<Piece>> {
    if img.source_bytes <= max_bytes {
        return Ok(vec![Piece { bytes: img.source_bytes, verbatim: true, segments: img.segments.clone() }]);
    }
    let bs = img.block_size as u64;
    if max_bytes < FILE_HDR + 3 * CHUNK_HDR + bs {
        return Err(Error::Refused(format!("download size {max_bytes} is too small for {bs}-byte blocks")));
    }
    let mut pieces = Vec::new();
    let mut cur: Vec<Segment> = Vec::new();
    // worst case per piece: header, leading and trailing DONT_CARE; each segment adds its header, its data and
    // possibly a DONT_CARE for the gap before it
    let base = FILE_HDR + 2 * CHUNK_HDR;
    let mut used = base;
    let close = |cur: &mut Vec<Segment>, pieces: &mut Vec<Piece>| {
        if !cur.is_empty() {
            pieces.push(Piece { bytes: sparse_bytes(img, cur), verbatim: false, segments: std::mem::take(cur) });
        }
    };
    for seg in &img.segments {
        let mut seg = *seg;
        loop {
            let room = max_bytes.saturating_sub(used + 2 * CHUNK_HDR);
            match seg {
                Segment::Fill { .. } => {
                    if room < 4 {
                        close(&mut cur, &mut pieces);
                        used = base;
                        continue;
                    }
                    cur.push(seg);
                    used += 2 * CHUNK_HDR + 4;
                    break;
                }
                Segment::Raw { block, blocks, offset } => {
                    let fit = (room / bs).min(blocks as u64) as u32;
                    if fit == 0 {
                        close(&mut cur, &mut pieces);
                        used = base;
                        continue;
                    }
                    cur.push(Segment::Raw { block, blocks: fit, offset });
                    used += 2 * CHUNK_HDR + fit as u64 * bs;
                    if fit == blocks {
                        break;
                    }
                    seg = Segment::Raw { block: block + fit, blocks: blocks - fit, offset: offset + fit as u64 * bs };
                    close(&mut cur, &mut pieces);
                    used = base;
                }
            }
        }
    }
    close(&mut cur, &mut pieces);
    if pieces.is_empty() {
        // an all-DONT_CARE image still sends one (empty) sparse image
        pieces.push(Piece { bytes: sparse_bytes(img, &[]), verbatim: false, segments: vec![] });
    }
    Ok(pieces)
}

/// Produces a piece's bytes in order, `emit` getting at most ~1 MiB at a time.
pub fn write_piece(img: &Image, piece: &Piece, src: &mut dyn Source, emit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
    const STEP: u64 = 1 << 20;
    if piece.verbatim {
        let mut off = 0;
        while off < img.source_bytes {
            let mut buf = vec![0u8; (img.source_bytes - off).min(STEP) as usize];
            src.read_at(off, &mut buf)?;
            emit(&buf)?;
            off += buf.len() as u64;
        }
        return Ok(());
    }
    let bs = img.block_size as u64;
    let chunk = |ty: u16, blocks: u32, total: u64| {
        let mut h = Vec::with_capacity(12);
        h.extend_from_slice(&ty.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&blocks.to_le_bytes());
        h.extend_from_slice(&(total as u32).to_le_bytes());
        h
    };
    let mut hdr = Vec::with_capacity(28);
    hdr.extend_from_slice(&MAGIC.to_le_bytes());
    for v in [1u16, 0, FILE_HDR as u16, CHUNK_HDR as u16] {
        hdr.extend_from_slice(&v.to_le_bytes());
    }
    for v in [img.block_size, img.total_blocks, count_chunks(img, &piece.segments) as u32, 0] {
        hdr.extend_from_slice(&v.to_le_bytes());
    }
    emit(&hdr)?;
    let mut at = 0u32;
    for s in &piece.segments {
        if s.block() > at {
            emit(&chunk(SKIP, s.block() - at, CHUNK_HDR))?;
        }
        match *s {
            Segment::Fill { blocks, value, .. } => {
                let mut c = chunk(FILL, blocks, CHUNK_HDR + 4);
                c.extend_from_slice(&value.to_le_bytes());
                emit(&c)?;
            }
            Segment::Raw { blocks, offset, .. } => {
                let len = blocks as u64 * bs;
                emit(&chunk(RAW, blocks, CHUNK_HDR + len))?;
                let mut done = 0;
                while done < len {
                    let n = (len - done).min(STEP);
                    let mut buf = vec![0u8; n as usize];
                    // a raw source's last block may be short: the rest is zero padding
                    let avail = img.source_bytes.saturating_sub(offset + done).min(n);
                    src.read_at(offset + done, &mut buf[..avail as usize])?;
                    emit(&buf)?;
                    done += n;
                }
            }
        }
        at = s.block() + s.blocks();
    }
    if at < img.total_blocks {
        emit(&chunk(SKIP, img.total_blocks - at, CHUNK_HDR))?;
    }
    Ok(())
}

/// Builds a sparse image from block runs (tests and the delta flash): `None` = DONT_CARE.
pub fn build(block_size: u32, blocks: &[Option<Vec<u8>>]) -> Vec<u8> {
    let bs = block_size as usize;
    let mut chunks: Vec<(u16, u32, Vec<u8>)> = Vec::new();
    for b in blocks {
        let (ty, data) = match b {
            None => (SKIP, vec![]),
            Some(d) if d.chunks(4).all(|w| w == &d[..4]) => (FILL, d[..4].to_vec()),
            Some(d) => (RAW, d.clone()),
        };
        assert!(ty == SKIP || b.as_ref().unwrap().len() == bs);
        match chunks.last_mut() {
            Some(last) if last.0 == ty && (ty == SKIP || ty == RAW || last.2 == data) => {
                last.1 += 1;
                if ty == RAW {
                    last.2.extend_from_slice(&data);
                }
            }
            _ => chunks.push((ty, 1, data)),
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    for v in [1u16, 0, FILE_HDR as u16, CHUNK_HDR as u16] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [block_size, blocks.len() as u32, chunks.len() as u32, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for (ty, n, data) in chunks {
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&((CHUNK_HDR as usize + data.len()) as u32).to_le_bytes());
        out.extend_from_slice(&data);
    }
    out
}

/// What a sequence of sparse images writes onto a partition of `blocks` blocks (`None` = never written).
pub fn apply(partition: &mut [Option<Vec<u8>>], image: &[u8]) -> Result<()> {
    let mut src: &[u8] = image;
    let img = parse(&mut src, 4096)?;
    let bs = img.block_size as usize;
    for s in &img.segments {
        for k in 0..s.blocks() as usize {
            let at = s.block() as usize + k;
            partition[at] = Some(match *s {
                Segment::Raw { offset, .. } => {
                    let o = offset as usize + k * bs;
                    let mut b = image.get(o..(o + bs).min(image.len())).unwrap_or(&[]).to_vec();
                    b.resize(bs, 0);
                    b
                }
                Segment::Fill { value, .. } => value.to_le_bytes().repeat(bs / 4),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Option<Vec<u8>>> {
        (0..40u32)
            .map(|i| match i % 7 {
                0 | 1 => None,
                2 => Some(vec![0; 64]),
                3 => Some(0xdead_beefu32.to_le_bytes().repeat(16)),
                _ => Some((0..64).map(|b| (b as u32 * 3 + i) as u8).collect()),
            })
            .collect()
    }

    #[test]
    fn parse_plan_write_matches_input() {
        let blocks = sample();
        let img_bytes = build(64, &blocks);
        let mut src: &[u8] = &img_bytes;
        let img = parse(&mut src, 4096).unwrap();
        assert!(img.sparse);
        assert_eq!(img.total_blocks, 40);
        for max in [img_bytes.len() as u64, 200, 300, 1000] {
            let pieces = plan(&img, max).unwrap();
            let mut part: Vec<Option<Vec<u8>>> = vec![None; 40];
            for p in &pieces {
                assert!(p.bytes <= max, "piece {} > {max}", p.bytes);
                let mut out = Vec::new();
                let mut src: &[u8] = &img_bytes;
                write_piece(&img, p, &mut src, &mut |b| {
                    out.extend_from_slice(b);
                    Ok(())
                })
                .unwrap();
                assert_eq!(out.len() as u64, p.bytes);
                apply(&mut part, &out).unwrap();
            }
            assert_eq!(part, blocks, "max {max}");
        }
    }

    #[test]
    fn raw_image_padding_and_split() {
        let raw: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
        let mut src: &[u8] = &raw;
        let img = parse(&mut src, 4096).unwrap();
        assert!(!img.sparse);
        assert_eq!((img.total_blocks, img.tail_pad), (3, 3 * 4096 - 10_000));
        let pieces = plan(&img, 5000).unwrap();
        assert_eq!(pieces.len(), 3);
        let mut part = vec![None; 3];
        for p in &pieces {
            let mut out = Vec::new();
            let mut src: &[u8] = &raw;
            write_piece(&img, p, &mut src, &mut |b| {
                out.extend_from_slice(b);
                Ok(())
            }).unwrap();
            apply(&mut part, &out).unwrap();
        }
        let flat: Vec<u8> = part.into_iter().flat_map(|b| b.unwrap()).collect();
        assert_eq!(&flat[..10_000], &raw[..]);
        assert!(flat[10_000..].iter().all(|&b| b == 0));
    }

    #[test]
    fn rejects_bad_tables() {
        let mut img = build(64, &sample());
        img[20..24].copy_from_slice(&99u32.to_le_bytes()); // chunk count
        let mut src: &[u8] = &img;
        assert!(parse(&mut src, 4096).is_err());
        let mut img = build(64, &sample());
        img[16..20].copy_from_slice(&41u32.to_le_bytes()); // total blocks
        let mut src: &[u8] = &img;
        assert!(parse(&mut src, 4096).is_err());
    }
}
