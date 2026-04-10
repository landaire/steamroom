use bytes::Bytes;
use crate::connection::CmServer;
use crate::error::Error;
use super::Transport;

pub struct TcpTransport {
    // private
}

impl TcpTransport {
    pub async fn connect(server: &CmServer) -> Result<Self, Error> {
        todo!()
    }
}

impl Transport for TcpTransport {
    async fn send(&self, payload: &[u8]) -> Result<(), Error> {
        todo!()
    }

    async fn recv(&self) -> Result<Bytes, Error> {
        todo!()
    }
}
