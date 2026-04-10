use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use std::net::SocketAddr;
use crate::connection::{CmServer, CmServerAddr};
use crate::connection::framing;
use crate::error::{ConnectionError, Error};
use super::Transport;

pub struct TcpTransport {
    reader: Mutex<tokio::io::ReadHalf<TcpStream>>,
    writer: Mutex<tokio::io::WriteHalf<TcpStream>>,
}

impl TcpTransport {
    pub async fn connect(server: &CmServer) -> Result<Self, Error> {
        let addr = match &server.addr {
            CmServerAddr::Resolved(addr) => *addr,
            CmServerAddr::Dns { host, port } => {
                let addrs: Vec<SocketAddr> = tokio::net::lookup_host(format!("{host}:{port}"))
                    .await
                    .map_err(|_| ConnectionError::DnsResolutionFailed)?
                    .collect();
                *addrs.first().ok_or(ConnectionError::DnsResolutionFailed)?
            }
        };

        let stream = TcpStream::connect(addr)
            .await
            .map_err(ConnectionError::Io)?;

        let (reader, writer) = tokio::io::split(stream);
        Ok(Self {
            reader: Mutex::new(reader),
            writer: Mutex::new(writer),
        })
    }
}

impl Transport for TcpTransport {
    async fn send(&self, payload: &[u8]) -> Result<(), Error> {
        let frame = framing::frame_bytes(payload);
        let mut writer = self.writer.lock().await;
        writer
            .write_all(&frame)
            .await
            .map_err(ConnectionError::Io)?;
        writer.flush().await.map_err(ConnectionError::Io)?;
        Ok(())
    }

    async fn recv(&self) -> Result<Bytes, Error> {
        let mut reader = self.reader.lock().await;

        // Read 8-byte header: u32 length + u32 magic
        let mut header = [0u8; 8];
        reader
            .read_exact(&mut header)
            .await
            .map_err(|_| ConnectionError::Disconnected)?;

        let length = u32::from_le_bytes(header[..4].try_into().unwrap());
        let magic = &header[4..8];
        if magic != framing::MAGIC {
            return Err(ConnectionError::BadMagic(
                u32::from_le_bytes(magic.try_into().unwrap()),
            )
            .into());
        }

        let mut payload = vec![0u8; length as usize];
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|_| ConnectionError::Disconnected)?;

        Ok(Bytes::from(payload))
    }
}
