use std::net::SocketAddr;

use anyhow::anyhow;
use anyhow::Result;
use futures::SinkExt;
use ppaass_common::{
    generate_uuid, MessageFramedRead, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest,
    PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, PpaassError, ProxyMessagePayloadTypeValue, RsaCryptoFetcher,
    TcpConnectRequest, TcpConnectResult, TcpConnector, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};
use tokio::net::TcpStream;
use tracing::{debug, error};

use crate::config::{ProxyConfig, DEFAULT_TARGET_STREAM_SO_LINGER};

pub(crate) struct TcpConnectFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_id: &'a str,
    pub user_token: &'a str,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub agent_address: SocketAddr,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
}

pub(crate) struct TcpConnectFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub target_stream: TcpStream,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
    pub message_id: String,
}

#[allow(unused)]
pub(crate) struct TcpConnectFlowError {
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub agent_address: SocketAddr,
    pub source: anyhow::Error,
}

pub(crate) struct TcpConnectFlow;

impl TcpConnectFlow {
    pub async fn exec<T>(
        TcpConnectFlowRequest {
            connection_id,
            message_id,
            user_token,
            source_address,
            target_address,
            message_framed_write,
            message_framed_read,
            agent_address,
            ..
        }: TcpConnectFlowRequest<'_, T>,
        configuration: &ProxyConfig,
    ) -> Result<TcpConnectFlowResult<T>, TcpConnectFlowError>
    where
        T: RsaCryptoFetcher,
    {
        let target_stream_so_linger = configuration.target_stream_so_linger().unwrap_or(DEFAULT_TARGET_STREAM_SO_LINGER);
        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token,
        })
        .await
        .map_err(|e| TcpConnectFlowError {
            connection_id: connection_id.to_owned(),
            message_id: message_id.to_owned(),
            user_token: user_token.to_owned(),
            source_address: source_address.clone(),
            target_address: target_address.clone(),
            agent_address,
            source: anyhow!(e),
        })?;
        let connect_addresses = target_address.clone().try_into().map_err(|e: PpaassError| TcpConnectFlowError {
            connection_id: connection_id.to_owned(),
            message_id: message_id.to_owned(),
            user_token: user_token.to_owned(),
            source_address: source_address.clone(),
            target_address: target_address.clone(),
            agent_address,
            source: anyhow!(e),
        })?;
        let TcpConnectResult {
            connected_stream: target_stream,
        } = match TcpConnector::connect(TcpConnectRequest {
            connection_id: connection_id.to_owned(),
            connect_addresses,
            connected_stream_so_linger: target_stream_so_linger,
        })
        .await
        {
            Ok(v) => v,
            Err(e) => {
                error!("Connection [{connection_id}] fail connect to target: [{target_address:?}] because of error: {e:?}");

                let connect_fail_payload = MessagePayload {
                    source_address: Some(source_address.clone()),
                    target_address: Some(target_address.clone()),
                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                    data: None,
                };
                let WriteMessageFramedResult { mut message_framed_write, .. } = MessageFramedWriter::write(WriteMessageFramedRequest {
                    message_framed_write,
                    message_payloads: Some(vec![connect_fail_payload]),
                    payload_encryption_type,
                    user_token,
                    ref_id: Some(message_id),
                    connection_id: Some(connection_id),
                })
                .await
                .map_err(|WriteMessageFramedError { source, .. }| {
                    error!("Connection [{connection_id}] fail to write connect fail result to agent because of error: {source:?}");
                    TcpConnectFlowError {
                        connection_id: connection_id.to_owned(),
                        message_id: message_id.to_owned(),
                        user_token: user_token.to_owned(),
                        source_address: source_address.clone(),
                        target_address: target_address.clone(),
                        agent_address,
                        source: anyhow!(source),
                    }
                })?;
                message_framed_write.close().await.map_err(|e| TcpConnectFlowError {
                    connection_id: connection_id.to_owned(),
                    message_id: message_id.to_owned(),
                    user_token: user_token.to_owned(),
                    source_address: source_address.clone(),
                    target_address: target_address.clone(),
                    agent_address,
                    source: anyhow!(e),
                })?;
                return Err(TcpConnectFlowError {
                    connection_id: connection_id.to_owned(),
                    message_id: message_id.to_owned(),
                    user_token: user_token.to_owned(),
                    source_address,
                    target_address: target_address.clone(),
                    agent_address,
                    source: anyhow!(e),
                });
            },
        };
        debug!(
            "Connection [{}] agent address: {}, success connect to target {:#?}",
            connection_id, agent_address, target_address
        );

        let connect_success_payload = MessagePayload {
            source_address: Some(source_address.clone()),
            target_address: Some(target_address.clone()),
            payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
            data: None,
        };
        let WriteMessageFramedResult { message_framed_write } = MessageFramedWriter::write(WriteMessageFramedRequest {
            message_framed_write,
            message_payloads: Some(vec![connect_success_payload]),
            payload_encryption_type,
            user_token,
            ref_id: Some(message_id),
            connection_id: Some(connection_id),
        })
        .await
        .map_err(|WriteMessageFramedError { source, .. }| {
            error!("Connection [{connection_id}] fail to write connect success result to agent because of error: {source:#?}");
            TcpConnectFlowError {
                connection_id: connection_id.to_owned(),
                message_id: message_id.to_owned(),
                user_token: user_token.to_owned(),
                source_address: source_address.clone(),
                target_address: target_address.clone(),
                agent_address,
                source: anyhow!(source),
            }
        })?;
        Ok(TcpConnectFlowResult {
            target_stream,
            message_framed_read,
            message_framed_write,
            source_address,
            target_address,
            user_token: user_token.to_owned(),
            message_id: message_id.to_owned(),
        })
    }
}
