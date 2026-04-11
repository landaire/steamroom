use super::Transport;
use crate::connection::CmServer;
use crate::error::ConnectionError;
use crate::error::Error;
use bytes::Bytes;
use futures_util::SinkExt;
use futures_util::StreamExt;
use std::pin::Pin;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub struct WebSocketTransport {
    sink: Mutex<futures_util::stream::SplitSink<WsStream, Message>>,
    stream: Mutex<futures_util::stream::SplitStream<WsStream>>,
}

impl WebSocketTransport {
    pub async fn connect(server: &CmServer) -> Result<Self, Error> {
        let (host, port) = match &server.addr {
            crate::connection::CmServerAddr::Dns { host, port } => (host.clone(), *port),
            crate::connection::CmServerAddr::Resolved(addr) => (addr.ip().to_string(), addr.port()),
        };

        let url = format!("wss://{host}:{port}/cmsocket/");
        tracing::debug!("websocket connecting to {url}");

        let (ws, _) = connect_async(&url).await.map_err(|e| {
            ConnectionError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                e,
            ))
        })?;

        let (sink, stream) = ws.split();
        Ok(Self {
            sink: Mutex::new(sink),
            stream: Mutex::new(stream),
        })
    }
}

impl Transport for WebSocketTransport {
    fn send(
        &self,
        payload: &[u8],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>> {
        let msg = Message::Binary(payload.to_vec().into());
        Box::pin(async move {
            self.sink.lock().await.send(msg).await.map_err(|e| {
                ConnectionError::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, e))
            })?;
            Ok(())
        })
    }

    fn recv(&self) -> Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>> {
        Box::pin(async move {
            loop {
                let msg = self
                    .stream
                    .lock()
                    .await
                    .next()
                    .await
                    .ok_or(ConnectionError::Disconnected)?
                    .map_err(|e| {
                        ConnectionError::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, e))
                    })?;

                match msg {
                    Message::Binary(data) => return Ok(Bytes::from(data.to_vec())),
                    Message::Close(_) => return Err(ConnectionError::Disconnected.into()),
                    _ => continue, // skip ping/pong/text
                }
            }
        })
    }
}
