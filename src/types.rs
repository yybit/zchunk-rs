use std::io::{self, Error, ErrorKind, Write};

/// Extends [`Read`] with methods for reading variant int. (For `std::io`.)
pub trait ReadVariantInt: io::Read {
    #[inline]
    fn read_variant_int(&mut self) -> Result<VariantInt, std::io::Error> {
        let mut bs = Vec::new();

        loop {
            let mut buf = [0; 1];
            self.read_exact(&mut buf)?;

            bs.push(buf[0]);

            if buf[0] & 0x80 != 0 {
                break;
            }
        }

        Ok(VariantInt(bs))
    }
}

impl<R: io::Read + ?Sized> ReadVariantInt for R {}

/// Extends [`Write`] with methods for writing variant int. (For `std::io`.)
pub trait WriteVariantInt: io::Write {
    #[inline]
    fn write_variant_int(&mut self, i: VariantInt) -> Result<(), std::io::Error> {
        let length = i.0.len();

        for (index, &b) in i.0.iter().enumerate() {
            let b_with_flag = if index == length - 1 {
                b | 0x80
            } else {
                b & 0x7f
            };
            self.write_all(&[b_with_flag])?;
        }

        Ok(())
    }
}

impl<W: io::Write + ?Sized> WriteVariantInt for W {}

/// VariantInt use LittleEndian.
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct VariantInt(Vec<u8>);

impl From<u64> for VariantInt {
    fn from(value: u64) -> Self {
        let mut num = value;
        let mut bytes = Vec::new();
        while num >= 0x80 {
            bytes.push((num as u8) & 0x7f);
            num >>= 7;
        }
        bytes.push((num as u8) | 0x80);
        Self(bytes)
    }
}

impl VariantInt {
    pub fn byte_size(&self) -> usize {
        self.0.len()
    }

    pub fn from_bytes(b: Vec<u8>) -> Self {
        Self(b)
    }

    pub fn to_u64(&self) -> Result<u64, std::io::Error> {
        if self.0.len() > 10 {
            return Err(Error::new(
                ErrorKind::Other,
                "VariantInt has greater than 10 bytes",
            ));
        }

        let mut num = 0u64;
        for (i, &byte) in self.0.iter().enumerate() {
            let last_seven_bits = byte & 0x7f;
            num |= (last_seven_bits as u64) << (7 * i);
            if byte & 0x80 != 0 {
                return Ok(num);
            }
        }
        Ok(num)
    }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), std::io::Error> {
        writer.write_all(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use crate::VariantInt;

    fn test_variant_int_inner(n: u64, expect_bytes_size: usize, expect_bytes: &[u8]) {
        let mut buf = Vec::new();
        let vint = VariantInt::from(n);
        assert_eq!(vint.byte_size(), expect_bytes_size);
        assert_eq!(vint.to_u64().unwrap(), n);
        vint.write_to(&mut buf).unwrap();
        assert_eq!(buf, expect_bytes);
    }

    #[test]
    fn test_variant_int() {
        test_variant_int_inner(0, 1, &[0x80]);
        test_variant_int_inner(5, 1, &[0x85]);
        test_variant_int_inner(255, 2, &[0x7f, 0x81]);

        let vint = VariantInt::from_bytes(vec![0; 1]);
        assert_eq!(vint.to_u64().unwrap(), 0);
    }
}
