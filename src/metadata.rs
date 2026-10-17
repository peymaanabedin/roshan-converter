//! Bounded, in-place EXIF fixes. Offsets and unknown camera/MakerNote data stay intact.
use std::collections::HashSet;

#[derive(Clone, Copy)]
enum Endian { Little, Big }
impl Endian {
    fn u16(self, bytes: &[u8], at: usize) -> Option<u16> {
        let a: [u8; 2] = bytes.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(match self { Self::Little=>u16::from_le_bytes(a), Self::Big=>u16::from_be_bytes(a) })
    }
    fn u32(self, bytes: &[u8], at: usize) -> Option<u32> {
        let a: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(match self { Self::Little=>u32::from_le_bytes(a), Self::Big=>u32::from_be_bytes(a) })
    }
    fn put16(self, bytes: &mut [u8], at: usize, value: u16) {
        let v = match self { Self::Little=>value.to_le_bytes(), Self::Big=>value.to_be_bytes() };
        bytes[at..at+2].copy_from_slice(&v);
    }
    fn put32(self, bytes: &mut [u8], at: usize, value: u32) {
        let v = match self { Self::Little=>value.to_le_bytes(), Self::Big=>value.to_be_bytes() };
        bytes[at..at+4].copy_from_slice(&v);
    }
}

fn tiff(raw: &[u8]) -> Option<(&[u8], Endian)> {
    if raw.len() < 8 { return None; }
    let b = if raw.starts_with(b"II\x2a\0") || raw.starts_with(b"MM\0\x2a") {
        raw
    } else if raw.starts_with(b"Exif\0\0") {
        raw.get(6..)?
    } else {
        let offset = u32::from_be_bytes(raw.get(..4)?.try_into().ok()?) as usize;
        raw.get(4usize.checked_add(offset)?..)?
    };
    if b.len() < 8 { return None; }
    let endian = match &b[..4] {
        b"II\x2a\0" => Endian::Little,
        b"MM\0\x2a" => Endian::Big,
        _=>return None,
    };
    Some((b, endian))
}

fn entry_bounds(b: &[u8], endian: Endian, offset: usize) -> Option<(usize,usize)> {
    if offset < 8 { return None; }
    let count = endian.u16(b, offset)? as usize;
    let start = offset.checked_add(2)?;
    let end = start.checked_add(count.checked_mul(12)?)?;
    if end.checked_add(4)? > b.len() { return None; }
    Some((start,end))
}

pub fn orientation(raw: &[u8]) -> Option<u16> {
    let (b, endian) = tiff(raw)?;
    let offset = endian.u32(b,4)? as usize;
    let (start,end) = entry_bounds(b,endian,offset)?;
    for p in (start..end).step_by(12) {
        if endian.u16(b,p)? == 0x112 && endian.u16(b,p+2)? == 3 && endian.u32(b,p+4)? == 1 {
            return endian.u16(b,p+8).filter(|v| (1..=8).contains(v));
        }
    }
    None
}

/// Returns TIFF bytes without Exif\0\0; jpeg-encoder adds that prefix itself.
pub fn normalize(raw: &[u8], width: u32, height: u32) -> Result<Vec<u8>, &'static str> {
    normalize_with_limit(raw,width,height,65_527)
}

