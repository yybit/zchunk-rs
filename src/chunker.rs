use std::io::Read;

use crate::errors::ZchunkError;

const CHUNKER_WINDOW_SIZE: usize = 48;
const CHUNKER_BUZHASH_BITMASK: u32 = 2u32.pow(15) - 1;
const CHUNKER_SIZE_MIN_DEFAULT: usize = (CHUNKER_BUZHASH_BITMASK as usize + 1) / 4;
const CHUNKER_SIZE_MAX_DEFAULT: usize = (CHUNKER_BUZHASH_BITMASK as usize + 1) * 4;

const HASH_TABLE: &[u32] = &[
    0x458be752, 0xc10748cc, 0xfbbcdbb8, 0x6ded5b68, 0xb10a82b5, 0x20d75648, 0xdfc5665f, 0xa8428801,
    0x7ebf5191, 0x841135c7, 0x65cc53b3, 0x280a597c, 0x16f60255, 0xc78cbc3e, 0x294415f5, 0xb938d494,
    0xec85c4e6, 0xb7d33edc, 0xe549b544, 0xfdeda5aa, 0x882bf287, 0x3116737c, 0x05569956, 0xe8cc1f68,
    0x0806ac5e, 0x22a14443, 0x15297e10, 0x50d090e7, 0x4ba60f6f, 0xefd9f1a7, 0x5c5c885c, 0x82482f93,
    0x9bfd7c64, 0x0b3e7276, 0xf2688e77, 0x8fad8abc, 0xb0509568, 0xf1ada29f, 0xa53efdfe, 0xcb2b1d00,
    0xf2a9e986, 0x6463432b, 0x95094051, 0x5a223ad2, 0x9be8401b, 0x61e579cb, 0x1a556a14, 0x5840fdc2,
    0x9261ddf6, 0xcde002bb, 0x52432bb0, 0xbf17373e, 0x7b7c222f, 0x2955ed16, 0x9f10ca59, 0xe840c4c9,
    0xccabd806, 0x14543f34, 0x1462417a, 0x0d4a1f9c, 0x087ed925, 0xd7f8f24c, 0x7338c425, 0xcf86c8f5,
    0xb19165cd, 0x9891c393, 0x325384ac, 0x0308459d, 0x86141d7e, 0xc922116a, 0xe2ffa6b6, 0x53f52aed,
    0x2cd86197, 0xf5b9f498, 0xbf319c8f, 0xe0411fae, 0x977eb18c, 0xd8770976, 0x9833466a, 0xc674df7f,
    0x8c297d45, 0x8ca48d26, 0xc49ed8e2, 0x7344f874, 0x556f79c7, 0x6b25eaed, 0xa03e2b42, 0xf68f66a4,
    0x8e8b09a2, 0xf2e0e62a, 0x0d3a9806, 0x9729e493, 0x8c72b0fc, 0x160b94f6, 0x450e4d3d, 0x7a320e85,
    0xbef8f0e1, 0x21d73653, 0x4e3d977a, 0x1e7b3929, 0x1cc6c719, 0xbe478d53, 0x8d752809, 0xe6d8c2c6,
    0x275f0892, 0xc8acc273, 0x4cc21580, 0xecc4a617, 0xf5f7be70, 0xe795248a, 0x375a2fe9, 0x425570b6,
    0x8898dcf8, 0xdc2d97c4, 0x0106114b, 0x364dc22f, 0x1e0cad1f, 0xbe63803c, 0x5f69fac2, 0x4d5afa6f,
    0x1bc0dfb5, 0xfb273589, 0x0ea47f7b, 0x3c1c2b50, 0x21b2a932, 0x6b1223fd, 0x2fe706a8, 0xf9bd6ce2,
    0xa268e64e, 0xe987f486, 0x3eacf563, 0x1ca2018c, 0x65e18228, 0x2207360a, 0x57cf1715, 0x34c37d2b,
    0x1f8f3cde, 0x93b657cf, 0x31a019fd, 0xe69eb729, 0x8bca7b9b, 0x4c9d5bed, 0x277ebeaf, 0xe0d8f8ae,
    0xd150821c, 0x31381871, 0xafc3f1b0, 0x927db328, 0xe95effac, 0x305a47bd, 0x426ba35b, 0x1233af3f,
    0x686a5b83, 0x50e072e5, 0xd9d3bb2a, 0x8befc475, 0x487f0de6, 0xc88dff89, 0xbd664d5e, 0x971b5d18,
    0x63b14847, 0xd7d3c1ce, 0x7f583cf3, 0x72cbcb09, 0xc0d0a81c, 0x7fa3429b, 0xe9158a1b, 0x225ea19a,
    0xd8ca9ea3, 0xc763b282, 0xbb0c6341, 0x020b8293, 0xd4cd299d, 0x58cfa7f8, 0x91b4ee53, 0x37e4d140,
    0x95ec764c, 0x30f76b06, 0x5ee68d24, 0x679c8661, 0xa41979c2, 0xf2b61284, 0x4fac1475, 0x0adb49f9,
    0x19727a23, 0x15a7e374, 0xc43a18d5, 0x3fb1aa73, 0x342fc615, 0x924c0793, 0xbee2d7f0, 0x8a279de9,
    0x4aa2d70c, 0xe24dd37f, 0xbe862c0b, 0x177c22c2, 0x5388e5ee, 0xcd8a7510, 0xf901b4fd, 0xdbc13dbc,
    0x6c0bae5b, 0x64efe8c7, 0x48b02079, 0x80331a49, 0xca3d8ae6, 0xf3546190, 0xfed7108b, 0xc49b941b,
    0x32baf4a9, 0xeb833a4a, 0x88a3f1a5, 0x3a91ce0a, 0x3cc27da1, 0x7112e684, 0x4a3096b1, 0x3794574c,
    0xa3c8b6f3, 0x1d213941, 0x6e0a2e00, 0x233479f1, 0x0f4cd82f, 0x6093edd2, 0x5d7d209e, 0x464fe319,
    0xd4dcac9e, 0x0db845cb, 0xfb5e4bc3, 0xe0256ce1, 0x09fb4ed1, 0x0914be1e, 0xa5bdb2c3, 0xc6eb57bb,
    0x30320350, 0x3f397e91, 0xa67791bc, 0x86bc0e2c, 0xefa0a7e2, 0xe9ff7543, 0xe733612c, 0xd185897b,
    0x329e5388, 0x91dd236b, 0x2ecb0d93, 0xf4d82a3d, 0x35b5c03f, 0xe4e606f0, 0x05b21843, 0x37b45964,
    0x5eff22f4, 0x6027f4cc, 0x77178b3c, 0xae507131, 0x7bf7cabc, 0xf9c18d66, 0x593ade65, 0xd95ddf11,
];

