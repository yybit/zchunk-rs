use std::{
    collections::HashMap,
    io::{self, BufRead, Cursor, Read, Seek, SeekFrom, Write},
};

use sha2::{Digest, Sha256, Sha512};

use crate::{
    chunker::Chunker,
    errors::ZchunkError,
    types::{ReadVariantInt, VariantInt},
};

const ZCHUNK_VERSION_1: &[u8] = b"\0ZCK1";
const ZCHUNK_DETACHED_VERSION_1: &[u8] = b"\0ZHR1";

const CHECKSUM_SHA1: u8 = 0;
const CHECKSUM_SHA256: u8 = 1;
const CHECKSUM_SHA512: u8 = 2;
const CHECKSUM_SHA512_128: u8 = 3; //first 128 bits of SHA-512 checksum

const COMPRESSION_NONE: u8 = 0;
const COMPRESSION_ZSTD: u8 = 2;

#[derive(Debug)]
pub struct Lead {
    id: [u8; 5],
    checksum_type: VariantInt,
    header_size: VariantInt,
    header_checksum: [u8; 32],
}

impl Lead {
    pub fn new(header_size: usize) -> Result<Self, ZchunkError> {
        Ok(Self {
            id: ZCHUNK_VERSION_1.try_into()?,
            checksum_type: (CHECKSUM_SHA256 as u64).into(),
            header_size: (header_size as u64).into(),
            header_checksum: [0; 32],
        })
    }

    pub fn write_to(
        &self,
        mut writer: impl Write,
        ignore_checksum: bool,
    ) -> Result<(), std::io::Error> {
        writer.write_all(&self.id)?;
        self.checksum_type.write_to(&mut writer)?;
        self.header_size.write_to(&mut writer)?;
        if !ignore_checksum {
            writer.write_all(&self.header_checksum)?;
        }

        Ok(())
    }

    pub fn set_header_checksum(&mut self, header_checksum: [u8; 32]) {
        self.header_checksum = header_checksum;
    }

    pub fn byte_size(&self) -> usize {
        self.id.len()
            + self.checksum_type.byte_size()
            + self.header_size.byte_size()
            + self.header_checksum.len()
    }

