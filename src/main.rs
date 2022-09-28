extern crate core;

use std::{io::Read, path::Path, str::FromStr, sync::mpsc::channel};

use anyhow::Result;
use clap::Parser;
use hotwatch::{Event, Hotwatch};
use tracing::{debug, error, Level};
use tracing::{info, metadata::LevelFilter};
use tracing_subscriber::Registry;
use tracing_subscriber::{fmt::Layer, prelude::__tracing_subscriber_SubscriberExt};

use config::{ProxyArguments, ProxyConfig, ProxyLogConfig};
use ppaass_common::LogTimer;
use server::ProxyServerSignal;

use crate::server::ProxyServer;

pub(crate) mod config;
mod connection;
pub(crate) mod server;
pub(crate) mod service;

fn merge_arguments_and_log_config(arguments: &ProxyArguments, log_config: &mut ProxyLogConfig) {
    if let Some(ref log_dir) = arguments.log_dir {
        log_config.set_log_dir(log_dir.to_string())
    }
    if let Some(ref log_file) = arguments.log_file {
        log_config.set_log_file(log_file.to_string())
    }
    if let Some(ref max_log_level) = arguments.max_log_level {
        log_config.set_max_log_level(max_log_level.to_string())
    }
}

fn merge_arguments_and_config(arguments: &ProxyArguments, config: &mut ProxyConfig) {
    if let Some(port) = arguments.port {
        config.set_port(port);
    }
    if let Some(compress) = arguments.compress {
        config.set_compress(compress);
    }

    if let Some(target_buffer_size) = arguments.target_buffer_size {
        config.set_target_buffer_size(target_buffer_size)
    }
    if let Some(message_framed_buffer_size) = arguments.message_framed_buffer_size {
        config.set_message_framed_buffer_size(message_framed_buffer_size)
    }
    if let Some(ref rsa_root_dir) = arguments.rsa_root_dir {
        config.set_rsa_root_dir(rsa_root_dir.to_string())
    }

    if let Some(so_backlog) = arguments.so_backlog {
        config.set_so_backlog(so_backlog)
    }
}

fn main() -> Result<()> {
    let arguments = ProxyArguments::parse();
    let mut log_configuration_file = std::fs::File::open(config::DEFAULT_PROXY_LOG_CONFIG_FILE).expect("Fail to read proxy log configuration file.");
    let mut log_configuration_file_content = String::new();
    log_configuration_file
        .read_to_string(&mut log_configuration_file_content)
        .expect("Fail to read proxy server log configuration file.");
    let mut log_configuration = toml::from_str::<ProxyLogConfig>(&log_configuration_file_content).expect("Fail to parse proxy log configuration file");
    merge_arguments_and_log_config(&arguments, &mut log_configuration);
    let log_directory = log_configuration.log_dir().as_ref().expect("No log directory given.");
    let log_file = log_configuration.log_file().as_ref().expect("No log file name given.");
    let default_log_level = &Level::ERROR.to_string();
    let log_max_level = log_configuration.max_log_level().as_ref().unwrap_or(default_log_level);
    let file_appender = tracing_appender::rolling::daily(log_directory, log_file);
    let (non_blocking, _appender_guard) = tracing_appender::non_blocking(file_appender);
    let log_level_filter = LevelFilter::from_str(log_max_level).expect("Fail to initialize log filter");
    let subscriber = Registry::default()
        .with(
            Layer::default()
                .with_level(true)
                .with_target(true)
                .with_timer(LogTimer)
                .with_thread_ids(true)
                .with_file(true)
                .with_ansi(false)
                .with_line_number(true)
                .with_writer(non_blocking),
        )
        .with(log_level_filter);
    tracing::subscriber::set_global_default(subscriber).expect("Fail to initialize tracing subscriber");
    loop {
        let (proxy_server_signal_sender, proxy_server_signal_receiver) = channel();
        let proxy_server_signal_sender_for_watch_configuration = proxy_server_signal_sender.clone();
        let proxy_server_signal_sender_for_watch_rsa = proxy_server_signal_sender.clone();
        let mut configuration_file_watch = Hotwatch::new()?;
        let configuration_file_path = arguments
            .configuration_file
            .as_ref()
            .map(|file_path| Path::new(file_path))
            .unwrap_or(Path::new(config::DEFAULT_PROXY_CONFIG_FILE));
        let configuration_file_path_for_logging = configuration_file_path.clone().to_owned();
        configuration_file_watch
            .watch(configuration_file_path, move |event| {
                info!("Event happen on watching file: {:?}", event);
                match event {
                    Event::NoticeWrite(_) => {
                        debug!("Receive write event on configuration file: {configuration_file_path_for_logging:?}");
                        return;
                    },
                    Event::Remove(_) => {
                        debug!("Receive remove event on configuration file: {configuration_file_path_for_logging:?}");
                        return;
                    },
                    event => {
                        if let Err(e) = proxy_server_signal_sender_for_watch_configuration.send(ProxyServerSignal::Shutdown) {
                            error!("Fail to notice proxy server shutdown, event: {event:?}, error: {e:#?}");
                        };
                    },
                }
            })
            .expect("Fail to start proxy server configuration file watch");
        let configuration_file_content = std::fs::read_to_string(configuration_file_path).expect("Fail to read proxy configuration file.");
        let mut configuration = toml::from_str::<ProxyConfig>(&configuration_file_content).expect("Fail to parse proxy configuration file");
        merge_arguments_and_config(&arguments, &mut configuration);
        let rsa_dir_path = configuration.rsa_root_dir().as_ref().expect("Fail to read rsa root directory.");
        let mut rsa_folder_watch = Hotwatch::new().expect("Fail to start proxy server rsa folder watch");
        rsa_folder_watch
            .watch(rsa_dir_path, move |event| {
                info!("Event happen on watching dir:{:?}", event);
                if let Err(e) = proxy_server_signal_sender_for_watch_rsa.send(ProxyServerSignal::Shutdown) {
                    error!("Fail to notice proxy server shutdown, event: {event:?}, error: {:#?}", e);
                };
            })
            .expect("Fail to start proxy server rsa folder watch");
        proxy_server_signal_sender
            .send(ProxyServerSignal::Startup { configuration })
            .expect("Fail to send startup single to proxy server");
        let proxy_server = ProxyServer::new(proxy_server_signal_receiver)?;
        let proxy_server_guard = proxy_server.run()?;
        proxy_server_guard.join().expect("Error happen in proxy server.");
    }
}
