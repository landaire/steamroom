pub mod capture;
pub mod recording;
pub mod replay;
pub mod tcp;
pub mod websocket;

use crate::error::Error;
use bytes::Bytes;

pub trait Transport: Send + Sync + 'static {
    fn send(
        &self,
        payload: &[u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>>;

    fn recv(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>>;
}