pub struct Chunker<R> {
    min: usize,
    max: usize,
    bitmask: u32,

    reader: R,
    buf: Vec<u8>,
    reach_eof: bool,
}

impl<R: Read> Chunker<R> {
    pub fn default(reader: R) -> Self {
        Self::new(
            CHUNKER_SIZE_MIN_DEFAULT,
            CHUNKER_SIZE_MAX_DEFAULT,
            CHUNKER_BUZHASH_BITMASK,
            reader,
        )
    }

    pub fn new(min: usize, max: usize, bitmask: u32, reader: R) -> Self {
        Self {
            min,
            max,
            reader,
            buf: Vec::new(),
            bitmask,
            reach_eof: false,
        }
    }

    fn fill_buffer(&mut self) -> Result<(), std::io::Error> {
        if self.buf.len() < self.max {
            let mut buf = vec![0; self.max - self.buf.len()];
            let n = self.reader.read(&mut buf)?;

            self.buf.extend_from_slice(&buf[..n]);
        }

        Ok(())
    }
}

impl<R: Read> Iterator for Chunker<R> {
    type Item = Result<Vec<u8>, ZchunkError>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.reach_eof {
            if let Err(e) = self.fill_buffer() {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    self.reach_eof = true;
                } else {
                    return Some(Err(e.into()));
                }
            }
        }

        let buf_len = self.buf.len();

        // buf is empty, so no more data
        if buf_len == 0 {
            return None;
        }

        // when buf size less than minimum size, return all buffer data instead of computing hash
        if buf_len < self.min {
            return Some(Ok(self.buf.drain(..).collect()));
        }

        // determine first window position
        let (first_window_start, first_window_end) = if self.min > CHUNKER_WINDOW_SIZE {
            (self.min - CHUNKER_WINDOW_SIZE, self.min)
        } else {
            (0, CHUNKER_WINDOW_SIZE)
        };
        let mut window = self.buf[first_window_start..first_window_end]
            .to_vec()
            .clone();

        let mut checksum: u32 = 0;
        // compute hash for all bytes in window
        window.iter().enumerate().for_each(|(i, b)| {
            checksum ^= HASH_TABLE[*b as usize].rotate_left((CHUNKER_WINDOW_SIZE - i - 1) as u32)
        });

        let mut idx: usize = 0;

        // shift the window to the buffer end
        for (i, &b) in self.buf[self.min..].iter().enumerate() {
            let out = window[idx];
            window[idx] = b;
            idx = (idx + 1) % CHUNKER_WINDOW_SIZE;
            checksum = checksum.rotate_left(1)
                ^ HASH_TABLE[out as usize].rotate_left(CHUNKER_WINDOW_SIZE as u32)
                ^ HASH_TABLE[b as usize];

            if checksum & self.bitmask == 0 {
                return Some(Ok(self.buf.drain(..self.min + i).collect()));
            }
        }

        Some(Ok(self.buf.drain(..).collect()))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::BufReader};

    use sha2::{Digest, Sha512_256};

    use super::Chunker;

    struct Chunk {
        size: usize,
        id: String,
    }

    impl Chunk {
        fn new(size: usize, id: &str) -> Self {
            Self {
                size,
                id: id.to_owned(),
            }
        }
    }

    #[test]
    fn test_chunker() {
        let file = File::open("testdata/chunker.input").unwrap();
        let file_size = file.metadata().unwrap().len();
        let reader = BufReader::new(file);

        let chunks = [
            Chunk::new(
                9841,
                "b4283ac8e10a8046db348a62f502b8dbd515ac40edc9efd0eec32072d16ac486",
            ),
            Chunk::new(
                85818,
                "e29798f5bf99bef91241d5addfdd90771a921418ac5b72335e57f97aa0129278",
            ),
            Chunk::new(
                76545,
                "7d15ab556cf4eed49515d0364ae79d9213f612e7e1d4957471e1e601ab222458",
            ),
            Chunk::new(
                16088,
                "daf8e1308b31b998d33bed5dda05998b570b29f881b2cc4822998e550b38b74b",
            ),
            Chunk::new(
                52085,
                "e0b5c0fd453a77e6eaa8b21bb94a0dc62c3504bff401ac66dd53fa1e80fa4583",
            ),
            Chunk::new(
                20298,
                "355002c35a4ce411e92ac588c7a7843745d38a2ec9ed585503185efcc8334bdc",
            ),
            Chunk::new(
                37021,
                "250b1a21c606c7905c44fd6e5ab1f5ddf9424a6750e36828db6051589a24f2a4",
            ),
            Chunk::new(
                42182,
                "20550940a46414803164dbb27ae20dd241966d88f7738cf2ef68f3819bf14ff3",
            ),
            Chunk::new(
                47509,
                "956b04f3ecc4bf2b4170aec11a725d9b8ca1ed4eec53f6e779d7a89bee33b10d",
            ),
            Chunk::new(
                22996,
                "6dabf607c0379164a6ac8d89b9c4948058145f41b3eb460547224af9550edd62",
            ),
            Chunk::new(
                56379,
                "3119f66e9663a998cdd2a1ce14b1f0ba3a9a6601b2713192b6d9b4ce918460dd",
            ),
            Chunk::new(
                12589,
                "0eb36e21db99b710945bf5b8069d83611071d2265939d4f67e38277d2ab7b7b7",
            ),
            Chunk::new(
                62811,
                "a9c3b5bc32b97fb5cf9f6a655b075ea72d99997a5a72f7de8e8be3e9d01b2c29",
            ),
            Chunk::new(
                13456,
                "045e761b2ac3fd1402cdd3148496bf0a1bf482128cb65bd56bc32220d7fc5fdb",
            ),
            Chunk::new(
                74529,
                "11d1771aee585728f1250e81a1a0dadf61f0e1386254aff06d6c35e3fe8e2a66",
            ),
            Chunk::new(
                26428,
                "75c030c5087bcb54fddaab772c393f65352b554141bc98d6105786f8ba035b6e",
            ),
            Chunk::new(
                56138,
                "dc03c9976c5f0ee57845df35e7c6933e9bc2dfb1cc33313358fd03309bec8f3a",
            ),
            Chunk::new(
                28179,
                "cfd2c6aef4bbe4db661c214fd033dfbb6c86dc994ee5f96249a405f5510cf263",
            ),
            Chunk::new(
                24548,
                "ad9562227fac34440ef025988b7696943851e3b9d6c85562a55317fdc352ca17",
            ),
            Chunk::new(
                88299,
                "150ceeaf222bb7c4d30d7eee46f7eda8bf1a1c3445ceac88ee34b4da379fe670",
            ),
            Chunk::new(
                45982,
                "97e3f448230bee3c7bbb4763699ae7b7df93261a7478e872815f427af8ec922b",
            ),
            Chunk::new(
                11699,
                "5267c6556d7c6c72ebbb2aab6f313e0e695cc193b7bbdd24ab190f6da90cb87e",
            ),
            Chunk::new(
                18242,
                "90c4196b3b13d7031eda3fef25d38745214a6b1916326f712d28015ae93b9848",
            ),
            Chunk::new(
                44344,
                "ab488f7244d0f191945598af00c96e49004c85b6bb0df32621ea4d147546ef61",
            ),
            Chunk::new(
                54938,
                "1dbe844fe16331921a0c58982808a52b2963b51e2cbfa89b65726bbef53c058a",
            ),
            Chunk::new(
                10756,
                "0ccd0a714a2e2986c6b0702d1fdda1f705618e9fe34d057f4e98f3d8dfe44210",
            ),
            Chunk::new(
                8876,
                "9e927393baadb1eea7cdaa41623614f6a62d36f48c2d60de5a452896d92d64f2",
            ),
        ];

        let chunker = Chunker::default(reader);
        let mut total_size = 0;
        for (i, c) in chunker.into_iter().enumerate() {
            let chunk = c.unwrap();

            let mut hasher = Sha512_256::new();
            hasher.update(&chunk);
            let result = hasher.finalize();
            let expect_chunk = chunks.get(i).unwrap();

            assert_eq!(hex::encode(result), expect_chunk.id);
            assert_eq!(chunk.len(), expect_chunk.size);

            total_size += chunk.len();
        }

        assert_eq!(file_size, total_size as u64);
    }
}
