use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Multipart, State, Path,
    },
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};

use axum::extract::DefaultBodyLimit; //This import defines how big the files are allowed to be.

use axum_server::tls_rustls::RustlsConfig;
use std::{
    fs::{File,self},
    collections::HashMap,
    net::{SocketAddr, UdpSocket, IpAddr}, // Added IpAddr
    sync::{Arc, Mutex},
    path::PathBuf,
    io::{Write, BufRead, BufReader},
};
use tokio::sync::mpsc;
use serde::Serialize;
use futures::{sink::SinkExt, stream::StreamExt};

type Tx = mpsc::UnboundedSender<Message>;

#[derive(Clone)]
struct AppState {
    clients: Arc<Mutex<HashMap<String, Tx>>>,
}

#[derive(Serialize)]
struct ClientInfo {
    id: String,
    status: String,
}

// CONFIGURATION LOADER
fn load_or_create_config() -> usize {
    let config_path = "hConfig.ini";
    let default_size_str = "1024*1024*1024"; // 1GB representation
    
    // 1. Create file if it doesn't exist
    if !std::path::Path::new(config_path).exists() {
        println!("Config file not found. Creating {}...", config_path);
        let mut file = File::create(config_path).expect("Failed to create config file");
        writeln!(file, "[Settings]").unwrap();
        writeln!(file, "# Set the max upload size in bytes (math allowed, e.g., 1024*1024*1024 = 1GB, so change it as you need to and re-run the EXE.)").unwrap();
        writeln!(file, "file_Size = {}", default_size_str).unwrap();
        
        // Return default (1GB)
        return 1024 * 1024 * 1024;
    }

    // 2. Read existing file
    println!("Reading configuration from {}...", config_path);
    let file = File::open(config_path).expect("Failed to open config file");
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.unwrap_or_default();
        let line = line.trim();

        // Ignore comments and empty lines
        if line.starts_with('#') || line.is_empty() || line.starts_with('[') {
            continue;
        }

        // Parse "file_Size = ..."
        if line.starts_with("file_Size") {
            if let Some(value_part) = line.split('=').nth(1) {
                // MAGIC: Split by '*' and multiply parts to allow "1024*1024" syntax
                let calculated_size: usize = value_part
                    .split('*')
                    .map(|s| s.trim().parse::<usize>().unwrap_or(1))
                    .product();
                
                println!("Configured Max Upload Size: {} bytes", calculated_size);
                return calculated_size;
            }
        }
    }

    // Fallback if parsing failed
    println!("Warning: Could not parse file_Size. Using default 1GB.");
    1024 * 1024 * 1024
}



// Helper to find the LAN IP so other devices can connect
fn get_local_ip() -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip().to_string())
}

fn ensure_certificates() -> Result<(), Box<dyn std::error::Error>> {
    let cert_path = PathBuf::from("cert.pem");
    let key_path = PathBuf::from("key.pem");

    // Create host directory if it doesn't exist
    if let Some(parent) = cert_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if cert_path.exists() && key_path.exists() {
        return Ok(());
    }

    println!("Generating self-signed certificates...");

    // 1. Generate params 
    let mut params = rcgen::CertificateParams::new(vec![
        "localhost".to_string(), 
        "127.0.0.1".to_string()
    ]);

    // 2. Add LAN IP
    if let Some(ip_str) = get_local_ip() {
        println!("Adding local IP to cert: {}", ip_str);
        if let Ok(ip) = ip_str.parse::<IpAddr>() {
            params.subject_alt_names.push(rcgen::SanType::IpAddress(ip));
        }
    }

    // 3. Generate Certificate (This exists in v0.11)
    let cert = rcgen::Certificate::from_params(params)?;
    
    // 4. Serialize
    let pem_serialized = cert.serialize_pem()?;
    let key_serialized = cert.serialize_private_key_pem();

    // 5. Write to disk
    fs::write(&cert_path, pem_serialized)?;
    fs::write(&key_path, key_serialized)?;

    println!("Certificates generated successfully!");
    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    
    // 1. LOAD CONFIGURATION
    let max_file_size = load_or_create_config();

    // 2. Generate keys BEFORE loading them
    if let Err(e) = ensure_certificates() {
        eprintln!("Error generating certificates: {}", e);
        return;
    }

    // 3. Setup HTTPS Config
    let config = RustlsConfig::from_pem_file("cert.pem", "key.pem")
        .await
        .expect("Failed to load certs! Did you generate them?");

    let state = AppState {
        clients: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/api/clients", get(list_clients))
        .route("/api/upload/:target", post(upload_file))
        .route("/ws", get(ws_handler))
        .with_state(state)

        //THIS CONTROLS FILE SIZE. CHANGE AS LARGE OR AS SMALL AS NEEDED.
        .layer(DefaultBodyLimit::max(max_file_size)); // Change this from our lovely config folder


    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("HTTPS Server listening on https://{}/", addr);

    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn serve_index() -> Html<&'static str> {
    Html(include_str!("../index.html"))
}

async fn list_clients(State(state): State<AppState>) -> Json<Vec<ClientInfo>> {
    let map = state.clients.lock().unwrap();
    let list = map.keys()
        .map(|k| ClientInfo { id: k.clone(), status: "Online".into() })
        .collect();
    Json(list)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    
    let mut client_id: Option<String> = None;

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() { break; }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if text.starts_with("ID:") {
                let id = text.replace("ID:", "");
                println!("Client Connected: {}", id);
                state.clients.lock().unwrap().insert(id.clone(), tx.clone());
                client_id = Some(id);
            }
        }
    }

    send_task.abort();
    if let Some(id) = client_id {
        println!("Client Disconnected: {}", id);
        state.clients.lock().unwrap().remove(&id);
    }
}

async fn upload_file(
    Path(target_id): Path<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    println!("Starting upload to: {}", target_id); // Log start

    let tx = {
        let map = state.clients.lock().unwrap();
        match map.get(&target_id) {
            Some(tx) => tx.clone(),
            None => {
                println!("Error: Target {} not found", target_id);
                return (StatusCode::NOT_FOUND, "Client not found").into_response();
            }
        }
    };

    // Loop through the "form" data
    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field.file_name().unwrap_or("unknown.bin").to_string();
        println!("Receiving file: {}", filename); // Log filename

        // Read the data (This might take a while for big files)
        match field.bytes().await {
            Ok(data) => {
                println!("File size: {} bytes. Sending to client...", data.len());
                let _ = tx.send(Message::Text(filename));
                let _ = tx.send(Message::Binary(data.to_vec()));
            }
            Err(e) => println!("Failed to read file bytes: {}", e),
        }
    }

    println!("Upload complete!");
    (StatusCode::OK, "File Sent!").into_response()
}