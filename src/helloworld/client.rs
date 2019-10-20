pub mod hello_world {
    tonic::include_proto!("helloworld");
}

use hello_world::{client::GreeterClient, HelloRequest};


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = GreeterClient::connect("http://[::1]:50051")?;

    let request = tonic::Request::new(HelloRequest {
        name: "hello".into(),
    });

    let response = client.say_hello(request).await?;

    println!("RESPONSE={:?}", response);

    let request = tonic::Request::new(hello_world::StreamMeMaybeRequest {
        client_id: "siderant".into(),
    });

    let mut response = client.stream_me_maybe(request).await?.into_inner();
    while let Some(task) = response.message().await? {
        println!("Task: {} - {}", task.task_id, task.task_payload);
    }
    Ok(())
}
