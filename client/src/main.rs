use futures_util::{SinkExt, StreamExt};
use url::Url;
use tokio_tungstenite::{tungstenite::{Message}, client_async};
// NEW IMPORTS
use native_tls::TlsConnector;
use tokio_native_tls::TlsConnector as TokioTlsConnector;
use std::{fs::{self, File},
io::{BufRead, BufReader, Write},
path::Path,
process,
};
struct ClientConfig {
    host_ip: String,
    device_name: String,
}

fn load_config() -> ClientConfig {
    let config_path = "cli_config.ini";

    // 1. If file does not exist, create it and EXIT
    if !Path::new(config_path).exists() {
        println!("Configuration file not found.");
        println!("Creating '{}'...", config_path);

        let mut file = File::create(config_path).expect("Failed to create config file");
        writeln!(file, "host_tailscale_ip = \"\"").unwrap();
        writeln!(file, "client_device_name = \"\"").unwrap();

        println!("------------------------------------------------");
        println!("PLEASE EDIT 'cli_config.ini' with your Host IP and Device Name.");
        println!("The program will now exit.");
        println!("------------------------------------------------");
        
        // STOP THE PROGRAM HERE
        process::exit(1); 
    }

    // 2. Read the file
    let file = File::open(config_path).expect("Failed to open config file");
    let reader = BufReader::new(file);

    let mut host_ip = String::new();
    let mut device_name = String::new();

    for line in reader.lines() {
        let line = line.unwrap_or_default();
        let line = line.trim();

        if line.starts_with("host_tailscale_ip") {
            // Extract content between quotes
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if start < end {
                        host_ip = line[start+1..end].to_string();
                    }
                }
            }
        } else if line.starts_with("client_device_name") {
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if start < end {
                        device_name = line[start+1..end].to_string();
                    }
                }
            }
        }
    }

    // 3. Validation: Did the user actually fill it out?
    if host_ip.is_empty() || device_name.is_empty() {
        eprintln!("Error: 'cli_config.ini' is missing values!");
        eprintln!("Please open the file and enter your Host IP and Device Name inside the quotes.");
        process::exit(1);
    }

    ClientConfig {
        host_ip,
        device_name,
    }
}







#[tokio::main]
async fn main() {

    // 1. LOAD CONFIG (Will exit here if file is new)
    let config = load_config();

    println!("Loaded Configuration:");
    println!("  Host: {}", config.host_ip);
    println!("  Name: {}", config.device_name);

    let connect_addr = format!("wss://{}:3000/ws", config.host_ip);
    let url = Url::parse(&connect_addr).expect("Bad URL format");

    println!("Connecting to {} (Ignoring Cert Errors)...", url);
    println!("\nVisit the page using https://{}:3000/",config.host_ip); //maybe this works better?

    
    
    // 1. Create a TCP stream first
    let tcp_stream = tokio::net::TcpStream::connect(format!("{}:3000", config.host_ip))
        .await
        .expect("Failed to open TCP connection");

    // 2. Configure TLS to accept ANY certificate (The "Unsafe" Fix)
    let cx = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();
    let cx = TokioTlsConnector::from(cx);

    // 3. Wrap TCP in TLS
    let tls_stream = cx.connect(&config.host_ip, tcp_stream).await.expect("TLS Handshake failed");

    // 4. Start WebSocket over the TLS stream
    let (ws_stream, _) = client_async(url.to_string(), tls_stream)
        .await
        .expect("Failed to start WebSocket");

    println!("Connected Securely!");
    
    let (mut write, mut read) = ws_stream.split();
    
    // 1. Identify ourselves by this name.
    let my_id = config.device_name; 

    write.send(Message::Text(format!("ID:{}", my_id).into())).await.expect("Login Failed");

    let save_dir = "files"; //put the save directory wherever the program runs

    fs::create_dir_all(save_dir).unwrap();
    println!("Client {} Active. Files will be saved to: {}",my_id, save_dir);

    let mut pending_filename: Option<String> = None;

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                println!("Incoming file: {}", text);
                pending_filename = Some(text.to_string());
            }
            Ok(Message::Binary(data)) => {
                if let Some(name) = pending_filename.take() {
                    let path = format!("{}/{}", save_dir, name);
                    let mut file = File::create(&path).unwrap();
                    file.write_all(&data).unwrap();
                    println!("File saved: {}", path);
                } else {
                    println!("Error: No filename header!");
                }
            }
            _ => {}
        }
    }
}