    pub fn from_reader(mut reader: impl Read) -> Result<Self, ZchunkError> {
        let mut id = [0; 5];
        reader.read_exact(&mut id)?;

        if id != ZCHUNK_VERSION_1 && id != ZCHUNK_DETACHED_VERSION_1 {
            return Err(ZchunkError::InvalidLeaderID(id.clone()));
        }

        let checksum_type = reader.read_variant_int()?;
        match checksum_type.to_u64()? as u8 {
            CHECKSUM_SHA1 | CHECKSUM_SHA256 => {}
            t => return Err(ZchunkError::InvalidChecksumType(t)),
        }

        let header_size = reader.read_variant_int()?;

        let mut header_checksum = [0; 32];
        reader.read_exact(&mut header_checksum)?;

        Ok(Lead {
            id,
            checksum_type,
            header_size,
            header_checksum,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PrefaceFlags {
    vint: VariantInt,
    uint: u64,
}

impl PrefaceFlags {
    pub fn from_variant_int(n: VariantInt) -> Result<Self, ZchunkError> {
        let uint = n.to_u64()?;
        Ok(Self { vint: n, uint })
    }

    pub fn from_u64(n: u64) -> Self {
        Self {
            vint: VariantInt::from(n),
            uint: n,
        }
    }

    pub fn write_to(&self, writer: impl Write) -> Result<(), std::io::Error> {
        self.vint.write_to(writer)
    }

    pub fn byte_size(&self) -> usize {
        self.vint.byte_size()
    }

    fn has_stream(&self) -> bool {
        self.uint & 0x01 != 0
    }

    fn has_optional(&self) -> bool {
        self.uint & 0x02 != 0
    }

    // fn has_uncompressed(&self) -> bool {
    //     self.uint & 0x04 != 0
    // }
}

#[derive(Debug)]
pub struct Preface {
    data_checksum: [u8; 32],
    flags: PrefaceFlags,
    compression_type: VariantInt,
    optional_element_count: Option<VariantInt>,
}

impl Preface {
    pub fn new(data_checksum: [u8; 32]) -> Self {
        Self {
            data_checksum: data_checksum,
            flags: PrefaceFlags::from_u64(0),
            compression_type: (COMPRESSION_ZSTD as u64).into(),
            optional_element_count: None,
        }
    }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), std::io::Error> {
        writer.write_all(&self.data_checksum)?;
        self.flags.write_to(&mut writer)?;
        self.compression_type.write_to(&mut writer)?;

        if let Some(count) = &self.optional_element_count {
            count.write_to(writer)?;
        }

        Ok(())
    }

    pub fn byte_size(&self) -> usize {
        let mut n =
            self.data_checksum.len() + self.flags.byte_size() + self.compression_type.byte_size();
        if let Some(count) = &self.optional_element_count {
            n += count.byte_size();
        }
        n
    }

    pub fn from_reader(mut reader: impl Read) -> Result<Self, ZchunkError> {
        let mut data_checksum = [0; 32];
        reader.read_exact(&mut data_checksum)?;

        let flags = PrefaceFlags::from_variant_int(reader.read_variant_int()?)?;
        let compression_type = reader.read_variant_int()?;

        let compression_type_u8 = compression_type.to_u64()? as u8;
        if compression_type_u8 != COMPRESSION_NONE && compression_type_u8 != COMPRESSION_ZSTD {
            return Err(ZchunkError::InvalidCompresionType(compression_type_u8));
        }

        let optional_element_count = if flags.has_optional() {
            Some(reader.read_variant_int()?)
        } else {
            None
        };

        Ok(Preface {
            data_checksum,
            flags,
            compression_type,
            optional_element_count,
        })
    }
}

type ChunkOffset = u32;
// type ChunkIndex = usize;

#[derive(Debug)]
pub struct Index {
    size: VariantInt,
    checksum_type: VariantInt,
    chunks_count: VariantInt,
    dict_chunk: Chunk,
    data_chunks: Vec<(Chunk, ChunkOffset)>,
}

impl Index {
    pub fn new(chunks: Vec<Chunk>) -> Result<Self, ZchunkError> {
        let dict_chunk = Chunk::new([0; 16], 0, 0);

        let checksum_type = VariantInt::from(CHECKSUM_SHA512_128 as u64);
        let chunks_count = VariantInt::from(chunks.len() as u64 + 1);
        let size = checksum_type.byte_size()
            + chunks_count.byte_size()
            + dict_chunk.byte_size()
            + chunks.iter().map(|c| c.byte_size()).sum::<usize>();

        // first data chunk offset is the end of dict chunk
        let mut chunk_offset = dict_chunk.length.to_u64()? as u32;

        // compute offset for each data chunk
        let mut data_chunks = Vec::new();
        for c in chunks {
            let length = c.length.to_u64()? as u32;
            data_chunks.push((c, chunk_offset));
            chunk_offset += length;
        }

        Ok(Self {
            size: (size as u64).into(),
            checksum_type,
            chunks_count,
            dict_chunk,
            data_chunks,
        })
    }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), std::io::Error> {
        self.size.write_to(&mut writer)?;
        self.checksum_type.write_to(&mut writer)?;
        self.chunks_count.write_to(&mut writer)?;
        self.dict_chunk.write_to(&mut writer)?;
        for (chunk, _) in &self.data_chunks {
            chunk.write_to(&mut writer)?;
        }

        Ok(())
    }

    pub fn byte_size(&self) -> usize {
        self.checksum_type.byte_size()
            + self.chunks_count.byte_size()
            + self.dict_chunk.byte_size()
            + self
                .data_chunks
                .iter()
                .map(|(c, _)| c.byte_size())
                .sum::<usize>()
            + self.size.byte_size()
    }

    pub fn from_reader(mut reader: impl Read, flags: PrefaceFlags) -> Result<Self, ZchunkError> {
        let size = reader.read_variant_int()?;
        let checksum_type = reader.read_variant_int()?;

        // check checksum type
        let checksum_type_u8 = checksum_type.to_u64()? as u8;
        if ![
            CHECKSUM_SHA1,
            CHECKSUM_SHA256,
            CHECKSUM_SHA512,
            CHECKSUM_SHA512_128,
        ]
        .contains(&checksum_type_u8)
        {
            return Err(ZchunkError::InvalidChecksumType(checksum_type_u8));
        }

        let chunks_count = reader.read_variant_int()?;

        let dict_chunk = Chunk::from_reader(&mut reader, flags.clone())?;

        let mut chunk_offset = dict_chunk.length.to_u64()? as u32;
        let mut data_chunks = Vec::new();
        for _ in 0..(chunks_count.to_u64()? - 1) {
            let chunk = Chunk::from_reader(&mut reader, flags.clone())?;
            let length = chunk.length.to_u64()? as u32;
            data_chunks.push((chunk, chunk_offset));
            chunk_offset += length;
        }

        // check index size
        let expect_index_size = (checksum_type.byte_size()
            + chunks_count.byte_size()
            + dict_chunk.byte_size()
            + data_chunks
                .iter()
                .map(|(c, _)| c.byte_size())
                .sum::<usize>()) as u64;
        let index_size = size.to_u64()?;
        if expect_index_size != index_size {
            return Err(ZchunkError::InvalidIndexSize {
                expected: expect_index_size,
                found: index_size,
            });
        }

        Ok(Index {
            size,
            checksum_type,
            chunks_count,
            dict_chunk,
            data_chunks,
        })
    }
}

#[derive(Debug, Clone, Hash)]
pub struct Chunk {
    stream: Option<VariantInt>, // if flag 0 is set to 1
    checksum: [u8; 16],
    length: VariantInt,
    uncompressed_length: VariantInt,
}

impl Chunk {
    pub fn new(checksum: [u8; 16], length: u32, uncompressed_length: u32) -> Self {
        Self {
            stream: None,
            checksum,
            length: (length as u64).into(),
            uncompressed_length: (uncompressed_length as u64).into(),
        }
    }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), std::io::Error> {
        if let Some(s) = &self.stream {
            s.write_to(&mut writer)?;
        }
        writer.write_all(&self.checksum)?;
        self.length.write_to(&mut writer)?;
        self.uncompressed_length.write_to(writer)?;

        Ok(())
    }

    pub fn byte_size(&self) -> usize {
        let mut n =
            self.checksum.len() + self.length.byte_size() + self.uncompressed_length.byte_size();

        if let Some(stream) = &self.stream {
            n += stream.byte_size();
        }

        n
    }

    pub fn from_reader(mut reader: impl Read, flags: PrefaceFlags) -> Result<Self, ZchunkError> {
        let stream = if flags.has_stream() {
            Some(reader.read_variant_int()?)
        } else {
            None
        };

        let mut checksum = [0; 16];
        reader.read_exact(&mut checksum)?;

        let length = reader.read_variant_int()?;
        let uncompressed_length = reader.read_variant_int()?;

        Ok(Chunk {
            stream,
            checksum,
            length,
            uncompressed_length,
        })
    }
}

impl PartialEq for Chunk {
    fn eq(&self, other: &Self) -> bool {
        self.checksum == other.checksum
            && self.length == other.length
            && self.uncompressed_length == other.uncompressed_length
    }
}

impl Eq for Chunk {}

#[derive(Debug)]
pub struct Signatures {
    count: VariantInt,
    signatures: Vec<Signature>,
}

impl Signatures {
    pub fn new(signatures: Vec<Signature>) -> Self {
        Self {
            count: (signatures.len() as u64).into(),
            signatures,
        }
    }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), std::io::Error> {
        self.count.write_to(&mut writer)?;
        for sig in &self.signatures {
            sig.write_to(&mut writer)?;
        }

        Ok(())
    }

