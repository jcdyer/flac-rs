/// The writer is responsible for turning structures into bytes in a file.

use std::{fmt::Write, io, marker::PhantomData};

use bitwriter::BitWriter;

use crate::{frame::{Frame, Sample}, headers::{MetadataBlock, MetadataBlockStreamInfo}};

pub struct HeaderWriter<W, S> {
    w: W,
    stream_info: MetadataBlockStreamInfo,
    md5: md5::Md5,
    _s: PhantomData<S>
}

impl<W: std::io::Write, S> HeaderWriter<W, S> {
    pub fn new(&mut self, w: W, stream_info: MetadataBlockStreamInfo) -> HeaderWriter<W, S> {
        HeaderWriter {
            w,
            stream_info,
            md5: md5::Md5::default(),
            _s: PhantomData,
        }
    }
    pub fn write_headers(mut self, headers: impl IntoIterator<Item=MetadataBlock>) -> io::Result<FrameWriter<W, S>> {

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

impl<W: Write, S: Sample> FrameWriter<W, S> {
    pub fn write_frame(f: Frame<S>) -> io::Result<()> {
        let mut writer  = BitWriter::with_capacity(5000);

        f.put_into(&mut writer);
        todo!()
    }
}
