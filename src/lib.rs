use std::{
    convert::Infallible,
    pin::Pin,
    task::Context
};
use futures::{
    Future,
    Poll
};

struct ReadLineFuture<R>(R);

impl<R: tokio::io::AsyncRead> Future for ReadLineFuture<R> {
    type Output = Infallible;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let mut byte = [0x8, 1];
        self.0.poll_read(ctx, &mut byte)
    }
}
