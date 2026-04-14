//! Trait framework for "parse or skip a thing at a known offset" — UE often
//! references arrays by `(count, offset)` pairs that may live elsewhere in the
//! file, so we model both the location (`StreamInfo`) and the read-time count
//! (`ReadInfo`) separately.

#![allow(dead_code)]

use crate::{
    archive::{SerializedFlags, SerializedObjectVersion},
    ObjectVersion, ObjectVersionUE5, Result,
};
use binread::BinReaderExt;
use std::io::{Read, Seek, SeekFrom};

mod implementations;
pub use implementations::*;

/// Element-count side of an array reference, captured when the parser is
/// already positioned at the right place to read it.
pub trait ReadInfo: Sized {
    fn get_count(&self) -> u64;

    fn from_current_position<R>(reader: &mut R) -> Result<Self>
    where
        R: Seek + Read + BinReaderExt;
}

/// File-offset side of a reference. Pairs with a `ReadInfo` to give the parser
/// everything needed to seek + walk the data.
pub trait StreamInfo: Sized {
    type ReadInfoType: ReadInfo;

    fn get_offset(&self) -> u64;

    fn from_current_position<R>(reader: &mut R) -> Result<Self>
    where
        R: Seek + Read + BinReaderExt;

    fn from_indirect_reference<R>(reader: &mut R) -> Result<Self>
    where
        R: Read + BinReaderExt;

    fn to_read_info(&self) -> Self::ReadInfoType;
}

/// Marker tying a parseable/skippable type to its `StreamInfo` flavour.
pub trait Deferrable {
    type StreamInfoType: StreamInfo;
}

/// Anything that can be deserialized into a concrete value.
pub trait Parseable: Deferrable
where
    Self: Sized,
{
    type ParsedType: Sized;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        read_info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags;

    fn parse_with_info<R>(
        reader: &mut R,
        stream_info: &Self::StreamInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        reader.seek(SeekFrom::Start(stream_info.get_offset()))?;
        Self::parse_with_info_seekless(reader, &stream_info.to_read_info())
    }

    /// Parse "right here, right now" — picks up both the location and the
    /// count from the reader's current position.
    fn parse_inline<R>(reader: &mut R) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        let info = <Self::StreamInfoType as StreamInfo>::ReadInfoType::from_current_position(reader)?;
        Self::parse_with_info_seekless(reader, &info)
    }

    /// Parse a `(count, offset)` reference at the cursor and follow it to read
    /// the data, then restore the cursor to where the reference ended.
    fn parse_indirect<R>(reader: &mut R) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        let info = Self::StreamInfoType::from_indirect_reference(reader)?;
        let saved = reader.stream_position()?;
        let parsed = Self::parse_with_info(reader, &info)?;
        reader.seek(SeekFrom::Start(saved))?;
        Ok(parsed)
    }
}

/// Anything that can be advanced past without producing a value.
pub trait Skippable: Deferrable {
    fn seek_past_with_info<R>(reader: &mut R, stream_info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read;

    fn seek_past<R>(reader: &mut R) -> Result<()>
    where
        R: Seek + Read,
    {
        let info = Self::StreamInfoType::from_current_position(reader)?;
        Self::seek_past_with_info(reader, &info)
    }
}

#[derive(Debug)]
pub struct SingleItemReadInfo {}

impl ReadInfo for SingleItemReadInfo {
    fn get_count(&self) -> u64 {
        1
    }

    fn from_current_position<R>(_reader: &mut R) -> Result<Self> {
        Ok(Self {})
    }
}

#[derive(Debug)]
pub struct SingleItemStreamInfo {
    pub offset: u64,
}

impl SingleItemStreamInfo {
    pub fn from_stream<R>(reader: &mut R) -> Result<Self>
    where
        R: Seek,
    {
        Ok(Self {
            offset: reader.stream_position()?,
        })
    }
}

impl StreamInfo for SingleItemStreamInfo {
    type ReadInfoType = SingleItemReadInfo;

    fn get_offset(&self) -> u64 {
        self.offset
    }

    fn from_current_position<R>(reader: &mut R) -> Result<Self>
    where
        R: Read + Seek,
    {
        Ok(Self {
            offset: reader.stream_position()?,
        })
    }

    fn from_indirect_reference<R>(reader: &mut R) -> Result<Self>
    where
        R: Read + BinReaderExt,
    {
        let off: i32 = reader.read_le()?;
        Ok(Self { offset: off as u64 })
    }

    fn to_read_info(&self) -> Self::ReadInfoType {
        SingleItemReadInfo {}
    }
}

#[derive(Debug)]
pub struct ArrayReadInfo {
    pub count: u64,
}

impl ReadInfo for ArrayReadInfo {
    fn get_count(&self) -> u64 {
        self.count
    }

    fn from_current_position<R>(reader: &mut R) -> Result<Self>
    where
        R: Seek + Read + BinReaderExt,
    {
        let n: i32 = reader.read_le()?;
        Ok(Self { count: n as u64 })
    }
}

#[derive(Debug, Clone)]
pub struct ArrayStreamInfo {
    pub offset: u64,
    pub count: u64,
}

impl StreamInfo for ArrayStreamInfo {
    type ReadInfoType = ArrayReadInfo;

    fn get_offset(&self) -> u64 {
        self.offset
    }

    fn from_current_position<R>(reader: &mut R) -> Result<Self>
    where
        R: Seek + Read + BinReaderExt,
    {
        let n: i32 = reader.read_le()?;
        Ok(Self {
            offset: reader.stream_position()?,
            count: n as u64,
        })
    }

    fn from_indirect_reference<R>(reader: &mut R) -> Result<Self>
    where
        R: Read + BinReaderExt,
    {
        let n: i32 = reader.read_le()?;
        let off: i32 = reader.read_le()?;
        Ok(Self {
            offset: off as u64,
            count: n as u64,
        })
    }

    fn to_read_info(&self) -> Self::ReadInfoType {
        ArrayReadInfo { count: self.count }
    }
}
