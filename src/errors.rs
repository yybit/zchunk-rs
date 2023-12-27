use std::{array::TryFromSliceError, io};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ZchunkError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    TryFromSlice(#[from] TryFromSliceError),

    #[error("invalid leader id: {0:?}")]
    InvalidLeaderID([u8; 5]),

    #[error("invalid checksum type: {0}")]
    InvalidChecksumType(u8),

    #[error("invalid compression type: {0}")]
    InvalidCompresionType(u8),

    #[error("invalid header magic (expected {expected}, found {found})")]
    InvalidHeaderMagic { expected: u32, found: u32 },

    #[error("invalid header size (expected {expected}, found {found})")]
    InvalidHeaderSize { expected: u64, found: u64 },

    #[error("invalid index size (expected {expected}, found {found})")]
    InvalidIndexSize { expected: u64, found: u64 },

    #[error("the size of footer and entries does not match (expected {expected}, found {found})")]
    SizeNotMatch { expected: u32, found: u32 },

    #[error("header not found")]
    HeaderNotFound,

    #[error("chunk not found, index: {0}")]
    ChunkNotFound(usize),

    #[error("chunk checksum not match (len {len} expected {expected:?}, found {found:?})")]
    ChunkChecksumNotMatch {
        len: usize,
        expected: [u8; 16],
        found: [u8; 16],
    },
}
