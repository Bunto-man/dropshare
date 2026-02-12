// Pseudo-code for the Client
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{StreamExt, SinkExt};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    let connect_addr = "ws://100.106.87.107:3000/ws";
    let (ws_stream, _) = connect_async(connect_addr).await.expect("Failed to connect");
    let (mut write, mut read) = ws_stream.split();

    // 1. Identify ourselves
    write.send(Message::Text("Identity:Laptop".to_string())).await.unwrap();

    // 2. Ensure shared folder exists
    fs::create_dir_all("./shared").await.unwrap();

    println!("Client connected. Waiting for files...");

    // 3. Listen loop
    while let Some(msg) = read.next().await {
        let msg = msg.unwrap();
        if let Message::Binary(data) = msg {
            // In a real app, you need a header to know the filename.
            // For now, let's just write to a timestamped file.
            let filename = format!("./shared/received_{}.bin", chrono::Utc::now().timestamp());
            let mut file = File::create(filename).await.unwrap();
            file.write_all(&data).await.unwrap();
            println!("File received and saved!");
        }
    }
}