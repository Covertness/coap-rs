use std::io::{Error, ErrorKind, Result};
use std::marker::PhantomData;
use std::sync::{Arc};
use std::collections::HashMap;
use std::time::Duration;
use std::task::{Poll, Context};
use std::pin::Pin;

use tokio::net::{UdpSocket, udp::SendHalf};
use tokio::task;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{channel, Sender, Receiver};
use tokio::stream::StreamExt;

use crate::Method;
use crate::message::packet::{Packet, ObserveOption};
use crate::message::response::{CoAPResponse, Status};
use crate::message::request::CoAPRequest;
use crate::message::IsMessage;


pub struct CoAPClientAsync<Transport> {
    tx: SendHalf,
    message_id: u16,
    _listener: task::JoinHandle<Result<()>>,
    _transport: PhantomData<Transport>,
    rx_handles: Arc<Mutex<HashMap<u32, (SenderKind, Sender<CoAPResponse>)>>>,
}

enum SenderKind {
    Request,
    Observer,
}


impl CoAPClientAsync<tokio::net::UdpSocket> {
    pub async fn new_udp<A, B>(bind_addr: A, peer_addr: B) -> Result<Self> 
    where
        A: tokio::net::ToSocketAddrs,
        B: tokio::net::ToSocketAddrs,
    {
        // Bind to local socket
        let transport = UdpSocket::bind(bind_addr).await?;
        transport.connect(peer_addr).await?;

        // TODO: we _could_ connect here to filter responses

        let (mut rx, tx) = transport.split();

        let rx_handles = Arc::new(Mutex::new(HashMap::<_, (SenderKind, Sender<CoAPResponse>)>::new()));
        let h = rx_handles.clone();

        // Create listener task
        let _listener = task::spawn(async move {
            let mut buff = bytes::BytesMut::new();

            while let Ok((n, _a)) = rx.recv_from(&mut buff).await {
                // Parse out packet
                let p = match Packet::from_bytes(&buff[..n]) {
                    Ok(packet) => packet,
                    Err(_) => return Err(Error::new(ErrorKind::InvalidInput, "packet error")),
                };

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

            Ok(())
        });

        Ok(Self {
            tx,
            message_id: 0,
            _listener,
            _transport: PhantomData,
            rx_handles,
        })
    }

    // Execute a CoAP request and return a response
    pub async fn request(&mut self, request: &CoAPRequest, timeout: Duration) -> Result<CoAPResponse> {

        // Fetch token from message
        let token = Self::token_from_slice(request.get_token());

        // Encode message
        let b = request.message.to_bytes().map_err(|_e| Error::new(ErrorKind::InvalidInput, "packet error"))?;
        
        // Generate response channel
        let (tx, rx) = channel(1);
        self.rx_handles.lock().await.insert(token, (SenderKind::Request, tx));

        let mut rx = rx.timeout(timeout);

        // Send request
        let _n = self.tx.send(&b).await?;

        // Await response
        // TODO: timeout could be builtin here?
        let rx = match rx.try_next().await? {
            Some(v) => v,
            None => return Err(Error::new(ErrorKind::InvalidInput, "no response")),
        };

        Ok(rx)
    }

    // Convenience method to perform a Get request
    pub async fn get(&mut self, url: &str, timeout: Duration) -> Result<CoAPResponse> {
        let (_domain, _port, path) = super::parse_coap_url(url)?;

        let mut packet = CoAPRequest::new();
        packet.set_path(path.as_str());

        self.request(&packet, timeout).await
    }

    // Convenience method to perform a Put request
    pub async fn put(&mut self, url: &str, data: &[u8], timeout: Duration) -> Result<CoAPResponse> {
        let (_domain, _port, path) = super::parse_coap_url(url)?;

        let mut request = CoAPRequest::new();
        request.set_method(Method::Put);
        request.set_token(Self::token());
        request.set_path(path.as_str());
        request.set_payload(data.to_vec());

        self.request(&request, timeout).await
    }

    // Convenience method to perform a Post request
    pub async fn post(&mut self, url: &str, data: &[u8], timeout: Duration) -> Result<CoAPResponse> {
        let (_domain, _port, path) = super::parse_coap_url(url)?;

        let mut request = CoAPRequest::new();
        request.set_method(Method::Post);
        request.set_token(Self::token());
        request.set_path(path.as_str());
        request.set_payload(data.to_vec());

        self.request(&request, timeout).await
    }


    // Start observation on a topic
    pub async fn observe(&mut self, url: &str) -> Result<CoAPObserverAsync> {
        // Setup response channel
        let (tx, rx) = channel(0);

        // Register as an observer

        let mut regester_req = CoAPRequest::new();
        regester_req.set_observe(vec![ObserveOption::Register as u8]);
        regester_req.set_token(Self::token());
        regester_req.set_path(url);

        let token = Self::token_from_slice(regester_req.get_token());

        let register_resp = self.request(&regester_req, Duration::from_secs(3)).await?;

        if *register_resp.get_status() != Status::Content {
            return Err(Error::new(ErrorKind::NotFound, "the resource not found"));
        }

        // Store response channel
        self.rx_handles.lock().await.insert(token, (SenderKind::Observer, tx));

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

        let deregister_resp = self.request(&deregister_req, Duration::from_secs(3)).await?;

        // TODO: anything to check here?
        let _ = deregister_resp;

        Ok(())
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

