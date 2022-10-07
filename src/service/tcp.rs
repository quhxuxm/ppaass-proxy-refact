use std::task::{Context, Poll};

use futures::future::BoxFuture;
use ppaass_common::{NetAddress, PpaassError};
use tokio::net::TcpStream;
use tower::Service;

use crate::tunnel::TcpTunnel;

use super::ProxyRsaCryptoFetcher;

pub(crate) mod connect;
pub(crate) mod relay;

pub(crate) struct TcpServiceRequest {
    pub(crate) agent_message_id: String,
    pub(crate) agent_message_reference_id: String,
    pub(crate) agent_message_connection_id: String,
    pub(crate) agent_message_source_address: NetAddress,
    pub(crate) agent_message_target_address: NetAddress,
    pub(crate) agent_message_payload_data: Vec<u8>,
    pub(crate) agent_tcp_tunnel: TcpTunnel<TcpStream, ProxyRsaCryptoFetcher>,
    pub(crate) user_token: String,
}

pub(crate) struct TcpServiceResponse;

pub(crate) struct TcpService;

impl TcpService {
    pub(crate) fn new() -> Self {
        TcpService {}
    }
}

impl Service<TcpServiceRequest> for TcpService {
    type Response = Result<TcpServiceResponse, Self::Error>;

    type Error = PpaassError;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: TcpServiceRequest) -> Self::Future {
        todo!()
    }
}
