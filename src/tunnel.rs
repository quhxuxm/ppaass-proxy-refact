use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Result;
use futures::Stream;
use ppaass_common::RsaCryptoFetcher;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, ToSocketAddrs};

use crate::tunnel::agent::AgentConnection;
use crate::tunnel::target::TargetConnection;

mod agent;
mod target;

trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Unpin {}

impl<T> AsyncReadAndWrite for T where T: AsyncRead + AsyncWrite + Unpin {}

// impl AsyncReadAndWrite for Box<dyn AsyncReadAndWrite> {}

pub(crate) enum TcpTunnelStatus {
    New,
    TargetAssigned,
    TargetConnected,
    TargetDisconnected,
    Closed,
}
pub(crate) struct TcpTunnel<A, F>
where
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    agent_connection: AgentConnection<A, F>,
    target_connection: Option<TargetConnection<A>>,
    status: TcpTunnelStatus,
}

impl<A, F> TcpTunnel<A, F>
where
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    pub(crate) fn new(
        agent_tcp_rw: Box<dyn AsyncReadAndWrite>, agent_addresses: A, rsa_crypto_fetcher: Arc<F>, compress: bool, buffer_size: usize,
    ) -> Result<Self> {
        let agent_connection = AgentConnection::new(agent_tcp_rw, agent_addresses, rsa_crypto_fetcher, compress, buffer_size);
        Ok(Self {
            agent_connection,
            target_connection: None,
            status: TcpTunnelStatus::New,
        })
    }

    async fn connect_to_target(&mut self, target_tcp_addresses: A) -> Result<()> {
        let target_tcp_stream = TcpStream::connect(&target_tcp_addresses).await?;
        self.status = TcpTunnelStatus::TargetConnected;
        self.target_connection = Some(TargetConnection::new(Box::new(target_tcp_stream), target_tcp_addresses));
        Ok(())
    }
}
impl<A, F> Future for TcpTunnel<A, F>
where
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    type Output = Result<(), anyhow::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.status {
            TcpTunnelStatus::New => {
                return Poll::Pending;
            },
            _ => {
                return Poll::Pending;
            },
        }
    }
}
