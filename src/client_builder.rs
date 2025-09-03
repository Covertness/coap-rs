use crate::transport::Transport;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use std::error::Error;

pub struct CoAPClientBuilder {
    transport: Option<Transport>,
    addr: Option<SocketAddr>,
    domain: Option<String>,
    websocket_url: Option<String>,
}

impl CoAPClientBuilder {
    pub fn new() -> Self {
        CoAPClientBuilder {
            transport: None,
            addr: None,
            domain: None,
            websocket_url: None,
        }
    }

    pub fn with_udp(mut self, addr: SocketAddr) -> Self {
        self.addr = Some(addr);
        self.transport = Some(Transport::Udp);
        self
    }

    pub fn with_tcp(mut self, addr: SocketAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    pub fn with_tls(mut self, addr: SocketAddr, domain: String) -> Self {
        self.addr = Some(addr);
        self.domain = Some(domain);
        self
    }

    pub fn with_websocket(mut self, url: String) -> Self {
        self.websocket_url = Some(url);
        self
    }

    pub async fn build(self) -> Result<CoAPClient, Box<dyn Error>> {
        let transport = match (self.addr, self.domain, self.websocket_url) {
            (Some(addr), None, None) => {
                if let Some(Transport::Udp) = self.transport {
                    Transport::Udp
                } else {
                    Transport::connect_tcp(addr).await?
                }
            }
            (Some(addr), Some(domain), None) => {
                Transport::connect_tls(addr, &domain).await?
            }
            (None, None, Some(url)) => {
                Transport::connect_ws(&url).await?
            }
            _ => return Err("Invalid transport configuration".into()),
        };

        Ok(CoAPClient {
            transport,
        })
    }
}

pub struct CoAPClient {
    transport: Transport,
}

impl CoAPClient {
    pub async fn send(&mut self, data: &[u8]) -> Result<(), Box<dyn Error>> {
        self.transport.send(data).await
    }

    pub async fn receive(&mut self) -> Result<Vec<u8>, Box<dyn Error>> {
        let buf = self.transport.receive().await?;
        Ok(buf.to_vec())
    }
}
