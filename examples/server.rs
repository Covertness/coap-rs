extern crate coap;

use coap::{Server, IsMessage, Method};
use tokio::runtime::current_thread::Runtime;

fn main() {
    let addr = "127.0.0.1:5683";
	let mut server = Server::new(addr).unwrap();

	println!("Server up on {}", addr);

	Runtime::new().unwrap().block_on(async move {
		server.run(move |request| {
            match request.get_method() {
				&Method::Get => println!("request by get {}", request.get_path()),
				&Method::Post => println!("request by post {}", String::from_utf8(request.message.payload).unwrap()),
				&Method::Put => println!("request by put {}", String::from_utf8(request.message.payload).unwrap()),
				_ => println!("request by other method"),
			};
			
			return match request.response {
				Some(mut message) => {
					message.set_payload(b"OK".to_vec());
					Some(message)
				},
				_ => None
			};
    	}).await.unwrap();
	});
}
