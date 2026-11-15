use bytes::{Buf, BufMut};

pub fn pack_uint<B: BufMut>(buf: &mut B, mut n: u64) {
    while n > 0x7f {
        buf.put_u8((n as u8) & 0x7f | 0x80);
        n >>= 7;
    }
    buf.put_u8(n as u8);
}

pub fn unpack_uint<B: Buf>(buf: &mut B) -> u64 {
    let mut n = 0;
    let mut shift = 0;
    loop {
        let byte = buf.get_u8();
        n |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break n;
        }
        shift += 7;
    }
}

pub fn pack_int<B: BufMut>(buf: &mut B, n: i64) {
    // Zigzag (small absolute values to small unsigned values)
    pack_uint(buf, ((n << 1) ^ (n >> 63)) as u64);
}

pub fn unpack_int<B: Buf>(buf: &mut B) -> i64 {
    let n = unpack_uint(buf);
    (n >> 1) as i64 ^ -(n as i64 & 1)
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_pack_uint_roundtrip() {
        arbtest(|u| {
            let n: u64 = u.arbitrary()?;

            let mut buf = Vec::new();
            pack_uint(&mut buf, n);
            assert_eq!(unpack_uint(&mut &buf[..]), n);
            Ok(())
        });
    }

    #[test]
    fn test_pack_int_roundtrip() {
        arbtest(|u| {
            let n: i64 = u.arbitrary()?;

            let mut buf = Vec::new();
            pack_int(&mut buf, n);
            assert_eq!(unpack_int(&mut &buf[..]), n);
            Ok(())
        });
    }
}
