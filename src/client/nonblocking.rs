use std::io::{Error, ErrorKind, Result};
use std::marker::PhantomData;
use std::sync::{Arc};
use std::collections::HashMap;
use std::time::Duration;
use std::task::{Poll, Context};
use std::pin::Pin;
use std::net::SocketAddr;

use log::{error, debug, trace};

use tokio::net::{UdpSocket, udp::SendHalf};
use tokio::{task, time};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{channel, Sender, Receiver};
use tokio::stream::StreamExt;

use crate::Method;
use crate::message::packet::{Packet, ObserveOption};
use crate::message::response::{CoAPResponse, Status};
use crate::message::request::CoAPRequest;
use crate::message::IsMessage;


pub const COAP_MTU: usize = 1600;

pub struct CoAPClientAsync<Transport> {
    peer_addr: SocketAddr,
    udp_tx: SendHalf,
    message_id: u16,
    _listener: task::JoinHandle<Result<()>>,
    _transport: PhantomData<Transport>,
    rx_handles: Arc<Mutex<HashMap<u32, (SenderKind, Sender<CoAPResponse>)>>>,
}

enum SenderKind {
    Request,
    Observer,
}

/// RequestOptions for configuring requests
#[derive(Debug, Clone, PartialEq)]
pub struct RequestOptions {
    pub retries: usize,
    pub timeout: Duration,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            retries: 3,
            timeout: Duration::from_secs(2),
        }
    }
}

impl CoAPClientAsync<tokio::net::UdpSocket> {
    pub async fn new_udp<A>(peer_addr: A) -> Result<Self> 
    where
        A: tokio::net::ToSocketAddrs,
    {
        // Resolve peer address to determine local socket type
        let peer_addr = peer_addr.to_socket_addrs().await?.next();

        let bind_addr = match peer_addr {
            Some(SocketAddr::V6(_)) => ":::0",
            Some(SocketAddr::V4(_)) => "0.0.0.0:0",
            None => {
                error!("No peer address found");
                return Err(Error::new(ErrorKind::NotFound, "no peer address found"));
            }
        };

        let peer_addr = peer_addr.unwrap();

        // Bind to local socket
        let transport = UdpSocket::bind(bind_addr).await
            .map_err(|e| {
                error!("Error binding local socket: {:?}", e);
                e
            })?;

        debug!("Bound to socket: {}", transport.local_addr()?);

        let (mut udp_rx, udp_tx) = transport.split();

        let rx_handles = Arc::new(Mutex::new(HashMap::<_, (SenderKind, Sender<CoAPResponse>)>::new()));
        let h = rx_handles.clone();

        // Create listener task
        let _listener = task::spawn(async move {
            let mut buff = vec![0u8; COAP_MTU];

            debug!("Started listener task");

            loop {
                // Receive from socket
                let (n, a) = udp_rx.recv_from(&mut buff).await?;

                trace!("Received data: {:?} from {:?}", &buff[..n], a);

                // Parse out packet
                let p = match Packet::from_bytes(&buff[..n]) {
                    Ok(packet) => packet,
                    Err(_) => {
                        error!("Error decoding packet: {:?}", &buff[..n]);
                        return Err(Error::new(ErrorKind::InvalidInput, "packet error"))
                    },
                };

                debug!("Received packet: {:?}", p);

                // Fetch transaction token
                let token = Self::token_from_slice(p.get_token());

                // Locate matching request sender
                let mut handles = h.lock().await;
                let (kind, tx) = match handles.get_mut(&token) {
                    Some(v) => v,
                    None => {
                        // No handler bound, drop
                        continue;
                    }
                };

                // Send response
                tx.send(CoAPResponse { message: p }).await.map_err(|e| Error::new(ErrorKind::Other, e))?;

                // Remove handle when done
                match kind {
                    SenderKind::Request => handles.remove(&token),
                    _ => None,
                };
            }
        });

        Ok(Self {
            udp_tx,
            peer_addr,
            message_id: 0,
            _listener,
            _transport: PhantomData,
            rx_handles,
        })
    }

    // Convenience method to perform a Get request
    pub async fn get(&mut self, resource: &str, options: &RequestOptions) -> Result<CoAPResponse> {
        let mut request = CoAPRequest::new();
        request.set_method(Method::Get);
        request.set_token(Self::token());
        request.set_path(resource);

        self.request(&request, options).await
    }

    // Convenience method to perform a Put request
    pub async fn put(&mut self, resource: &str, data: &[u8], options: &RequestOptions) -> Result<CoAPResponse> {
        let mut request = CoAPRequest::new();
        request.set_method(Method::Put);
        request.set_token(Self::token());
        request.set_path(resource);
        request.set_payload(data.to_vec());

        self.request(&request, options).await
    }

    // Convenience method to perform a Post request
    pub async fn post(&mut self, resource: &str, data: &[u8], options: &RequestOptions) -> Result<CoAPResponse> {
        let mut request = CoAPRequest::new();
        request.set_method(Method::Post);
        request.set_token(Self::token());
        request.set_path(resource);
        request.set_payload(data.to_vec());

        self.request(&request, options).await
    }

    // Execute a CoAP request and return a response
    pub async fn request(&mut self, request: &CoAPRequest, options: &RequestOptions) -> Result<CoAPResponse> {

        // Fetch token from message
        let token = Self::token_from_slice(request.get_token());

        // Encode message
        let b = request.message.to_bytes().map_err(|_e| Error::new(ErrorKind::InvalidInput, "packet error"))?;
        
        // Generate response channel
        let (tx, mut rx) = channel(1);
        self.rx_handles.lock().await.insert(token, (SenderKind::Request, tx));

        self.do_request(&b, &mut rx, options).await
    }

