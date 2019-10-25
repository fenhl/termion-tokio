use std::{
    io::{self, Write},
    iter::empty,
    mem::replace,
    pin::Pin,
    task::Context
};

use bytes::{Buf, BytesMut, IntoBuf};
use futures::{
    Future,
    Poll::{self, *},
    Stream
};
use tokio::{
    codec::{Decoder, FramedRead},
    io::AsyncRead
};

use termion::event::{self, Event, Key};
use termion::raw::IntoRawMode;

/// A stream of input keys.
pub struct KeysStream<R> {
    inner: EventsStream<R>,
}

impl<R: AsyncRead + Unpin> Stream for KeysStream<R> {
    type Item = io::Result<Key>;

    fn poll_next(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            break match Pin::new(&mut self.inner).poll_next(ctx) {
                Ready(Some(Ok(Event::Key(k)))) => Ready(Some(Ok(k))),
                Ready(Some(Ok(_))) => continue,
                Ready(Some(Err(e))) => Ready(Some(Err(e))),
                Ready(None) => Ready(None),
                Pending => Pending,
            }
        }
    }
}

/// An iterator over input events.
pub struct EventsStream<R> {
    inner: EventsAndRawStream<R>,
}

impl<R: AsyncRead + Unpin> Stream for EventsStream<R> {
    type Item = io::Result<Event>;

    fn poll_next(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner)
            .poll_next(ctx)
            .map_ok(|(event, _)| event)
    }
}

/// An iterator over input events and the bytes that define them
type EventsAndRawStream<R> = FramedRead<R, EventsAndRawDecoder>;

pub struct EventsAndRawDecoder;

impl Decoder for EventsAndRawDecoder {
    type Item = (Event, Vec<u8>);
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match src.len() {
            0 => Ok(None),
            1 => match src[0] {
                b'\x1B' => {
                    src.advance(1);
                    Ok(Some((Event::Key(Key::Esc), vec![b'\x1B'])))
                }
                c => {
                    if let Ok(res) = parse_event(c, &mut empty()) {
                        src.advance(1);
                        Ok(Some(res))
                    } else {
                        Ok(None)
                    }
                }
            },
            _ => {
                let (off, res) = if let Some((c, cs)) = src.split_first() {
                    let cur = cs.into_buf();
                    let mut it = cur.iter().map(Ok);
                    if let Ok(res) = parse_event(*c, &mut it) {
                        (1 + cs.len() - it.len(), Ok(Some(res)))
                    } else {
                        (0, Ok(None))
                    }
                } else {
                    (0, Ok(None))
                };

                src.advance(off);
                res
            }
        }
    }
}

fn parse_event<I>(item: u8, iter: &mut I) -> io::Result<(Event, Vec<u8>)>
where
    I: Iterator<Item = io::Result<u8>>,
{
    let mut buf = vec![item];
    let result = {
        let mut iter = iter.inspect(|byte| {
            if let &Ok(byte) = byte {
                buf.push(byte);
            }
        });
        event::parse_event(item, &mut iter)
    };
    result
        .or(Ok(Event::Unsupported(buf.clone())))
        .map(|e| (e, buf))
}

enum ReadLineState<R> {
    Error(io::Error),
    Read(Vec<u8>, R),
    Done,
}

pub struct ReadLineFuture<R>(ReadLineState<R>);

impl<R> From<io::Error> for ReadLineFuture<R> {
    fn from(error: io::Error) -> Self {
        ReadLineFuture(ReadLineState::Error(error))
    }
}

impl<R> ReadLineFuture<R> {
    fn new(source: R) -> Self {
        ReadLineFuture(ReadLineState::Read(Vec::with_capacity(30), source))
    }
}

impl<R: Unpin + AsyncRead> Future for ReadLineFuture<R> {
    type Output = io::Result<(Option<String>, R)>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        use self::ReadLineState::*;
        match replace(&mut self.0, Done) {
            Error(error) => Ready(Err(error)),
            Read(mut buf, mut input) => loop {
                let mut byte = [0x8, 1];

                match Pin::new(&mut input).poll_read(ctx, &mut byte) {
                    Ready(Ok(1)) => match byte[0] {
                        0 | 3 | 4 => return Ready(Ok((None, input))),
                        0x7f => {
                            buf.pop();
                        }
                        b'\n' | b'\r' => {
                            let string = String::from_utf8(buf)
                                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                            return Ready(Ok((Some(string), input)));
                        }
                        c => {
                            buf.push(c);
                        }
                    },
                    Ready(Ok(_)) => continue,
                    Pending => (),
                    Ready(Err(e)) => return Ready(Err(e)),
                }

                self.0 = Read(buf, input);
                break Pending
            }
            Done => unreachable!(),
        }
    }
}

/// Extension to `Read` trait.
pub trait TermReadAsync: Sized {
    /// An iterator over input events.
    fn events_stream(self) -> EventsStream<Self>
    where
        Self: Sized;

    /// An iterator over key inputs.
    fn keys_stream(self) -> KeysStream<Self>
    where
        Self: Sized;

    /// Read a line.
    ///
    /// EOT and ETX will abort the prompt, returning `None`. Newline or carriage return will
    /// complete the input.
    fn read_line_future(self) -> ReadLineFuture<Self>;

    /// Read a password.
    ///
    /// EOT and ETX will abort the prompt, returning `None`. Newline or carriage return will
    /// complete the input.
    fn read_passwd_future<W: Write>(self, writer: &mut W) -> ReadLineFuture<Self> {
        if let Err(error) = writer.into_raw_mode() {
            error.into()
        } else {
            self.read_line_future()
        }
    }
}

impl<R: AsyncRead + TermReadAsyncEventsAndRaw> TermReadAsync for R {
    fn events_stream(self) -> EventsStream<Self> {
        EventsStream {
            inner: self.events_and_raw_stream(),
        }
    }
    fn keys_stream(self) -> KeysStream<Self> {
        KeysStream {
            inner: self.events_stream(),
        }
    }

    fn read_line_future(self) -> ReadLineFuture<Self> {
        ReadLineFuture::new(self)
    }
}

/// Extension to `TermReadAsync` trait. A separate trait in order to maintain backwards compatibility.
pub trait TermReadAsyncEventsAndRaw {
    /// An iterator over input events and the bytes that define them.
    fn events_and_raw_stream(self) -> EventsAndRawStream<Self>
    where
        Self: Sized;
}

impl<R: AsyncRead> TermReadAsyncEventsAndRaw for R {
    fn events_and_raw_stream(self) -> EventsAndRawStream<Self> {
        FramedRead::new(self, EventsAndRawDecoder)
    }
}
