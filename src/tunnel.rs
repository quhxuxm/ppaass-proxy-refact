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

pub(crate) enum TcpTunnelStatus {
    AgentTcpConnected,
    TargetTcpConnected,
    TargetTcpDisconnected,
    AgentTcpDisconnected,
    Closed,
}

#[pin_project(project=TcpTunnelProject)]
pub(crate) struct TcpTunnel<T, F>
where
    T: AsyncReadAndWrite,
    F: RsaCryptoFetcher,
{
    #[pin]
    agent_connection: AgentConnection<T, NetAddress, F>,
    #[pin]
    target_connection: Option<TargetConnection<TcpStream, NetAddress>>,
    status: TcpTunnelStatus,
}

impl<T, F> TcpTunnel<T, F>
where
    T: AsyncReadAndWrite,
    F: RsaCryptoFetcher,
{
    pub(crate) fn new(agent_tcp_rw: T, agent_addresses: NetAddress, rsa_crypto_fetcher: Arc<F>, compress: bool, buffer_size: usize) -> Result<Self> {
        let agent_connection = AgentConnection::new(agent_tcp_rw, agent_addresses, rsa_crypto_fetcher, compress, buffer_size);
        Ok(Self {
            agent_connection,
            target_connection: None,
            status: TcpTunnelStatus::AgentTcpConnected,
        })
    }

    async fn on_agent_tcp_connected(mut this: TcpTunnelProject<'_, T, F>) -> Result<TcpTunnelProject<'_, T, F>> {
        let agent_message = this.agent_connection.next().await;
        match agent_message {
            None => {
                return Ok(this);
            },
            Some(Err(e)) => {
                *this.status = TcpTunnelStatus::Closed;
                this.agent_connection.close().await?;
                return Err(anyhow!(e));
            },
            Some(Ok(Message {
                id,
                ref_id,
                connection_id,
                user_token,
                payload_encryption,
                payload: None,
            })) => {
                error!("Fail to read agent connection because of no payload, message id: {id}, message reference id: {ref_id:?}, client connection id: {connection_id:?}, user token: {user_token}");
                *this.status = TcpTunnelStatus::Closed;
                this.agent_connection.close().await?;
                return Err(anyhow!(PpaassError::CodecError));
            },
            Some(Ok(Message {
                id,
                ref_id,
                connection_id,
                user_token,
                payload_encryption,
                payload: Some(payload_bytes),
            })) => {
                let agent_message_payload = payload_bytes.try_into();
                match agent_message_payload {
                    Err(e) => {
                        *this.status = TcpTunnelStatus::Closed;
                        this.agent_connection.close().await?;
                        return Err(anyhow!(e));
                    },
                    Ok(MessagePayload {
                        source_address,
                        target_address,
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                        ..
                    }) => match target_address {
                        None => {
                            *this.status = TcpTunnelStatus::Closed;
                            Err(anyhow!(PpaassError::CodecError))
                        },
                        Some(target_address) => {
                            let target_socket_addresses = target_address.clone().to_socket_addrs()?.collect::<Vec<_>>();
                            let target_tcp_stream = TcpStream::connect(target_socket_addresses.as_slice()).await?;
                            *this.status = TcpTunnelStatus::TargetTcpConnected;
                            *this.target_connection = Some(TargetConnection::new(target_tcp_stream, target_address.clone()));

                            let tcp_connect_success_message_payload = MessagePayload {
                                source_address,
                                target_address: Some(target_address),
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                                data: None,
                            };
                            let tcp_connect_success_message = Message {
                                id: generate_uuid(),
                                ref_id: Some(id),
                                connection_id,
                                user_token,
                                payload_encryption,
                                payload: Some(tcp_connect_success_message_payload.try_into()?),
                            };
                            this.agent_connection.send(tcp_connect_success_message).await?;
                            Ok(this)
                        },
                    },
                    Ok(MessagePayload {
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat),
                        ..
                    }) => {
                        debug!("Handle connection heartbeat");
                        let PayloadEncryptionTypeSelectResult {
                            payload_encryption_type: payload_encryption,
                            ..
                        } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: user_token.as_str(),
                        })
                        .await?;
                        let heartbeat_success_message_payload = MessagePayload {
                            source_address: None,
                            target_address: None,
                            payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::HeartbeatSuccess),
                            data: None,
                        };
                        let heartbeat_success_message = Message {
                            id: generate_uuid(),
                            ref_id: Some(id),
                            connection_id,
                            user_token,
                            payload_encryption,
                            payload: Some(heartbeat_success_message_payload.try_into()?),
                        };
                        this.agent_connection.send(heartbeat_success_message).await?;
                        Ok(this)
                    },
                    Ok(MessagePayload {
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::DomainResolve),
                        data,
                        ..
                    }) => {
                        debug!("Handle domain name resolve.");
                        let data = data.ok_or(anyhow!(PpaassError::CodecError))?;
                        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } =
                            PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                                encryption_token: generate_uuid().into(),
                                user_token: user_token.as_str(),
                            })
                            .await?;
                        let DomainResolveRequest { id, name } = serde_json::from_slice(data.as_ref())?;
                        let (message_framed_write, message_framed_read) = async move {
                    return match dns_lookup::lookup_host(name.as_str()) {
                        Err(e) => {
                            error!(
                            "Connection [{connection_id}] fail to resolve domain [{name}] because of error, source address: {source_address:?}, target address: {target_address:?}, client address: {agent_address:?}, error: {e:?}");
                            let domain_resolve_fail = MessagePayload {
                                source_address: source_address.clone(),
                                target_address: target_address.clone(),
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::DomainResolveFail),
                                data: None,
                            };
                            MessageFramedWriter::write(WriteMessageFramedRequest {
                                message_framed_write,
                                message_payloads: Some(vec![domain_resolve_fail]),
                                payload_encryption_type,
                                user_token: user_token.as_str(),
                                ref_id: Some(message_id.as_str()),
                                connection_id: Some(connection_id),
                            }).await.map_err(|err| anyhow!(err.source))?;
                            Err(anyhow!( "Connection [{connection_id}] fail to resolve domain [{name}]  because of error, source address: {source_address:?}, target address: {target_address:?}, client address: {agent_address:?}, error: {e:#?}"))
                        }
                        Ok(ip_addresses) => {
                            let mut addresses = Vec::new();
                            ip_addresses.iter().for_each(|addr| {
                                if let IpAddr::V4(v4_addr)=addr{
                                    let ip_bytes = v4_addr.octets();
                                    addresses.push(ip_bytes);
                                    return;
                                }
                                warn!("Connection [{connection_id}] resolve domain [{name}]  to IPV6 address: {addr}");
                            });
                            let domain_resolve_response = DomainResolveResponse {
                                id,
                                name,
                                addresses,
                            };
                            let domain_resolve_response_bytes = serde_json::to_vec(&domain_resolve_response)?;
                            let domain_resolve_success = MessagePayload {
                                source_address: source_address.clone(),
                                target_address: target_address.clone(),
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::DomainResolveSuccess),
                                data: Some(domain_resolve_response_bytes.into()),
                            };
                            let WriteMessageFramedResult { message_framed_write } = MessageFramedWriter::write(WriteMessageFramedRequest {
                                message_framed_write,
                                message_payloads: Some(vec![domain_resolve_success]),
                                payload_encryption_type: payload_encryption_type.clone(),
                                user_token: user_token.as_str(),
                                ref_id: Some(message_id.as_str()),
                                connection_id: Some(connection_id),
                            }).await.map_err(|WriteMessageFramedError {
                                source,
                                ..
                            }| {
                                error!("Connection [{connection_id}] fail to write domain resolve success to agent because of error, source address: {source_address:?}, target address: {target_address:?}, client address: {agent_address:?}, error:{source:?}");
                                anyhow!(source)
                            })?;
                            Ok((
                                message_framed_write,
                                message_framed_read
                            ))
                        }
                    };
                }.await?;
                        Ok(this)
                    },
                    Ok(MessagePayload {
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate),
                        ..
                    }) => Ok(this),
                    Ok(MessagePayload {
                        source_address,
                        target_address,
                        payload_type,
                        data,
                    }) => {
                        error!("Fail to read agent connection because of invalid payload type do not match current status, message id: {id}, message reference id: {ref_id:?}, client connection id: {connection_id:?}, user token: {user_token}");
                        *this.status = TcpTunnelStatus::Closed;
                        this.agent_connection.close().await?;
                        return Err(anyhow!(PpaassError::CodecError));
                    },
                }
            },
        }
    }

    async fn on_target_tcp_connected(mut this: TcpTunnelProject<'_, T, F>) -> Result<TcpTunnelProject<'_, T, F>> {
        Ok(this)
    }
    async fn on_target_tcp_disconnected(mut this: TcpTunnelProject<'_, T, F>) -> Result<TcpTunnelProject<'_, T, F>> {
        Ok(this)
    }
    pub async fn exec(self: Pin<&mut Self>) -> Result<()> {
        let mut this = self.project();
        loop {
            match this.status {
                TcpTunnelStatus::AgentTcpConnected => {
                    debug!("Connection in AgentConnected status.");
                    this = TcpTunnel::on_agent_tcp_connected(this).await?
                },
                TcpTunnelStatus::TargetTcpConnected => {
                    debug!("Connection in TargetConnected status.");
                    this = TcpTunnel::on_target_tcp_connected(this).await?
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
