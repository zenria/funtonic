use async_stream::try_stream;
use tonic::{transport::Server, Request, Response, Status};
use futures_util::pin_mut;
use rand::prelude::*;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

use hello_world::{
    server::{Greeter, GreeterServer},
    HelloReply, HelloRequest, StreamMeMaybeReply, StreamMeMaybeRequest,
};
use std::pin::Pin;
use std::time::Duration;

#[derive(Default)]
pub struct MyGreeter {
    data: String,
}

type Stream<T> =
    Pin<Box<dyn futures_core::Stream<Item = std::result::Result<T, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        println!("Got a request: {:?}", request);

        let string = &self.data;

        println!("My data: {:?}", string);

        let reply = hello_world::HelloReply {
            message: "Zomg, it works!".into(),
        };
        Ok(Response::new(reply))
    }
    type StreamMeMaybeStream = Stream<StreamMeMaybeReply>;

    async fn stream_me_maybe(
        &self,
        request: tonic::Request<StreamMeMaybeRequest>,
    ) -> Result<tonic::Response<Self::StreamMeMaybeStream>, tonic::Status> {
        let request = request.into_inner();
        let session: u128 = rand::thread_rng().gen();
        println!(
            "Client {} opening session: {:x}",
            request.client_id,session
        );

        let stream = try_stream! {
            let mut i: u64 = 1 ;
            loop {
                let delay = tokio::timer::delay_for(Duration::from_secs(i));
                delay.await;
                println!(
                    "Client {}-{:x} - new task",
                    request.client_id,session
                );
                let task_id = format!("{}-{:x}-{}", request.client_id,session, i);
                yield StreamMeMaybeReply {
                    task_id: task_id.into(),
                    task_payload: "foo_bar".into()
                };
                i+=1;
            }
        };
        Ok(Response::new(Box::pin(stream) as Self::StreamMeMaybeStream))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse().unwrap();
    let greeter = MyGreeter::default();

    Server::builder()
        .serve(addr, GreeterServer::new(greeter))
        .await?;

    Ok(())
}
