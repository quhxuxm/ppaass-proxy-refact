use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll},
};
use std::{fmt::Debug, sync::Arc};
use std::{fs, path::Path};

use anyhow::Result;
use futures::{Sink, Stream};
use pin_project::pin_project;
use ppaass_common::{generate_uuid, Message, MessageCodec, PpaassError, RsaCrypto, RsaCryptoFetcher};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::ToSocketAddrs,
};
use tokio_util::codec::Framed;
use tracing::error;

use crate::config::ProxyConfig;

mod init;
mod tcp;
mod udp;

#[derive(Debug)]
pub(crate) struct ProxyRsaCryptoFetcher {
    cache: HashMap<String, RsaCrypto>,
}

impl ProxyRsaCryptoFetcher {
    pub fn new(configuration: &ProxyConfig) -> Result<Self> {
        let mut result = Self { cache: HashMap::new() };
        let rsa_dir_path = configuration.rsa_root_dir().as_ref().expect("Fail to read rsa root directory.");
        let rsa_dir = fs::read_dir(rsa_dir_path)?;
        rsa_dir.for_each(|entry| {
            let entry = match entry {
                Err(e) => {
                    error!("Fail to read {} directory because of error: {:#?}", rsa_dir_path, e);
                    return;
                },
                Ok(v) => v,
            };
            let user_token = entry.file_name();
            let user_token = user_token.to_str();
            let user_token = match user_token {
                None => {
                    error!("Fail to read {}{:?} directory because of user token not exist", rsa_dir_path, entry.file_name());
                    return;
                },
                Some(v) => v,
            };
            let public_key = match fs::read_to_string(Path::new(format!("{}{}/AgentPublicKey.pem", rsa_dir_path, user_token).as_str())) {
                Err(e) => {
                    error!("Fail to read {}{}/AgentPublicKey.pem because of error: {:#?}", rsa_dir_path, user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            let private_key = match fs::read_to_string(Path::new(format!("{}{}/ProxyPrivateKey.pem", rsa_dir_path, user_token).as_str())) {
                Err(e) => {
                    error!("Fail to read {}{}/ProxyPrivateKey.pem because of error: {:#?}", rsa_dir_path, user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            let rsa_crypto = match RsaCrypto::new(public_key, private_key) {
                Err(e) => {
                    error!("Fail to create rsa crypto for user: {} because of error: {:#?}", user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            result.cache.insert(user_token.to_string(), rsa_crypto);
        });
        Ok(result)
    }
}

impl RsaCryptoFetcher for ProxyRsaCryptoFetcher {
    fn fetch<Q>(&self, user_token: Q) -> Result<Option<&RsaCrypto>, PpaassError>
    where
        Q: AsRef<str>,
    {
        Ok(self.cache.get(user_token.as_ref()))
    }
}

pub(crate) trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Unpin {}

impl<T> AsyncReadAndWrite for T where T: AsyncRead + AsyncWrite + Unpin {}

#[pin_project]
#[derive(Debug)]
pub(crate) struct AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    id: String,
    #[pin]
    message_framed: Framed<T, MessageCodec<F>>,
    addresses: A,
}

impl<T, A, F> AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    pub(crate) fn new(concrete_read_write: T, addresses: A, rsa_crypto_fetcher: Arc<F>, compress: bool, buffer_size: usize) -> Self {
        let message_framed = Framed::with_capacity(concrete_read_write, MessageCodec::<F>::new(compress, rsa_crypto_fetcher), buffer_size);
        let id = generate_uuid();
        Self { message_framed, addresses, id }
    }

    pub(crate) fn get_id(&self) -> &str {
        self.id.as_str()
    }
}

impl<T, A, F> Stream for AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    type Item = Result<Message, PpaassError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.message_framed.poll_next(cx)
    }
}
impl<T, A, F> Sink<Message> for AgentConnection<T, A, F>
where
    T: AsyncReadAndWrite,
    A: ToSocketAddrs,
    F: RsaCryptoFetcher,
{
    type Error = PpaassError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.message_framed.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let this = self.project();
        this.message_framed.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.message_framed.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.message_framed.poll_close(cx)
    }
}
