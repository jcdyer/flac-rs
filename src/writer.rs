/// The writer is responsible for turning structures into bytes in a file.
use std::{io::{self, SeekFrom}, marker::PhantomData};

use bitwriter::BitWriter;
use md5::{Digest, Md5};

use crate::{
    frame::{Frame, Sample},
    headers::{MetadataBlock, MetadataBlockStreamInfo},
};

pub struct HeaderWriter<W, S> {
    w: W,
    stream_info: MetadataBlockStreamInfo,
    md5: md5::Md5,
    _s: PhantomData<S>,
}

impl<W: std::io::Write, S> HeaderWriter<W, S> {
    pub fn new(w: W, stream_info: MetadataBlockStreamInfo) -> HeaderWriter<W, S> {
        HeaderWriter {
            w,
            stream_info,
            md5: md5::Md5::default(),
            _s: PhantomData,
        }
    }
    pub fn write_headers(
        mut self,
        headers: impl IntoIterator<Item = MetadataBlock>,
    ) -> io::Result<FrameWriter<W, S>> {
        let mut writer = BitWriter::with_capacity(4096);

        writer.put(32, u32::from_be_bytes(*b"fLaC"));
        let mut headers = headers.into_iter().peekable();
        let is_last_header = headers.peek().is_none();
        self.stream_info.put_into(is_last_header, &mut writer);
        while let Some(header) = headers.next() {
            let is_last_header = headers.peek().is_none();
            header.put_into(is_last_header, &mut writer);
        }

        let bytes = writer.finish();
        self.w.write_all(&bytes)?;

        Ok(FrameWriter {
            w: self.w,
            stream_info: self.stream_info,
            md5: self.md5,
            _s: self._s,
        })
    }
}

pub struct FrameWriter<W, S> {
    w: W,
    stream_info: MetadataBlockStreamInfo,
    md5: md5::Md5,
    _s: PhantomData<S>,
}

// TODO: Make generic over <W, S: Sample>
impl<W: io::Write> FrameWriter<W, i16> {
    pub fn write_frame(&mut self, frame: Frame<i16>) -> io::Result<()> {
        let mut writer = BitWriter::with_capacity(5000);
        frame.put_into(&mut writer);
        let bytes = writer.finish();
        self.w.write_all(&bytes)?;
        Ok(())
    }

}

impl <W: io::Write + io::Seek, S> FrameWriter<W, S> {
    /// Call at the very end to fill in metadata about information learned by encoding the file
    /// This includes the MD5 sum, seek table, etc.
    pub fn finish(&mut self) -> io::Result<()> {
        self.w.seek(SeekFrom::Start(26))?; // Location of MD5 hash
        let md5 = std::mem::take(&mut self.md5);
        //self.w.write_all(&md5.finalize()[..])?;
        Ok(())
    }
}