pub fn normalize_with_limit(raw:&[u8],width:u32,height:u32,max_bytes:usize)->Result<Vec<u8>,&'static str> {
    let (bytes,endian) = tiff(raw).ok_or("Invalid EXIF metadata was omitted")?;
    if bytes.len() > max_bytes { return Err("EXIF metadata exceeds the output limit and was omitted"); }
    let mut b = bytes.to_vec();
    let root = endian.u32(&b,4).ok_or("Incomplete EXIF metadata was omitted")? as usize;
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    while let Some(offset) = pending.pop() {
        if !visited.insert(offset) { return Err("Malformed EXIF metadata was omitted"); }
        if visited.len() > 16 { return Err("EXIF metadata is too complex and was omitted"); }
        let (start,end) = entry_bounds(&b,endian,offset).ok_or("Incomplete EXIF metadata was omitted")?;
        for p in (start..end).step_by(12) {
            let tag = endian.u16(&b,p).unwrap();
            let kind = endian.u16(&b,p+2).unwrap();
            let count = endian.u32(&b,p+4).unwrap();
            if count != 1 { continue; }
            if tag == 0x8769 && kind == 4 {
                let exif_ifd = endian.u32(&b,p+8).unwrap() as usize;
                if exif_ifd != 0 { pending.push(exif_ifd); }
            }
            let value = match tag {
                0x0112 => Some(1),
                0x0100 | 0xa002 => Some(width),
                0x0101 | 0xa003 => Some(height),
                _=>None,
            };
            if let Some(value) = value {
                match kind {
                    3 if value <= u16::MAX as u32=>endian.put16(&mut b,p+8,value as u16),
                    4=>endian.put32(&mut b,p+8,value),
                    _=>{},
                }
            }
        }
        // The original thumbnail no longer describes the normalized output.
        // Detach the next-IFD chain, retaining bytes so MakerNote offsets remain valid.
        endian.put32(&mut b,end,0);
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(little: bool) -> Vec<u8> {
        let endian = if little { Endian::Little } else { Endian::Big };
        let mut b=vec![0u8;8+2+4*12+4];
        b[..4].copy_from_slice(if little { b"II\x2a\0" } else { b"MM\0\x2a" });
        endian.put32(&mut b,4,8);
        endian.put16(&mut b,8,4);
        for (i,(tag,kind,value)) in [(0x112u16,3u16,6u32),(0x100,4,10),(0x101,4,20),(0xc001,4,98765)].into_iter().enumerate() {
            let p=10+i*12;
            endian.put16(&mut b,p,tag); endian.put16(&mut b,p+2,kind); endian.put32(&mut b,p+4,1);
            if kind==3 { endian.put16(&mut b,p+8,value as u16); } else { endian.put32(&mut b,p+8,value); }
        }
        endian.put32(&mut b,58,400);
        let mut raw=vec![0,0,0,0]; raw.extend(b); raw
    }
    #[test]
    fn exif_both_endian_normalizes_geometry_preserves_unknown() {
        for little in [true,false] {
            let raw=sample(little); assert_eq!(orientation(&raw),Some(6));
            let b=normalize(&raw,20,10).unwrap();
            let e=if little { Endian::Little } else { Endian::Big };
            assert_eq!(e.u16(&b,18),Some(1));
            assert_eq!(e.u32(&b,30),Some(20));
            assert_eq!(e.u32(&b,42),Some(10));
            assert_eq!(e.u32(&b,54),Some(98765));
            assert_eq!(e.u32(&b,58),Some(0));
        }
    }
    #[test]
    fn exif_accepts_nonzero_heif_offset_and_rejects_bad_bounds() {
        let raw=sample(true); let mut shifted=vec![0,0,0,6]; shifted.extend(b"Exif\0\0"); shifted.extend(&raw[4..]);
        assert_eq!(orientation(&shifted),Some(6));
        assert!(normalize(&shifted,2,3).is_ok());
        let mut malformed=raw.clone(); malformed[..4].fill(255);
        assert!(normalize(&malformed,2,3).is_err());
        for n in 0..raw.len() { assert!(normalize(&raw[..n],2,3).is_err()); }
    }
    #[test]
    fn non_jpeg_outputs_can_retain_large_exif() {
        let mut raw=sample(true);raw.resize(80_000,0);
        assert!(normalize(&raw,10,20).is_err());
        assert_eq!(normalize_with_limit(&raw,10,20,16*1024*1024).unwrap().len(),raw.len()-4);
        assert_eq!(orientation(&raw[4..]),Some(6));
        let mut prefixed=b"Exif\0\0".to_vec();prefixed.extend_from_slice(&raw[4..]);
        assert_eq!(orientation(&prefixed),Some(6));
    }
}