    pub fn byte_size(&self) -> usize {
        self.count.byte_size() + self.signatures.iter().map(|s| s.byte_size()).sum::<usize>()
    }

    pub fn from_reader(mut reader: impl Read) -> Result<Self, ZchunkError> {
        let count = reader.read_variant_int()?;

        let mut signatures = Vec::new();
        for _ in 0..(count.to_u64()?) {
            let sigature = Signature::from_reader(&mut reader)?;
            signatures.push(sigature);
        }

        Ok(Signatures { count, signatures })
    }
}

#[derive(Debug)]
pub struct Signature {
    type_: VariantInt,
    size: VariantInt,
    signature: Vec<u8>,
}

impl Signature {
    // pub fn new(size: usize, signature: Vec<u8>) -> Self {
    //     Self {
    //         type_: 0u64.into(),
    //         size: (size as u64).into(),
    //         signature,
    //     }
    // }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), std::io::Error> {
        self.type_.write_to(&mut writer)?;
        self.size.write_to(&mut writer)?;
        writer.write_all(&self.signature)?;

        Ok(())
    }

    pub fn byte_size(&self) -> usize {
        self.type_.byte_size() + self.size.byte_size() + self.signature.len()
    }

    pub fn from_reader(mut reader: impl Read) -> Result<Self, ZchunkError> {
        let type_ = reader.read_variant_int()?;
        let size = reader.read_variant_int()?;

        let mut signature = vec![0; size.to_u64()? as usize];
        reader.read_exact(&mut signature)?;

        Ok(Signature {
            type_,
            size,
            signature,
        })
    }
}

