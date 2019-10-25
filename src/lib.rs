use std::{
    io,
    pin::Pin,
    task::Context
};
use futures::{
    Future,
    Poll
};

enum ReadLineState<R> {
    Error(io::Error),
    Read(Vec<u8>, R),
    Done,
}

struct ReadLineFuture<R>(ReadLineState<R>);

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

impl<R: tokio::io::AsyncRead> Future for ReadLineFuture<R> {
    type Output = io::Result<(Option<String>, R)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        match self.0 {
            ReadLineState::Read(_, input) => {
                let mut byte = [0x8, 1];
                input.poll_read(ctx, &mut byte)
            }
            _ => unimplemented!()
        }
    }
}
