use std::net::IpAddr;
use std::{
    fmt::Debug,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use anyhow::anyhow;
use anyhow::Result;
use dns_lookup;
use ppaass_common::{
    generate_uuid, AgentMessagePayloadTypeValue, DomainResolveRequest, DomainResolveResponse, MessageFramedRead, MessageFramedReader, MessageFramedWrite,
    MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector,
    PayloadType, ProxyMessagePayloadTypeValue, ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent,
    RsaCryptoFetcher, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error, warn};

use crate::{
    config::{self, ProxyConfig},
    service::{tcp::connect::TcpConnectFlowError, udp::associate::UdpAssociateFlowError},
};

use super::{
    tcp::connect::{TcpConnectFlow, TcpConnectFlowRequest, TcpConnectFlowResult},
    udp::associate::{UdpAssociateFlow, UdpAssociateFlowRequest, UdpAssociateFlowResult},
};

#[derive(Debug)]
pub(crate) struct InitFlowRequest<'a, T, TcpStream>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub agent_address: SocketAddr,
}

#[allow(unused)]
pub(crate) enum InitFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    Heartbeat {
        message_framed_read: MessageFramedRead<T, TcpStream>,
        message_framed_write: MessageFramedWrite<T, TcpStream>,
    },
    DomainResolve {
        message_framed_read: MessageFramedRead<T, TcpStream>,
        message_framed_write: MessageFramedWrite<T, TcpStream>,
    },
    Tcp {
        target_stream: TcpStream,
        message_framed_read: MessageFramedRead<T, TcpStream>,
        message_framed_write: MessageFramedWrite<T, TcpStream>,
        message_id: String,
        source_address: NetAddress,
        target_address: NetAddress,
        user_token: String,
    },
    Udp {
        message_framed_read: MessageFramedRead<T, TcpStream>,
        message_framed_write: MessageFramedWrite<T, TcpStream>,
        message_id: String,
        source_address: Option<NetAddress>,
        user_token: String,
        udp_binding_socket: Arc<UdpSocket>,
    },
}

#[derive(Clone, Default)]
pub(crate) struct InitializeFlow;

