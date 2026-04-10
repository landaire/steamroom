pub mod tcp;
pub mod recording;
pub mod replay;
pub mod capture;

use bytes::Bytes;
use crate::error::Error;

pub trait Transport: Send + Sync + 'static {
    fn send(
        &self,
        payload: &[u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>>;

    fn recv(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>>;
}