#[derive(Debug)]
struct DataChunk(Vec<u8>);

#[derive(Debug)]
pub struct Header {
    lead: Lead,
    preface: Preface,
    index: Index,
    signatures: Signatures,
}

impl Header {
    pub fn new(lead: Lead, preface: Preface, index: Index, signatures: Signatures) -> Self {
        Self {
            lead,
            preface,
            index,
            signatures,
        }
    }

    pub fn write_to(
        &mut self,
        mut writer: impl Write,
        ignore_checksum: bool,
    ) -> Result<(), std::io::Error> {
        self.lead.write_to(&mut writer, ignore_checksum)?;
        self.preface.write_to(&mut writer)?;
        self.index.write_to(&mut writer)?;
        self.signatures.write_to(&mut writer)?;

        Ok(())
    }

    /// compute header checksum, ignoring the header checksum field
    pub fn compute_and_set_checksum(&mut self) -> Result<(), ZchunkError> {
        let mut writer: Vec<u8> = Vec::with_capacity(self.lead.header_size.to_u64()? as usize);
        self.write_to(&mut writer, true)?;

        let mut hasher = Sha256::new();
        hasher.update(&writer);
        let result = hasher.finalize();

        self.lead.set_header_checksum(result[..].try_into()?);

        Ok(())
    }

    /// check if dict chunk is equal
    pub fn has_dict_chunk(&self, chunk: &Chunk) -> bool {
        self.index.dict_chunk == *chunk
    }

    /// get chunk offset by data chunk
    pub fn find_data_chunks(&self, chunks: Vec<Chunk>) -> HashMap<Chunk, ChunkOffset> {
        self.index
            .data_chunks
            .clone()
            .into_iter()
            .filter(|(c, _)| chunks.contains(&c))
            .collect()
    }
}

/// An encoder that compress input data from `Read` and write compressed data to `Write`
///
/// Require a temp `Read + Write + Seek` that store compressed chunks data, since building header is after the chunks data is generated
pub struct Encoder<RW, R> {
    header: Option<Header>,
    temp: RW,
    reader: R,
}

impl<RW: Read + Write + Seek, R: Read> Encoder<RW, R> {
    pub fn new(reader: R, temp: RW) -> Result<Self, ZchunkError> {
        Ok(Self {
            header: None,
            temp,
            reader,
        })
    }

    /// split data of reader to chunks, and use zstd to compress chunks, write to temp writer [without header]
    pub fn prepare_chunks(&mut self) -> Result<(), ZchunkError> {
        let chunker = Chunker::default(&mut self.reader);
        let mut chunks = Vec::new();
        let mut total_hasher = Sha256::new();
        for c in chunker {
            let uncompressed_chunk_data = c?;
            let compressed_chunk_data = zstd::encode_all(uncompressed_chunk_data.as_slice(), 3)?;

            // compute chunk checksum
            let mut hasher = Sha512::new();
            hasher.update(&compressed_chunk_data);
            let result = hasher.finalize();

            // compute checksum of all chunks
            total_hasher.update(&compressed_chunk_data);

            // write compressed data to temp writer
            self.temp.write_all(&compressed_chunk_data)?;

            // compose chunk metadata
            let chunk = Chunk::new(
                result[..16].try_into()?,
                compressed_chunk_data.len() as u32,
                uncompressed_chunk_data.len() as u32,
            );
            // print!("{} ", uncompressed_chunk_data.len());
            chunks.push(chunk);
        }

        let data_checksum = total_hasher.finalize();

        let signatures = Signatures::new(Vec::new());
        let index = Index::new(chunks)?;
        let preface = Preface::new(data_checksum[..].try_into()?);
        let header_size = signatures.byte_size() + index.byte_size() + preface.byte_size();
        let lead = Lead::new(header_size)?;

        let mut header = Header::new(lead, preface, index, signatures);
        header.compute_and_set_checksum()?;

        self.header = Some(header);

        Ok(())
    }