impl InitializeFlow {
    pub async fn exec<T>(
        InitFlowRequest {
            connection_id,
            message_framed_read,
            message_framed_write,
            agent_address,
        }: InitFlowRequest<'_, T, TcpStream>,
        configuration: &ProxyConfig,
    ) -> Result<InitFlowResult<T>>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let read_timeout = configuration
            .agent_connection_read_timeout()
            .unwrap_or(config::DEFAULT_AGENT_CONNECTION_READ_TIMEOUT);
        match MessageFramedReader::read(ReadMessageFramedRequest {
            connection_id: connection_id.clone(),
            message_framed_read,
            timeout: Some(read_timeout),
        })
        .await
        .map_err(|ReadMessageFramedError { source, .. }| {
            error!("Connection [{connection_id}] handle agent connection fail because of error: {source}.");
            anyhow!(source)
        })? {
            ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        user_token,
                        message_id,
                        message_payload:
                            Some(MessagePayload {
                                source_address,
                                target_address,
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat),
                                ..
                            }),
                        ..
                    }),
                ..
            } => {
                let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } =
                    PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                        encryption_token: generate_uuid().into(),
                        user_token: user_token.as_str(),
                    })
                    .await?;
                let heartbeat_success = MessagePayload {
                    source_address: None,
                    target_address: None,
                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::HeartbeatSuccess),
                    data: None,
                };
                let WriteMessageFramedResult { message_framed_write } = MessageFramedWriter::write(WriteMessageFramedRequest {
                    message_framed_write,
                    message_payloads: Some(vec![heartbeat_success]),
                    payload_encryption_type,
                    user_token: user_token.as_str(),
                    ref_id: Some(message_id.as_str()),
                    connection_id: Some(connection_id),
                })
                    .await.map_err(|WriteMessageFramedError { source, .. }| {
                    error!("Connection [{}] fail to write heartbeat success to agent because of error, source address: {:?}, target address: {:?}, client address: {:?}", connection_id, source_address, target_address, agent_address);
                    return anyhow!(source);
                })?;
                return Ok(InitFlowResult::Heartbeat {
                    message_framed_write,
                    message_framed_read,
                });
            },
            ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        user_token,
                        message_id,
                        message_payload:
                            Some(MessagePayload {
                                source_address,
                                target_address,
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::DomainResolve),
                                data,
                            }),
                        ..
                    }),
                ..
            } => {
                let data = data.ok_or(anyhow!(
                    "Connection [{connection_id}] fail to do domain resolve because of no data, source address: {source_address:?}, target address: {target_address:?}, client address: {agent_address:?}",
                ))?;
                let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } =
                    PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                        encryption_token: generate_uuid().into(),
                        user_token: user_token.as_str(),
                    })
                    .await?;
                let domain_resolve_request: DomainResolveRequest = serde_json::from_slice(data.as_ref())?;
                let target_domain_name = domain_resolve_request.name.as_str();
                let target_domain_name = if target_domain_name.ends_with(".") {
                    let result = &target_domain_name[0..target_domain_name.len() - 1];
                    debug!("Resolving domain name(end with .): {result}");
                    result.to_string()
                } else {
                    debug!("Resolving domain name(not end with .): {target_domain_name}");
                    target_domain_name.to_string()
                };
                let (message_framed_write, message_framed_read) = async move {
                    return match dns_lookup::lookup_host(target_domain_name.as_str()) {
                        Err(e) => {
                            error!(
                            "Connection [{connection_id}] fail to resolve domain [{target_domain_name}] because of error, source address: {source_address:?}, target address: {target_address:?}, client address: {agent_address:?}, error: {e:#?}");
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
                            Err(anyhow!( "Connection [{connection_id}] fail to resolve domain [{target_domain_name}]  because of error, source address: {source_address:?}, target address: {target_address:?}, client address: {agent_address:?}, error: {e:#?}"))
                        }
                        Ok(ip_addresses) => {
                            let mut addresses = Vec::new();
                            ip_addresses.iter().for_each(|addr| {
                                if let IpAddr::V4(v4_addr)=addr{
                                    let ip_bytes = v4_addr.octets();
                                    addresses.push(ip_bytes);
                                    return;
                                }
                                warn!("Connection [{connection_id}] resolve domain [{target_domain_name}]  to IPV6 address: {addr}");
                            });
                            let domain_resolve_response = DomainResolveResponse {
                                id: domain_resolve_request.id,
                                name: domain_resolve_request.name,
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
                                error!("Connection [{}] fail to write domain resolve success to agent because of error, source address: {:?}, target address: {:?}, client address: {:?}", connection_id, source_address, target_address, agent_address);
                                anyhow!(source)
                            })?;
                            Ok((
                                message_framed_write,
                                message_framed_read
                            ))
                        }
                    };
                }.await?;
                return Ok(InitFlowResult::DomainResolve {
                    message_framed_write,
                    message_framed_read,
                });
            },
            ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        user_token,
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                                target_address: Some(target_address),
                                source_address: Some(source_address),
                                ..
                            }),
                        ..
                    }),
                ..
            } => {
                debug!(
                    "Connection [{}] begin tcp connect, source address: {:?}, target address: {:?}, client address: {:?}",
                    connection_id, source_address, target_address, agent_address
                );
                let TcpConnectFlowResult {
                    target_stream,
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    target_address,
                    user_token,
                    message_id,
                    ..
                } = TcpConnectFlow::exec(
                    TcpConnectFlowRequest {
                        connection_id,
                        message_id: message_id.as_str(),
                        message_framed_read,
                        message_framed_write,
                        agent_address,
                        source_address,
                        target_address,
                        user_token: user_token.as_str(),
                    },
                    configuration,
                )
                .await
                .map_err(|TcpConnectFlowError { connection_id, source, .. }| {
                    error!("Connection [{connection_id}] handle agent connection fail to do tcp connect because of error: {source:#?}.");
                    anyhow!(source)
                })?;
                debug!(
                    "Connection [{connection_id}] complete tcp connect, source address: {source_address:?}, target address: {target_address:?}, client address: {agent_address:?}"
                );
                Ok(InitFlowResult::Tcp {
                    message_framed_write,
                    message_framed_read,
                    target_stream,
                    message_id,
                    source_address,
                    target_address,
                    user_token,
                })
            },
            ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        user_token,
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate),
                                target_address: None,
                                source_address,
                                ..
                            }),
                        ..
                    }),
                ..
            } => {
                debug!("Connection [{connection_id}] begin udp associate, source address: {source_address:?}, agent address: {agent_address:?}");
                let UdpAssociateFlowResult {
                    connection_id,
                    message_id,
                    user_token,
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    udp_binding_socket,
                } = UdpAssociateFlow::exec(
                    UdpAssociateFlowRequest {
                        message_framed_read,
                        message_framed_write,
                        agent_address,
                        connection_id,
                        message_id: message_id.as_str(),
                        source_address,
                        user_token: user_token.as_str(),
                    },
                    configuration,
                )
                .await
                .map_err(|UdpAssociateFlowError { connection_id, source, .. }| {
                    error!("Connection [{connection_id}] handle agent connection fail to do tcp connect because of error: {source:#?}.");
                    anyhow!(source)
                })?;
                debug!("Connection [{connection_id}] complete udp associate, source address: {source_address:?}, agent address: {agent_address:?}");
                Ok(InitFlowResult::Udp {
                    message_framed_write,
                    message_framed_read,
                    message_id,
                    source_address,
                    user_token,
                    udp_binding_socket,
                })
            },
            other => {
                error!("Connection [{connection_id}] handle agent connection fail because of invalid message content:\n{other:#?}\n.");
                Err(anyhow!(
                    "Connection [{connection_id}] handle agent connection fail because of invalid message content."
                ))
            },
        }
    }
}
