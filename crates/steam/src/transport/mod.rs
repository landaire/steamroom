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
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;

    fn recv(&self) -> impl std::future::Future<Output = Result<Bytes, Error>> + Send;
}