    /// write header and chunks to `Write`, which require `prepare_chunks`
    pub fn compress_to(&mut self, mut writer: impl Write) -> Result<(), ZchunkError> {
        let header = self.header.as_mut().ok_or(ZchunkError::HeaderNotFound)?;
        header.write_to(&mut writer, false)?;

        self.temp.seek(SeekFrom::Start(0))?;
        io::copy(&mut self.temp, &mut writer)?;

        Ok(())
    }
}

/// A decoder that decompress input data from `BufRead + Seek`, and write uncompressed data to `Write`
pub struct Decoder<R> {
    header: Header,
    header_size: u64,
    reader: R,
}

impl<R: BufRead + Seek> Decoder<R> {
    pub fn new(mut reader: R) -> Result<Self, ZchunkError> {
        let lead = Lead::from_reader(&mut reader)?;
        let preface = Preface::from_reader(&mut reader)?;
        let index = Index::from_reader(&mut reader, preface.flags.clone())?;
        let signatures = Signatures::from_reader(&mut reader)?;

        let expect_header_size = lead.header_size.to_u64()? + lead.byte_size() as u64;
        let header_size = reader.stream_position()?;
        if expect_header_size != header_size {
            return Err(ZchunkError::InvalidHeaderSize {
                expected: expect_header_size,
                found: header_size,
            });
        }

        let header = Header::new(lead, preface, index, signatures);

        Ok(Self {
            header,
            header_size,
            reader,
        })
    }

    /// get chunk data by offset and chunk, no decompression
    ///
    /// offset is relative to the end of header, so seeking reader need plus header size
    fn get_chunk_data(&mut self, offset: u64, chunk: &Chunk) -> Result<Vec<u8>, ZchunkError> {
        let length = chunk.length.to_u64()? as usize;
        let mut buf = vec![0; length];
        if length == 0 {
            return Ok(buf);
        }

        self.reader
            .seek(SeekFrom::Start(self.header_size + offset))?;
        self.reader.read_exact(&mut buf)?;

        let result: [u8; 16] = match self.header.index.checksum_type.to_u64()? as u8 {
            CHECKSUM_SHA256 => {
                let mut hasher = Sha256::new();
                hasher.update(&buf);
                hasher.finalize()[..16].try_into()?
            }
            CHECKSUM_SHA512 | CHECKSUM_SHA512_128 => {
                let mut hasher = Sha512::new();
                hasher.update(&buf);
                let checksum: &[u8] = &hasher.finalize()[..];
                checksum[..16].try_into()?
            }
            t => {
                return Err(ZchunkError::InvalidChecksumType(t));
            }
        };

        if chunk.checksum != result {
            return Err(ZchunkError::ChunkChecksumNotMatch {
                len: length,
                expected: chunk.checksum,
                found: result,
            });
        }

        Ok(buf)
    }

    /// get uncompressed dict chunk
    fn get_uncompressed_dict(&mut self) -> Result<Option<Vec<u8>>, ZchunkError> {
        let dict_chunk = self.header.index.dict_chunk.clone();
        let data = self.get_chunk_data(0, &dict_chunk)?;

        let dict = if data.len() != 0 {
            Some(zstd::decode_all(Cursor::new(data))?)
        } else {
            None
        };

        Ok(dict)
    }

    /// decompress and assemble chunks, and write chunks to `Write`
    pub fn decompress_to(&mut self, mut writer: impl Write) -> Result<(), ZchunkError> {
        let dict = self.get_uncompressed_dict()?;

        // decompress data chunks
        for (chunk, _) in &self.header.index.data_chunks {
            let length = chunk.length.to_u64()?;
            let reader = &mut self.reader;
            let input = reader.take(length);
            // println!(
            //     "{} {} {:?}",
            //     chunk.uncompressed_length.to_u64()?,
            //     chunk.length.to_u64()?,
            //     chunk.checksum
            // );

            match dict {
                Some(ref d) => {
                    let mut decoder = zstd::Decoder::with_dictionary(input, &d)?;
                    io::copy(&mut decoder, &mut writer)?;
                }
                None => {
                    zstd::stream::copy_decode(input, &mut writer)?;
                }
            };
        }

        Ok(())
    }

