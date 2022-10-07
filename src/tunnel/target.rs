use std::task::{Context, Poll};
use std::{net::ToSocketAddrs, pin::Pin};

use pin_project::pin_project;
use tokio::io::AsyncRead;

use crate::tunnel::AsyncReadAndWrite;

#[pin_project]
pub(crate) struct TargetConnection<T, A>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
{
    #[pin]
    concrete_rw: T,
    addresses: A,
}

impl<T, A> TargetConnection<T, A>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
{
    pub(crate) fn new(concrete_rw: T, addresses: A) -> Self {
        Self { concrete_rw, addresses }
    }
}

impl<T, A> AsyncRead for TargetConnection<T, A>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = self.project();
        this.concrete_rw.poll_read(cx, buf)
    }
}
