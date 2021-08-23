/// The writer is responsible for turning structures into bytes in a file.

use std::{fmt::Write, io, marker::PhantomData};

use bytes::{BufMut, BytesMut};

use crate::{frame::Frame, headers::{MetadataBlock, MetadataBlockStreamInfo}};

pub struct HeaderWriter<W, S> {
    w: W,
    stream_info: MetadataBlockStreamInfo,
    md5: md5::Md5,
    _s: PhantomData<S>
}

impl<W: Write, S> HeaderWriter<W, S> {
    pub fn new(&mut self, w: W, stream_info: MetadataBlockStreamInfo) -> HeaderWriter<W, S> {
        HeaderWriter {
            w,
            stream_info,
            md5: md5::Md5::default(),
            _s: PhantomData,
        }
    }
    pub fn write_headers(mut self, headers: impl IntoIterator<Item=MetadataBlock>) -> io::Result<FrameWriter<W, S>> {
        let mut buf = BytesMut::with_capacity(4096);
        buf.put(&b"fLaC"[..]);
        let mut headers = headers.into_iter().peekable();
        let last = headers.peek().is_none();
        self.stream_info.put_into(last, &mut buf);
        while let Some(header) = headers.next() {
            header.put_into(headers.peek().is_none(), &mut buf);
        }
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

impl<W: Write, S> FrameWriter<W, S> {
    pub fn write_frame(f: Frame<S>) -> io::Result<()> {
        let buf  = BytesMut::with_capacity(5000);
        todo!()
    }
}