    /// copy current zchunk reader to another writer, which using a cache zchunk file
    pub fn sync_to(
        &mut self,
        mut cache: Decoder<R>,
        mut writer: impl Write,
    ) -> Result<(), ZchunkError> {
        // write header
        self.header.write_to(&mut writer, false)?;

        // write dict
        let dict_chunk = self.header.index.dict_chunk.clone();
        let dict = if cache.header.has_dict_chunk(&dict_chunk) {
            cache.get_chunk_data(0, &dict_chunk)?
        } else {
            self.get_chunk_data(0, &dict_chunk)?
        };
        writer.write_all(&dict)?;

        // find existed chunks in cache
        let cache_chunk_offset_map = cache.header.find_data_chunks(
            self.header
                .index
                .data_chunks
                .clone()
                .into_iter()
                .map(|(c, _)| c)
                .collect(),
        );

        // write chunks
        for (chunk, offset) in self.header.index.data_chunks.clone() {
            let data = match cache_chunk_offset_map.get(&chunk) {
                Some(&o) => cache.get_chunk_data(o as u64, &chunk)?,
                None => self.get_chunk_data(offset as u64, &chunk)?,
            };
            writer.write_all(&data)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::BufReader};

    use sha2::{Digest, Sha256};
    use tempfile::Builder;

    use super::{Decoder, Encoder};
    fn test_decoder_inner(path: &str, checksum: &str) {
        let file = File::open(path).unwrap();
        let mut reader = BufReader::new(file);

        let mut decoder = Decoder::new(&mut reader).unwrap();

        let mut hasher = Sha256::new();
        decoder.decompress_to(&mut hasher).unwrap();

        assert_eq!(hex::encode(hasher.finalize()), checksum);
    }

    #[test]
    fn test_decompress() {
        test_decoder_inner("testdata/c25ffa05cf1fdeb67801847df96c33933b1ee1ea081af52edff4ff371a1c814c-comps-Server.x86_64.xml.zck",
        "14a39837e647b53517485cb00acc4d3cd989d13d68033213b1bb143330349f68");
        test_decoder_inner("testdata/3c6181c789ef9e8ed23f4072eb2f8f529002abd5166273a9734d7d39f7a810ae-comps-Server.x86_64.xml.zck",
        "4a1a7a9d98dd9764f67d4a608828fa8afca99889afe8b178228f5d37959c1ebf");
    }

    #[test]
    fn test_compress() {
        let input = File::open(
            "testdata/14a39837e647b53517485cb00acc4d3cd989d13d68033213b1bb143330349f68-comps-Server.x86_64.xml",
        )
        .unwrap();

        let path = "testdata/unittest-comps-Server.x86_64.xml.zck";
        let output = File::create(path).unwrap();

        let temp = Builder::new()
            .prefix("unittest-")
            .tempfile_in("testdata/")
            .unwrap();

        let mut encoder = Encoder::new(input, temp).unwrap();
        encoder.prepare_chunks().unwrap();
        encoder.compress_to(output).unwrap();

        test_decoder_inner(
            "testdata/unittest-primary.xml.zck",
            "14a39837e647b53517485cb00acc4d3cd989d13d68033213b1bb143330349f68",
        );
    }

    #[test]
    fn test_sync() {
        let source_file = File::open("testdata/c25ffa05cf1fdeb67801847df96c33933b1ee1ea081af52edff4ff371a1c814c-comps-Server.x86_64.xml.zck").unwrap();
        let mut source_reader = BufReader::new(source_file);

        let cache_file = File::open("testdata/3c6181c789ef9e8ed23f4072eb2f8f529002abd5166273a9734d7d39f7a810ae-comps-Server.x86_64.xml.zck").unwrap();
        let mut cache_reader = BufReader::new(cache_file);

        let mut source_decoder = Decoder::new(&mut source_reader).unwrap();
        let cache_decoder = Decoder::new(&mut cache_reader).unwrap();
        let mut hasher = Sha256::new();
        source_decoder.sync_to(cache_decoder, &mut hasher).unwrap();
        assert_eq!(
            hex::encode(hasher.finalize()),
            "c25ffa05cf1fdeb67801847df96c33933b1ee1ea081af52edff4ff371a1c814c"
        );
    }
}
