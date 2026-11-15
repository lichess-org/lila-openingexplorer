use bytes::BufMut;

use crate::packed::{pack_uint, unpack_uint};

pub fn pack_bytes<B: BufMut>(buf: &mut B, bytes: &[u8]) {
    pack_uint(buf, bytes.len() as u64);
    buf.put_slice(bytes);
}

pub fn unpack_bytes<'a>(buf: &mut &'a [u8]) -> &'a [u8] {
    let len = unpack_uint(buf) as usize;
    let (bytes, rest) = buf.split_at(len);
    *buf = rest;
    bytes
}

pub fn pack_string<B: BufMut>(buf: &mut B, s: &str) {
    pack_bytes(buf, s.as_bytes());
}

pub fn unpack_string<'a>(buf: &mut &'a [u8]) -> &'a str {
    str::from_utf8(unpack_bytes(buf)).expect("valid utf-8")
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_pack_bytes_roundtrip() {
        arbtest(|u| {
            let bytes: Vec<u8> = u.arbitrary()?;

            let mut buf = Vec::new();
            pack_bytes(&mut buf, &bytes);
            assert_eq!(unpack_bytes(&mut &buf[..]), bytes);
            Ok(())
        });
    }

    #[test]
    fn test_pack_string_roundtrip() {
        arbtest(|u| {
            let s: String = u.arbitrary()?;

            let mut buf = Vec::new();
            pack_string(&mut buf, &s);
            assert_eq!(unpack_string(&mut &buf[..]), s);
            Ok(())
        });
    }
}
