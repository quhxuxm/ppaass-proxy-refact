use anyhow::Result;
use server::ProxyServer;
use tokio::{net::TcpListener, runtime::Builder};
use tracing::info;

mod server;

pub fn main() -> Result<()> {
    // let (command_send, command_recv) = tokio::sync::mpsc::channel(1024);
    // let proxy_server = Box::new(ProxyServer::new(command_recv, 80, false)?);
    // // let mut proxy_server = Box::pin(proxy_server);
    // let proxy_server = Box::pin(Box::leak(proxy_server));
    // let proxy_server_ref = proxy_server.as_mut();
    // proxy_server_ref.run();
    Ok(())
}
