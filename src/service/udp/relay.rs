use std::{fmt::Debug, net::ToSocketAddrs, sync::Arc};

use anyhow::Result;

use ppaass_common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};
use pretty_hex;
use tokio::net::{TcpStream, UdpSocket};
use tracing::{error, info};

use pretty_hex::*;

use crate::config::{ProxyConfig, DEFAULT_UDP_RELAY_TIMEOUT_SECONDS};
const SIZE_64KB: usize = 65535;

#[allow(unused)]
#[derive(Debug)]
pub(crate) struct UdpRelayFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_id: &'a str,
    pub user_token: &'a str,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub udp_binding_socket: Arc<UdpSocket>,
}

#[derive(Debug)]
pub(crate) struct UdpRelayFlowResult {
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
}

pub(crate) struct UdpRelayFlow;

impl UdpRelayFlow {
    pub async fn exec<T>(
        UdpRelayFlowRequest {
            connection_id,
            message_id,
            user_token,
            mut message_framed_read,
            mut message_framed_write,
            udp_binding_socket,
            ..
        }: UdpRelayFlowRequest<'_, T>,
        _configuration: &ProxyConfig,
    ) -> Result<UdpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let udp_relay_timeout = _configuration.udp_relay_timeout().unwrap_or(DEFAULT_UDP_RELAY_TIMEOUT_SECONDS);
        let (udp_response_sender, mut udp_response_receiver) = tokio::sync::mpsc::channel::<MessagePayload>(1024);
        let connection_id = connection_id.to_owned();
        let message_id = message_id.to_owned();
        let user_token = user_token.to_owned();
        {
            let udp_response_sender = udp_response_sender.clone();
            let connection_id = connection_id.clone();
            let message_id = message_id.clone();
            let user_token = user_token.clone();
            tokio::spawn(async move {
                loop {
                    match MessageFramedReader::read(ReadMessageFramedRequest {
                        connection_id: &connection_id,
                        message_framed_read,
                        timeout: None,
                    })
                    .await
                    {
                        Err(ReadMessageFramedError { source, .. }) => {
                            error!("Udp relay has a error when read from agent, connection id: [{connection_id}] , error: {source:?}.");
                            return;
                        },
                        Ok(ReadMessageFramedResult { content: None, .. }) => {
                            info!("Udp relay nothing to read, connection id: [{connection_id}] , message id:{message_id}, user token:{user_token}",);
                            return;
                        },
                        Ok(ReadMessageFramedResult {
                            message_framed_read: _message_framed_read,
                            content:
                                Some(ReadMessageFramedResultContent {
                                    message_payload:
                                        Some(MessagePayload {
                                            source_address,
                                            target_address: Some(target_address),
                                            payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpData),
                                            data: Some(data),
                                        }),
                                    ..
                                }),
                        }) => {
                            let udp_target_addresses = match target_address.clone().to_socket_addrs() {
                                Ok(v) => v,
                                Err(e) => {
                                    error!("Udp relay fail to convert target address, connection id: [{connection_id}], error: {e:?}");
                                    return;
                                },
                            };

                            if let Err(e) = udp_binding_socket.connect(udp_target_addresses.collect::<Vec<_>>().as_slice()).await {
                                error!("Udp relay fail connect to target, target: [{target_address:?}], connection id: [{connection_id}], error:{e:?}");
                                return;
                            };
                            info!(
                                "Udp relay begin to send udp data from agent to target: [{target_address:?}], connection id: [{connection_id}], data:\n{}\n",
                                pretty_hex::pretty_hex(&data)
                            );
                            if let Err(e) = udp_binding_socket.send(&data).await {
                                error!(
                                    "Udp relay fail to send udp packet to target, connection id:[{connection_id}], target: [{target_address:?}], error:{e:?}"
                                );
                                return;
                            };
                            let udp_binding_socket_for_receive = Arc::clone(&udp_binding_socket);
                            let udp_response_sender = udp_response_sender.clone();
                            let connection_id = connection_id.clone();
                            tokio::spawn(async move {
                                let mut receive_buffer = [0u8; SIZE_64KB];
                                let received_data_size = match tokio::time::timeout(
                                    std::time::Duration::from_secs(udp_relay_timeout),
                                    udp_binding_socket_for_receive.recv(&mut receive_buffer),
                                )
                                .await
                                {
                                    Err(_elapsed) => {
                                        error!("Timeout( {udp_relay_timeout} seconds) to receive udp packet from target, connection id: [{connection_id}], target: [{target_address:?}]");
                                        drop(udp_response_sender);
                                        return;
                                    },
                                    Ok(Err(e)) => {
                                        error!("Udp relay fail to receive udp packet from target, connection id: [{connection_id}], target: [{target_address:?}], error:{e:?}");
                                        drop(udp_response_sender);
                                        return;
                                    },
                                    Ok(Ok(v)) => v,
                                };
                                let received_data = &receive_buffer[0..received_data_size];
                                info!(
                                    "Udp relay receive data from target, connection id:[{connection_id}], target:[{target_address:?}], data:\n{}\n",
                                    pretty_hex(&received_data)
                                );
                                let message_payload = MessagePayload {
                                    data: Some(received_data.to_vec()),
                                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpDataSuccess),
                                    source_address: source_address.clone(),
                                    target_address: Some(target_address.clone()),
                                };

                                if let Err(e) = udp_response_sender.send(message_payload).await {
                                    error!("Fail to receive udp data because of error: {e:?}")
                                };
                                drop(udp_response_sender);
                            });
                            message_framed_read = _message_framed_read;
                        },
                        Ok(unknown_content) => {
                            error!("Udp relay fail, invalid payload when read from agent, connection id: [{connection_id}], invalid payload:\n{unknown_content:#?}\n");
                            return;
                        },
                    };
                }
            });
        }
        {
            let connection_id = connection_id.clone();
            let message_id = message_id.clone();
            let user_token = user_token.clone();
            tokio::spawn(async move {
                while let Some(message_payload) = udp_response_receiver.recv().await {
                    let target_address = message_payload.target_address.clone();
                    let PayloadEncryptionTypeSelectResult {
                        user_token,
                        payload_encryption_type,
                        ..
                    } = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                        encryption_token: generate_uuid().into(),
                        user_token: &user_token,
                    })
                    .await
                    {
                        Err(e) => {
                            error!(
                            "Udp relay fail to select payload encryption, connection id: [{connection_id}], target address: [{target_address:?}], error:{e:?}"
                        );
                            continue;
                        },
                        Ok(v) => v,
                    };
                    message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                        connection_id: Some(connection_id.as_str()),
                        message_framed_write,
                        message_payloads: Some(vec![message_payload]),
                        payload_encryption_type: payload_encryption_type.clone(),
                        ref_id: Some(&message_id),
                        user_token: user_token.as_str(),
                    })
                    .await
                    {
                        Err(WriteMessageFramedError { message_framed_write, source }) => {
                            error!(
                            "Udp relay fail to write data to target, connection id: [{connection_id}], target address: [{target_address:?}], error:{source:?}"
                        );
                            message_framed_write
                        },
                        Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
                    };
                }
                udp_response_receiver.close();
            });
        }
        drop(udp_response_sender);
        Ok(UdpRelayFlowResult {
            connection_id,
            message_id,
            user_token,
        })
    }
}
