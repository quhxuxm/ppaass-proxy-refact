use std::pin::Pin;
use std::task::{Context, Poll};

use futures::AsyncRead;
use pin_project::pin_project;
use tokio::io::AsyncRead;
use tokio::net::ToSocketAddrs;

use crate::tunnel::AsyncReadAndWrite;

#[pin_project]
pub(crate) struct TargetConnection<A>
where
    A: ToSocketAddrs,
{
    #[pin]
    concrete_rw: Box<dyn AsyncReadAndWrite>,
    addresses: A,
}

impl<A> TargetConnection<A>
where
    A: ToSocketAddrs,
{
    pub(crate) fn new(concrete_rw: Box<dyn AsyncReadAndWrite>, addresses: A) -> Self {
        Self { concrete_rw, addresses }
    }
}

impl<A> AsyncRead for TargetConnection<A>
where
    A: ToSocketAddrs,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        this.concrete_rw.poll_read(cx, buf)
    }
}
