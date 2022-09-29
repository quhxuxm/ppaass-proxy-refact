use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Sink, Stream};
use pin_project::pin_project;
use ppaass_common::{Message, MessageCodec, PpaassError, RsaCryptoFetcher};
use tokio::net::ToSocketAddrs;
use tokio_util::codec::Framed;

use crate::tunnel::AsyncReadAndWrite;

#[pin_project]
pub(crate) struct AgentConnection<A, F>
where
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    #[pin]
    message_framed: Framed<Box<dyn AsyncReadAndWrite>, MessageCodec<F>>,
    addresses: A,
}

impl<A, F> AgentConnection<A, F>
where
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    pub(crate) fn new(concrete_read_write: Box<dyn AsyncReadAndWrite>, addresses: A, rsa_crypto_fetcher: Arc<F>, compress: bool, buffer_size: usize) -> Self {
        let message_framed = Framed::with_capacity(concrete_read_write, MessageCodec::<F>::new(compress, rsa_crypto_fetcher), buffer_size);
        Self { message_framed, addresses }
    }
}

impl<A, F> Stream for AgentConnection<A, F>
where
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    type Item = Result<Message, PpaassError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.message_framed.poll_next(cx)
    }
}
impl<A, F> Sink<Message> for AgentConnection<A, F>
where
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
