use std::sync::Arc;

use std::{net::ToSocketAddrs, pin::Pin};

use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use pin_project::pin_project;
use ppaass_common::{
    generate_uuid, AgentMessagePayloadTypeValue, DomainResolveRequest, Message, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, PpaassError,
    ProxyMessagePayloadTypeValue, RsaCryptoFetcher, WriteMessageFramedRequest, WriteMessageFramedResult,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tracing::{debug, error};

use crate::tunnel::agent::AgentConnection;
use crate::tunnel::target::TargetConnection;

mod agent;
mod target;

pub(crate) trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Unpin {}

impl<T> AsyncReadAndWrite for T where T: AsyncRead + AsyncWrite + Unpin {}

pub(crate) enum TcpTunnelStatus<T, F>
where
    T: AsyncReadAndWrite,
    F: RsaCryptoFetcher,
{
    AgentTcpConnected {
        agent_tcp_connection: AgentConnection<T, NetAddress, F>,
    },
    TargetTcpConnected {
        agent_tcp_connection: AgentConnection<T, NetAddress, F>,
        target_tcp_stream: TcpStream,
    },
    TargetTcpDisconnected {
        agent_tcp_connection: AgentConnection<T, NetAddress, F>,
    },
    AgentTcpDisconnected,
    Closed,
}

pub(crate) struct TcpTunnel<T, F>
where
    T: AsyncReadAndWrite,
    F: RsaCryptoFetcher,
{
    status: TcpTunnelStatus<T, F>,
}

impl<T, F> TcpTunnel<T, F>
where
    T: AsyncReadAndWrite,
    F: RsaCryptoFetcher,
{
    pub(crate) fn new(agent_tcp_rw: T, agent_addresses: NetAddress, rsa_crypto_fetcher: Arc<F>, compress: bool, buffer_size: usize) -> Self {
        let agent_tcp_connection = AgentConnection::new(agent_tcp_rw, agent_addresses, rsa_crypto_fetcher, compress, buffer_size);
        Self {
            status: TcpTunnelStatus::AgentTcpConnected { agent_tcp_connection },
        }
    }

    pub async fn exec(mut self) -> Result<()> {
        loop {
            match self.status {
                TcpTunnelStatus::AgentTcpConnected { agent_tcp_connection } => {
                    debug!("Connection in AgentConnected status.");
                    let agent_message = agent_tcp_connection.next().await.ok_or_else(|| {
                        error!("Fail to read tcp connect from agent connection because of nothing to read.");
                        PpaassError::CodecError
                    })??;
                    let agent_message_payload = agent_message.payload.ok_or_else(|| {
                        error!("Fail to read tcp connect from agent connection because of nothing to read.");
                        PpaassError::CodecError
                    })?;
                    let agent_message_payload: MessagePayload = agent_message_payload.try_into()?;
                    if let MessagePayload {
                        source_address,
                        target_address,
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                        data,
                    } = agent_message_payload
                    {
                        let target_tcp_stream = TcpStream::connect(
                            target_address
                                .as_ref()
                                .ok_or_else(|| {
                                    error!("Fail to read tcp connect from agent connection because of nothing to read.");
                                    PpaassError::CodecError
                                })?
                                .try_into()?,
                        )
                        .await?;
                        self.status = TcpTunnelStatus::TargetTcpConnected {
                            agent_tcp_connection,
                            target_tcp_stream,
                        };
                        continue;
                    }
                    if let MessagePayload {
                        source_address,
                        target_address,
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat),
                        data,
                    } = agent_message_payload
                    {
                        continue;
                    }
                    if let MessagePayload {
                        source_address,
                        target_address,
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::DomainResolve),
                        data,
                    } = agent_message_payload
                    {
                        continue;
                    }
                    return Err(anyhow!(PpaassError::CodecError));
                },
                TcpTunnelStatus::TargetTcpConnected {
                    agent_tcp_connection,
                    target_tcp_stream,
                } => {
                    debug!("Connection in TargetConnected status.");
                    let agent_message = agent_tcp_connection.next().await;
                    let agent_message = match agent_message {
                        None => {
                            return Ok(());
                        },
                        Some(v) => v?,
                    };
                    let agent_message_payload = agent_message.payload.ok_or_else(|| {
                        error!("Fail to read tcp connect from agent connection because of nothing to read.");
                        PpaassError::CodecError
                    })?;
                    let agent_message_payload: MessagePayload = agent_message_payload.try_into()?;
                    if let MessagePayload {
                        source_address,
                        target_address,
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                        data,
                    } = agent_message_payload
                    {
                        let target_tcp_stream = TcpStream::connect(
                            target_address
                                .as_ref()
                                .ok_or_else(|| {
                                    error!("Fail to read tcp connect from agent connection because of nothing to read.");
                                    PpaassError::CodecError
                                })?
                                .try_into()?,
                        )
                        .await?;
                        self.status = TcpTunnelStatus::TargetTcpConnected {
                            agent_tcp_connection,
                            target_tcp_stream,
                        };
                        continue;
                    }
                },
                TcpTunnelStatus::TargetTcpDisconnected => {
                    debug!("Connection in TargetConnected status.");
                    this = TcpTunnel::on_target_tcp_connected(this).await?
                },
                TcpTunnelStatus::AgentTcpDisconnected => todo!(),
                TcpTunnelStatus::Closed => todo!(),
            }
        }
    }
}
