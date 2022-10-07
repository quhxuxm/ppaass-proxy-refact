use std::task::{Context, Poll};

use futures::future::{ready, BoxFuture};
use ppaass_common::{AgentMessagePayloadTypeValue, Message, MessagePayload, PayloadType, PpaassError};
use tokio::net::TcpStream;
use tower::Service;
use tower::ServiceExt;

use crate::tunnel::TcpTunnel;

use super::{
    tcp::{TcpService, TcpServiceRequest},
    ProxyRsaCryptoFetcher,
};

pub(crate) struct TunnelDispatcherService;

pub(crate) struct TunnelDispatcherRequest {
    agent_message: Message,
    agent_tcp_tunnel: TcpTunnel<TcpStream, ProxyRsaCryptoFetcher>,
}

pub(crate) struct TunnelDispatcherResponse;

impl Service<TunnelDispatcherRequest> for TunnelDispatcherService {
    type Response = TunnelDispatcherResponse;

    type Error = PpaassError;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TunnelDispatcherRequest) -> Self::Future {
        let TunnelDispatcherRequest {
            agent_message:
                Message {
                    id: agent_message_id,
                    ref_id: agent_message_reference_id,
                    connection_id: agent_message_connection_id,
                    user_token,
                    payload_encryption: agent_message_payload_encryption,
                    payload: agent_message_payload_bytes,
                },
            agent_tcp_tunnel,
        } = req;
        let agent_message_payload_bytes = match agent_message_payload_bytes {
            None => return Box::pin(ready(Err(PpaassError::CodecError))),
            Some(v) => v,
        };
        let MessagePayload {
            source_address: agent_message_source_address,
            target_address: agent_message_target_address,
            payload_type: agent_message_payload_type,
            data: agent_message_payload_data,
        } = match agent_message_payload_bytes.try_into() {
            Err(e) => {
                return Box::pin(ready(Err(e)));
            },
            Ok(v) => v,
        };
        match agent_message_payload_type {
            PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect) => {
                async move {
                    let mut tcp_service = TcpService::new();
                    tcp_service.ready().await?.call(TcpServiceRequest {
                        agent_message_id,
                        agent_message_reference_id,
                        agent_message_connection_id,
                        agent_message_source_address,
                        agent_message_target_address,
                        agent_message_payload_data,
                        agent_tcp_tunnel,
                        user_token,
                    });
                }
            },
            PayloadType::AgentPayload(AgentMessagePayloadTypeValue::DomainResolve) => todo!(),
            PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat) => todo!(),
            PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate) => todo!(),
            _ => return Box::pin(ready(Err(PpaassError::CodecError))),
        };
        Box::pin(async { Ok(TunnelDispatcherResponse) })
    }
}