    // Start observation on a topic
    pub async fn observe(&mut self, url: &str, options: &RequestOptions) -> Result<CoAPObserverAsync> {

        // Setup registration message
        let mut regester_req = CoAPRequest::new();
        regester_req.set_observe(vec![ObserveOption::Register as u8]);
        regester_req.set_token(Self::token());
        regester_req.set_path(url);

        let token = Self::token_from_slice(regester_req.get_token());

        // Encode message
        let b = regester_req.message.to_bytes().map_err(|_e| Error::new(ErrorKind::InvalidInput, "packet error"))?;

        // Setup response channel
        let (tx, mut rx) = channel(10);
        let mut tx1 = tx.clone();
        self.rx_handles.lock().await.insert(token, (SenderKind::Observer, tx));

        let register_resp = self.do_request(&b, &mut rx, options).await?;

        // Handle response errors (expect a 2.05 on successful observe)
        if *register_resp.get_status() != Status::Content {
            // TODO: remove response channel
            return Err(Error::new(ErrorKind::NotFound, format!("Unexpected status code {:?}", register_resp.get_code())));
        }

        // Forward first response to observer
        tx1.send(register_resp).await.unwrap();

        Ok(CoAPObserverAsync{
            topic: url.to_string(),
            token, rx
        })
    }

    pub async fn unobserve(&mut self, observer: CoAPObserverAsync) -> Result<()> {
        
        // Send deregister packet
        let mut deregister_req = CoAPRequest::new();
        deregister_req.set_message_id(self.message_id());
        deregister_req.set_observe(vec![ObserveOption::Deregister as u8]);
        deregister_req.set_path(observer.topic.as_str());
        deregister_req.set_token(observer.token.to_be_bytes().to_vec());

        let deregister_resp = self.request(&deregister_req, &RequestOptions::default()).await?;

        // TODO: anything to check here?
        let _ = deregister_resp;

        Ok(())
    }

    async fn do_request(&mut self, encoded: &[u8], rx: &mut Receiver<CoAPResponse>, options: &RequestOptions) -> Result<CoAPResponse> {

        for _i in 0..options.retries {
            // Send encoded data
            trace!("Transmit data: {:?}", encoded);
            let _n = self.udp_tx.send_to(&encoded, &self.peer_addr).await?;

            // Await response
            match time::timeout(options.timeout, rx.next()).await {
                Ok(Some(v)) => return Ok(v),
                _ => continue,
            }
        }

        Err(Error::new(ErrorKind::TimedOut, "no response"))
    }

    
    fn token() -> Vec<u8> {
        let t = rand::random::<u32>();
        t.to_be_bytes().to_vec()
    }

    fn token_from_slice(v: &[u8]) -> u32 {
        let mut token_raw = [0u8; 4];

        token_raw[..v.len()].copy_from_slice(v);

        u32::from_be_bytes(token_raw)
    }

    fn message_id(&mut self) -> u16 {
        let id = self.message_id;
        self.message_id += 1;
        id
    }
}

/// CoAPObserverAsync object can be polled for subscriptions
pub struct CoAPObserverAsync {
    topic: String,
    token: u32,
    rx: Receiver<CoAPResponse>,
}

impl tokio::stream::Stream for CoAPObserverAsync {
    type Item = CoAPResponse;

    fn poll_next(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx).poll_next(ctx)
    }
}

#[cfg(test)]
mod test {
    use crate::*;
    use super::*;

    #[tokio::test]
    async fn test_get() {
        let mut client = CoAPClientAsync::new_udp("coap.me:5683").await.unwrap();

        let resp = client.get("hello", &RequestOptions::default()).await.unwrap();
        assert_eq!(resp.message.payload, b"world".to_vec());
    }

    async fn request_handler(req: CoAPRequest) -> Option<CoAPResponse> {
        debug!("test server request: {:?}", req);

        let uri_path_list = req.get_option(CoAPOption::UriPath).unwrap().clone();
        assert_eq!(uri_path_list.len(), 1);

        match req.response {
            Some(mut response) => {
                response.set_payload(uri_path_list.front().unwrap().clone());
                Some(response)
            }
            _ => None,
        }
    }

    #[tokio::test]
    async fn test_observe() {
        let opts = RequestOptions::default();

        // Setup server
        let server_port = server::test::spawn_server(request_handler).recv().unwrap();

        // Setup client
        let mut client = CoAPClientAsync::new_udp((("0.0.0.0", server_port))).await.unwrap();

        // Put initial data
        client.put("test", b"hello world 1", &opts).await.unwrap();

        // Initiate observation
        let mut observe = client.observe("test", &opts).await.unwrap();

        // Await response
        let resp = time::timeout(Duration::from_secs(10), observe.next()).await.unwrap();

        println!("RX 1: {:?}", resp);

        assert_eq!(resp.unwrap().message.payload, b"hello world 1".to_vec());

        // Send request
        client.put("test", b"hello world 2", &opts).await.unwrap();

        let resp = time::timeout(Duration::from_secs(10), observe.next()).await.unwrap();

        println!("RX 2: {:?}", resp);

        assert_eq!(resp.unwrap().message.payload, b"hello world 2".to_vec());
    }
}