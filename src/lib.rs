#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]

pub use bma_jrpc_derive::rpc_client;
use futures_lite::io::AsyncReadExt;
use http::status::StatusCode;
use isahc::config::Configurable;
use isahc::{AsyncReadResponseExt, ReadResponseExt, RequestExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt;
use std::sync::atomic;
use std::time::Duration;

const JSONRPC_VER: &str = "2.0";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

const MIME_JSON: &str = "application/json";
#[cfg(feature = "msgpack")]
const MIME_MSGPACK: &str = "application/msgpack";

pub trait Encoder: Default {
    fn encode<P: Serialize>(&self, payload: &P) -> Result<Vec<u8>, Error>;
    fn decode<'a, R: Deserialize<'a>>(&self, data: &'a [u8]) -> Result<R, Error>;
    fn mime(&self) -> &'static str;
}

#[derive(Default)]
pub struct Json {}

impl Encoder for Json {
    #[inline]
    fn encode<P: Serialize>(&self, payload: &P) -> Result<Vec<u8>, Error> {
        serde_json::to_vec(payload).map_err(Into::into)
    }
    #[inline]
    fn decode<'a, R: Deserialize<'a>>(&self, data: &'a [u8]) -> Result<R, Error> {
        serde_json::from_slice(data).map_err(Into::into)
    }
    #[inline]
    fn mime(&self) -> &'static str {
        MIME_JSON
    }
}

#[cfg(feature = "msgpack")]
#[derive(Default)]
pub struct MsgPack {}

#[cfg(feature = "msgpack")]
impl Encoder for MsgPack {
    #[inline]
    fn encode<P: Serialize>(&self, payload: &P) -> Result<Vec<u8>, Error> {
        rmp_serde::to_vec_named(payload).map_err(Into::into)
    }
    #[inline]
    fn decode<'a, R: Deserialize<'a>>(&self, data: &'a [u8]) -> Result<R, Error> {
        rmp_serde::from_slice(data).map_err(Into::into)
    }
    #[inline]
    fn mime(&self) -> &'static str {
        MIME_MSGPACK
    }
}

#[derive(Serialize)]
struct Request<'a, P> {
    jsonrpc: &'static str,
    id: usize,
    method: &'a str,
    params: P,
}

#[derive(Deserialize)]
struct Response<'a, R> {
    jsonrpc: &'a str,
    id: usize,
    result: Option<R>,
    error: Option<RpcError>,
}

#[derive(Deserialize, Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct RpcError {
    code: i16,
    message: Option<String>,
}

impl RpcError {
    #[inline]
    pub fn code(&self) -> i16 {
        self.code
    }
    #[inline]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

#[inline]
pub fn http_client(url: &str) -> HttpClient<Json> {
    HttpClient::<Json>::new(url)
}

pub struct HttpClient<C>
where
    C: Encoder,
{
    req_id: atomic::AtomicUsize,
    url: String,
    timeout: Duration,
    encoder: C,
}

pub trait Rpc {
    fn call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P) -> Result<R, Error>;
}

impl<C> Rpc for HttpClient<C>
where
    C: Encoder,
{
    fn call<P, R>(&self, method: &str, params: P) -> Result<R, Error>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let (http_request, id) = self.prepare_http_request(method, params)?;
        let mut http_response = http_request.send()?;
        if http_response.status() == StatusCode::OK {
            self.parse_response(&http_response.bytes()?, id)
        } else {
            Err(Error::Http(http_response.status(), http_response.text()?))
        }
    }
}

impl<C> HttpClient<C>
where
    C: Encoder,
{
    #[inline]
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            req_id: atomic::AtomicUsize::new(0),
            encoder: C::default(),
        }
    }
    #[inline]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    #[inline]
    fn prepare_http_request<'a, P: Serialize>(
        &'a self,
        method: &'a str,
        params: P,
    ) -> Result<(isahc::Request<Vec<u8>>, usize), Error> {
        let req = Request {
            jsonrpc: JSONRPC_VER,
            id: self.req_id.fetch_add(1, atomic::Ordering::SeqCst),
            method,
            params,
        };
        let payload = self.encoder.encode(&req)?;
        Ok((
            isahc::Request::post(&self.url)
                .timeout(self.timeout)
                .header("content-type", self.encoder.mime())
                .body(payload)?,
            req.id,
        ))
    }
    pub async fn call_async<P, R>(&self, method: &str, params: P) -> Result<R, Error>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let (http_request, id) = self.prepare_http_request(method, params)?;
        let mut resp = http_request.send_async().await?;
        if resp.status() == StatusCode::OK {
            let mut buf =
                Vec::with_capacity(usize::try_from(resp.body().len().unwrap_or_default())?);
            resp.body_mut().read_to_end(&mut buf).await?;
            self.parse_response(&buf, id)
        } else {
            Err(Error::Http(resp.status(), resp.text().await?))
        }
    }
    fn parse_response<'a, R: Deserialize<'a>>(&self, buf: &'a [u8], id: usize) -> Result<R, Error> {
        let resp: Response<R> = self.encoder.decode(buf)?;
        if resp.jsonrpc != JSONRPC_VER {
            return Err(Error::Protocol("invalid JSON RPC version"));
        }
        if resp.id != id {
            return Err(Error::Protocol("invalid response ID"));
        }
        if let Some(err) = resp.error {
            Err(Error::Rpc(err))
        } else if let Some(result) = resp.result {
            Ok(result)
        } else {
            Err(Error::Protocol("no result/error fields"))
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Protocol(&'static str),
    Rpc(RpcError),
    Transport(isahc::Error),
    Http(StatusCode, String),
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Protocol(s) => write!(f, "invalid server response: {}", s),
            Error::Rpc(e) => write!(f, "{} {}", e.code, e.message.as_deref().unwrap_or_default()),
            Error::Transport(s) => write!(f, "{}", s),
            Error::Http(code, s) => write!(f, "{} {}", code, s),
            Error::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for Error {}

macro_rules! impl_other_err {
    ($t: ty) => {
        impl From<$t> for Error {
            fn from(err: $t) -> Self {
                Self::Other(Box::new(err))
            }
        }
    };
}

impl From<isahc::http::Error> for Error {
    fn from(err: isahc::http::Error) -> Self {
        Self::Transport(err.into())
    }
}

impl From<isahc::Error> for Error {
    fn from(err: isahc::Error) -> Self {
        Self::Transport(err)
    }
}

impl_other_err!(serde_json::Error);
#[cfg(feature = "msgpack")]
impl_other_err!(rmp_serde::decode::Error);
#[cfg(feature = "msgpack")]
impl_other_err!(rmp_serde::encode::Error);
impl_other_err!(std::io::Error);
impl_other_err!(std::num::TryFromIntError);
