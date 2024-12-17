use bytes::{Buf, BufMut};

pub fn read_uint<B: Buf>(buf: &mut B) -> u64 {
    let mut n = 0;
    let mut shift = 0;
    loop {
        let byte = buf.get_u8();
        if byte & 128 == 0 {
            n |= u64::from(byte) << shift;
            return n;
        } else {
            n |= u64::from(byte & 127) << shift;
        }
        shift += 7;
    }
}

pub fn write_uint<B: BufMut>(buf: &mut B, mut n: u64) {
    while n > 127 {
        buf.put_u8((n as u8 & 127) | 128);
        n >>= 7;
    }
    buf.put_u8(n as u8);
}

#[cfg(test)]
mod tests {
    use quickcheck::quickcheck;

    use super::*;

    quickcheck! {
        fn test_uint_roundtrip(n: u64) -> bool {
            let mut buf = Vec::new();
            write_uint(&mut buf, n);

            let mut reader = &buf[..];
            read_uint(&mut reader) == n
        }
    }
}
