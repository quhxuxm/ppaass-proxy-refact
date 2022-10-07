use std::sync::Arc;
use std::task::{Context, Poll};
use std::{net::ToSocketAddrs, pin::Pin};

use futures::{Sink, Stream};
use pin_project::pin_project;
use ppaass_common::{Message, MessageCodec, PpaassError, RsaCryptoFetcher};

use tokio_util::codec::Framed;

use crate::tunnel::AsyncReadAndWrite;

#[pin_project]
pub(crate) struct AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    #[pin]
    message_framed: Framed<T, MessageCodec<F>>,
    addresses: A,
}

impl<T, A, F> AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    pub(crate) fn new(concrete_read_write: T, addresses: A, rsa_crypto_fetcher: Arc<F>, compress: bool, buffer_size: usize) -> Self {
        let message_framed = Framed::with_capacity(concrete_read_write, MessageCodec::<F>::new(compress, rsa_crypto_fetcher), buffer_size);
        Self { message_framed, addresses }
    }
}

impl<T, A, F> Stream for AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    type Item = Result<Message, PpaassError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.message_framed.poll_next(cx)
    }
}
impl<T, A, F> Sink<Message> for AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    type Error = PpaassError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.message_framed.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let this = self.project();
        this.message_framed.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.message_framed.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.message_framed.poll_close(cx)
    }
}
