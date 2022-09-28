use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{anyhow, Result};
use futures::future::IntoStream;
use futures::stream::IntoAsyncRead;
use ppaass_common::{MessageFramedGenerator, MessageFramedRead, MessageFramedWrite, NetAddress, PpaassError, RsaCryptoFetcher};
use tokio::io::Interest;
use tokio::net::TcpStream;

pub(crate) enum ProxyTcpChannelStatus {
    New,
    TargetAssigned(Vec<SocketAddr>),
    TargetConnected {
        target_tcp_stream: TcpStream,
        target_tcp_addresses: Vec<SocketAddr>,
    },
    TargetDisconnected,
    Closed,
}
pub(crate) struct ProxyTcpChannel<T>
where
    T: RsaCryptoFetcher,
{
    agent_message_framed_read: MessageFramedRead<T, TcpStream>,
    agent_message_framed_write: MessageFramedWrite<T, TcpStream>,
    agent_address: SocketAddr,
    status: ProxyTcpChannelStatus,
}

impl<T> ProxyTcpChannel<T>
where
    T: RsaCryptoFetcher,
{
    pub(crate) fn new(agent_tcp_stream: TcpStream, agent_address: SocketAddr, rsa_crypto_fetcher: Arc<T>) -> Result<Self> {
        let (agent_message_framed_read, agent_message_framed_write) = MessageFramedGenerator::generate(agent_tcp_stream, 64 * 1024, false, rsa_crypto_fetcher)?;
        Ok(Self {
            agent_message_framed_read,
            agent_message_framed_write,
            agent_address,
            status: ProxyTcpChannelStatus::New,
        })
    }

    async fn connect_to_target(&mut self, target_address: NetAddress) -> Result<()> {
        let target_tcp_addresses: Vec<SocketAddr> = target_address.try_into()?;
        let target_tcp_stream = TcpStream::connect(target_tcp_addresses.as_slice()).await?;
        self.status = ProxyTcpChannelStatus::TargetConnected {
            target_tcp_stream,
            target_tcp_addresses,
        };
        Ok(())
    }
}
impl<T> Future for ProxyTcpChannel<T>
where
    T: RsaCryptoFetcher,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.status {
            ProxyTcpChannelStatus::New => {
                self.get_mut().connect_to_target();
                return Poll::Pending;
            },
        }
        todo!()
    }
}

impl ProxyTcpChannel {
    pub(crate) async fn relay(&mut self) -> Result<()> {
        match self.status {
            ProxyTcpChannelStatus::Initialized => return Err(anyhow!("Connection not connect to target.")),
            ProxyTcpChannelStatus::TargetDisconnected => return Err(anyhow!("Connection disconnect target already.")),
            ProxyTcpChannelStatus::Closed => return Err(anyhow!("Connection closed already.")),
            ProxyTcpChannelStatus::TargetConnected {
                target_tcp_addresses,
                target_tcp_stream,
            } => loop {
                let ready_for_agent = self.agent_tcp_stream.ready(Interest::READABLE | Interest::WRITABLE).await?;
            },
        }
    }
}
