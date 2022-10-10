use anyhow::Result;
use futures::Future;
use pin_project::pin_project;
use std::{
    pin::Pin,
    sync::mpsc::{channel, Receiver, SendError, Sender},
    task::{Context, Poll},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    runtime::{Builder, Runtime},
};
use tracing::{error, info};

#[derive(Debug)]
pub(crate) enum Command {
    StartServer,
    StopServer,
}

#[derive(Debug)]
enum ProxyServerStatus {
    New,
    Running(Runtime),
    Stopped,
}
pub(crate) struct ProxyServer {
    status: ProxyServerStatus,
    bind_port: u16,
    use_ipv6: bool,
    command_receiver: Receiver<Command>,
    command_sender: Sender<Command>,
    thread_number: usize,
    _guard: Option<JoinHandle<()>>,
}

impl Drop for ProxyServer {
    fn drop(&mut self) {
        self._guard = None;
    }
}

impl ProxyServer {
    pub(crate) fn new(bind_port: u16, use_ipv6: bool, thread_number: usize) -> Result<Self> {
        let (command_sender, command_receiver) = channel::<Command>();
        Ok(Self {
            status: ProxyServerStatus::New,
            bind_port,
            use_ipv6,
            command_receiver,
            command_sender,
            _guard: None,
            thread_number,
        })
    }

    pub(crate) fn send_command(&self, command: Command) -> Result<(), SendError<Command>> {
        self.command_sender.send(command)
    }

    pub(crate) fn run(mut self) -> Result<()> {
        let command_receiver = self.command_receiver;
        let _guard = thread::spawn(move || loop {
            match command_receiver.recv() {
                Err(e) => {
                    error!("Fail to receive proxy server command because of error: {e:?}");
                    return;
                },
                Ok(command) => {
                    info!("Receive proxy server command: {command:?}");
                    match command {
                        Command::StartServer => match self.status {
                            status @ (ProxyServerStatus::New | ProxyServerStatus::Stopped) => {
                                info!("Proxy server currently in {status:?} status, going to create async runtime.");
                                let mut runtime_builder = Builder::new_multi_thread();
                                runtime_builder.enable_all().worker_threads(self.thread_number);
                                let runtime = match runtime_builder.build() {
                                    Err(e) => {
                                        error!("Fail to create proxy server async runtime because of error: {e:?}");
                                        return;
                                    },
                                    Ok(v) => v,
                                };
                                runtime.block_on(async {
                                    let binding_addr = if self.use_ipv6 {
                                        format!("::1:{}", self.bind_port)
                                    } else {
                                        format!("0.0.0.0:{}", self.bind_port)
                                    };
                                    let tcp_listener = match TcpListener::bind(binding_addr.clone()).await {
                                        Err(e) => {
                                            error!("Fail to bind proxy server on address [{binding_addr}] because of error: {e:?}");
                                            return;
                                        },
                                        Ok(v) => v,
                                    };
                                    info!("Proxy server started, success bind on address: {binding_addr}");
                                    loop {
                                        let (agent_tcp_stream, agent_tcp_socket_address) = match tcp_listener.accept().await {
                                            Err(e) => return,
                                            Ok(v) => v,
                                        };
                                    }
                                });
                                self.status = ProxyServerStatus::Running(runtime);
                            },
                            ProxyServerStatus::Running(_) => {
                                error!("Ignore start command when proxy server is running.");
                                continue;
                            },
                        },
                        Command::StopServer => match self.status {
                            ProxyServerStatus::New => return,
                            ProxyServerStatus::Running(runtime) => {
                                runtime.shutdown_timeout(Duration::from_secs(30));
                                self.status = ProxyServerStatus::Stopped;
                            },
                            ProxyServerStatus::Stopped => return,
                        },
                    }
                },
            };
        });
        self._guard = Some(_guard);
        Ok(())
    }
}
