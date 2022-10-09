use anyhow::Result;
use futures::Future;
use pin_project::pin_project;
use std::{
    pin::Pin,
    task::{Context, Poll},
    thread::JoinHandle,
};
use tokio::{
    net::TcpListener,
    runtime::{Builder, Runtime},
    sync::mpsc::{Receiver, Sender},
};
use tracing::{error, info};

#[derive(Debug)]
pub(super) enum ProxyCommand {
    Stop,
}

#[pin_project]
pub(super) struct ProxyServer {
    runtime: Runtime,
    bind_port: u16,
    use_ipv6: bool,
    command_receiver: Receiver<ProxyCommand>,
    command_sender: Sender<ProxyCommand>,
    manager: Option<JoinHandle<()>>,
}

impl ProxyServer {
    pub(super) fn new(bind_port: u16, use_ipv6: bool) -> Result<Self> {
        let mut runtime_builder = Builder::new_multi_thread();
        runtime_builder.enable_all().worker_threads(256);
        let runtime = runtime_builder.build()?;
        let (command_sender, command_receiver) = tokio::sync::mpsc::channel(1024);
        Ok(Self {
            runtime,
            bind_port,
            use_ipv6,
            command_receiver,
            command_sender,
            manager: None,
        })
    }

    pub(super) fn send_command(self: Pin<&'static mut Self>, command: ProxyCommand) {
        let this = self.project();
        this.runtime.spawn(async {
            if let Err(e) = this.command_sender.send(command).await {
                error!("Fail to send proxy command because of error: {e:?}");
            }
        });
    }

    pub(super) fn run(self: Pin<&'static mut Self>) -> Result<()> {
        let this = self.project();
        this.runtime.spawn(async {
            loop {
                match this.command_receiver.recv().await {
                    None => {
                        return;
                    },
                    Some(command) => match command {
                        ProxyCommand::Stop => {
                            this.runtime.
                            
                        },
                    },
                }
            }
        });
        this.runtime.block_on(async {
            let binding_addr = if *this.use_ipv6 {
                format!("::1:{}", this.bind_port)
            } else {
                format!("0.0.0.0:{}", this.bind_port)
            };
            let tcp_listener = TcpListener::bind(binding_addr.clone()).await?;
            info!("Proxy server started, bind on address: {binding_addr}");
            loop {
                let (agent_tcp_stream, agent_tcp_socket_address) = tcp_listener.accept().await?;
            }
            Ok(())
        })
    }
}
