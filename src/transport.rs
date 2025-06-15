use std::net::SocketAddr;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{WebSocketStream, connect_async};
use futures::{StreamExt, SinkExt};
use tokio_native_tls::TlsStream;
use std::error::Error;
use bytes::BytesMut;
use tokio_tungstenite::MaybeTlsStream;

pub enum Transport {
    Udp,
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
    WebSocket(WebSocketStream<MaybeTlsStream<TcpStream>>),
}

impl Transport {
    pub async fn connect_tcp(addr: SocketAddr) -> Result<Self, Box<dyn Error>> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Transport::Tcp(stream))
    }

    pub async fn connect_tls(addr: SocketAddr, domain: &str) -> Result<Self, Box<dyn Error>> {
        let tcp = TcpStream::connect(addr).await?;
        let cx = native_tls::TlsConnector::new()?;
        let cx = tokio_native_tls::TlsConnector::from(cx);
        let tls_stream = cx.connect(domain, tcp).await?;
        Ok(Transport::Tls(tls_stream))
    }

    pub async fn connect_ws(url: &str) -> Result<Self, Box<dyn Error>> {
        let (ws_stream, _) = connect_async(url).await?;
        Ok(Transport::WebSocket(ws_stream))
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<(), Box<dyn Error>> {
        match self {
            Transport::Udp => {
                // Use existing UDP implementation
                Ok(())
            }
            Transport::Tcp(stream) => {
                stream.write_all(data).await?;
                Ok(())
            }
            Transport::Tls(stream) => {
                stream.write_all(data).await?;
                Ok(())
            }
            Transport::WebSocket(ws) => {
                ws.send(tokio_tungstenite::tungstenite::Message::Binary(data.to_vec())).await?;
                Ok(())
            }
        }
    }

    pub async fn receive(&mut self) -> Result<BytesMut, Box<dyn Error>> {
        let mut buf = BytesMut::with_capacity(4096);
        match self {
            Transport::Udp => {
                // Use existing UDP implementation
                Ok(buf)
            }
            Transport::Tcp(stream) => {
                let mut temp_buf = vec![0u8; 4096];
                let n = stream.read(&mut temp_buf).await?;
                if n > 0 {
                    buf.extend_from_slice(&temp_buf[..n]);
                }
                Ok(buf)
            }
            Transport::Tls(stream) => {
                let mut temp_buf = vec![0u8; 4096];
                let n = stream.read(&mut temp_buf).await?;
                if n > 0 {
                    buf.extend_from_slice(&temp_buf[..n]);
                }
                Ok(buf)
            }
            Transport::WebSocket(ws) => {
                if let Some(msg) = ws.next().await {
                    match msg? {
                        tokio_tungstenite::tungstenite::Message::Binary(data) => {
                            buf.extend_from_slice(&data);
                            Ok(buf)
                        }
                        _ => Err("Unexpected WebSocket message type".into()),
                    }
                } else {
                    Err("WebSocket connection closed".into())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::time::Duration;
    use tokio::time::sleep;
    use tokio_tungstenite::accept_async;

    #[tokio::test]
    async fn test_tcp_transport() {
        // Start TCP server
        let server_addr: SocketAddr = "127.0.0.1:5683".parse().unwrap();
        let listener = TcpListener::bind(server_addr).await.unwrap();
        
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            socket.write_all(&buf[..n]).await.unwrap();
        });

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Test TCP client
        let mut transport = Transport::connect_tcp(server_addr).await.unwrap();
        let test_data = b"Hello TCP";
        transport.send(test_data).await.unwrap();
        
        let response = transport.receive().await.unwrap();
        assert_eq!(&response[..test_data.len()], test_data);
    }

    #[tokio::test]
    async fn test_websocket_transport() {
        // Start WebSocket server
        let server_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let listener = TcpListener::bind(server_addr).await.unwrap();
        
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws_stream = accept_async(stream).await.unwrap();
            
            while let Some(msg) = ws_stream.next().await {
                let msg = msg.unwrap();
                if msg.is_binary() {
                    ws_stream.send(msg).await.unwrap();
                }
            }
        });

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Test WebSocket client
        let mut transport = Transport::connect_ws("ws://127.0.0.1:8080").await.unwrap();
        let test_data = b"Hello WebSocket";
        transport.send(test_data).await.unwrap();
        
        let response = transport.receive().await.unwrap();
        assert_eq!(&response[..test_data.len()], test_data);
    }

    #[tokio::test]
    async fn test_error_handling() {
        // Test connection to non-existent server
        let result = Transport::connect_tcp("127.0.0.1:1234".parse().unwrap()).await;
        assert!(result.is_err());

        // Test connection to invalid WebSocket URL
        let result = Transport::connect_ws("invalid-url").await;
        assert!(result.is_err());
    }
}
