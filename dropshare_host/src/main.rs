use mimalloc::MiMalloc;
use axum::{
    extract::{
        DefaultBodyLimit,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Multipart, State, Path, ConnectInfo,
},
middleware::{self,Next},
    http::{StatusCode,Request},
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use std::{
    fs::{File,self},
    collections::HashMap,
    net::{SocketAddr, UdpSocket, IpAddr}, // Added IpAddr
    sync::{Arc, Mutex},
    path::PathBuf,
    io::{Write, BufRead, BufReader},
    time::Instant
};
use tokio::sync::mpsc;
use serde::Serialize;
use futures::{sink::SinkExt, stream::StreamExt};
type Tx = mpsc::Sender<Message>;

#[derive(Clone)]
struct AppState {
    clients: Arc<Mutex<HashMap<String, Tx>>>,
}

#[derive(Serialize)]
struct ClientInfo {
    id: String,
    status: String,
}

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// CONFIGURATION LOADER
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
    println!("Reading configuration...");
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
                //Let's make it look nice
                let pretty_size = calculated_size as f64; //in megabytes
                if (pretty_size /(1000.0*1000.0*1000.0))>1.0{
                    println!("Configured Max Upload Size: {} Gigabytes", pretty_size/(1024.0*1024.0*1024.0));
                }else if (pretty_size /(1000.0*1000.0))>1.0{
                    println!("Configured Max Upload Size: {} Megabytes", pretty_size/(1024.0*1024.0));
                }else if (pretty_size /(1000.0))>1.0{
                    println!("Configured Max Upload Size: {} Kilobytes", pretty_size/1024.0);
                }
                return calculated_size;
            }
        }
    }

    // Fallback if parsing failed
    println!("Warning: Could not parse file_Size parameter in hconfig.ini || Using default 1GB Max Upload Size.");
    1024 * 1024 * 1024
}



/// Helper to find the LAN IP so other devices can connect
fn get_local_ip() -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip().to_string())
}

///Ensure that certificates are made before running
fn ensure_certificates() -> Result<(), Box<dyn std::error::Error>> {
    let cert_path = PathBuf::from("cert.pem");
    let key_path = PathBuf::from("key.pem");

    // Create host directory if it doesn't exist
    if let Some(parent) = cert_path.parent() {
        fs::create_dir_all(parent)?;
    }if cert_path.exists() && key_path.exists() {
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
        println!("The local IP is used for making certificates ONLY");

        if let Ok(ip) = ip_str.parse::<IpAddr>() {
            params.subject_alt_names.push(rcgen::SanType::IpAddress(ip));
        }
    }

    let cert = rcgen::Certificate::from_params(params)?;
    let pem_serialized = cert.serialize_pem()?;
    let key_serialized = cert.serialize_private_key_pem();

    fs::write(&cert_path, pem_serialized)?;
    fs::write(&key_path, key_serialized)?;

    println!("HTTPS Certificates generated successfully!");
    Ok(())
}

async fn tailscale_only_middleware<B>(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<B>,
    next: Next<B>,
) -> Response {
    let ip = addr.ip();

    // Tailscale uses the Carrier-Grade NAT (CGNAT) space: 100.64.0.0/10
    let is_tailscale = match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // Checks if the IP starts with 100. and the second octet is between 64 and 127
            octets[0] == 100 && (octets[1] >= 64 && octets[1] <= 127)
        }
        std::net::IpAddr::V6(ipv6) => {
            // Tailscale IPv6 space starts with fd7a:115c:a1e0::/48
            let segments = ipv6.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
    };

    // Allow Tailscale IPs, and allow localhost so you can still test it directly on the Pi
    if is_tailscale || ip.is_loopback() {
        next.run(request).await // Let them through!
    } else {
        // Drop the connection and log the intrusion attempt
        println!("Blocked unauthorized network attempt from: {}", ip);
        (StatusCode::FORBIDDEN, "Access Denied. Try again some other time, sucker.").into_response()
    }
}

//This is the main function
#[tokio::main(worker_threads = 2)]
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
    let certificate_config = RustlsConfig::from_pem_file("cert.pem", "key.pem")
        .await
        .expect("Failed to load HTTPS Certificates! Did you generate them?");

    let state = AppState {
        clients: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/api/clients", get(list_clients))
        .route("/api/upload/:target", post(upload_file))
        .route("/ws", get(ws_handler))
        .with_state(state)
        .layer(DefaultBodyLimit::max(max_file_size)) // Change this from our lovely config folder
        .layer(middleware::from_fn(tailscale_only_middleware)); //implement the tailscale barrier to prevent others from getting in.

    let host_address = SocketAddr::from(([0, 0, 0, 0], 3000)); //LocalHost
    println!("HTTPS Server listening for clients.\n");

    axum_server::bind_rustls(host_address, certificate_config)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}
///Uses the HTML in index
async fn serve_index() -> Html<&'static str> {
    Html(include_str!("../index.html"))//maybe go through and clean up the HTML?
}
///Lists the clients that are active and can be spoken to
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
    let (tx, mut rx) = mpsc::channel(32);
    
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

                println!("Client Connected: {}\n", id);

                state.clients.lock().unwrap().insert(id.clone(), tx.clone());
                client_id = Some(id);
            }
        }
    }

    send_task.abort();
    if let Some(id) = client_id {
        println!("Client Disconnected: {}\n", id);
        state.clients.lock().unwrap().remove(&id);
    }
}

async fn upload_file(
    Path(target_id): Path<String>,
    headers: axum::http::HeaderMap,
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

    //get the expected size of the file!
    let expected_size: u64 = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse().ok())
        .unwrap_or(0);

    let mut last_print=Instant::now(); 
    // Loop through the "form" data
    while let Ok(Some(mut field)) = multipart.next_field().await {
        let filename = field.file_name().unwrap_or("unknown.bin").to_string();
        //grab the file name.
        println!("Receiving file stream: {}", filename); 

        //modify this to handle the filename and the file size. separate by pipe...
        let metadata_msg = format!("{}|{}", filename, expected_size);
        if tx.send(Message::Text(metadata_msg)).await.is_err() {
            println!("Error: Client disconnected before transfer.");
            break;
        }

        let mut total_bytes = 0;

        // 2. Stream the file natively using Axum's .chunk() method
        // This pulls small chunks (usually a few KB at a time) from the HTTP request
        while let Ok(Some(chunk)) = field.chunk().await {
            total_bytes += chunk.len();
            
            // Instantly send that small chunk over the WebSocket
            if tx.send(Message::Binary(chunk.to_vec())).await.is_err() {
                println!("Error: Client disconnected during the transfer of {}.",filename);
                break; // Stop reading if the client drops
            }
        }
            //Make this look better.
        let bytemark = total_bytes as f64;
    if last_print.elapsed().as_millis()>200{
            if ( bytemark/(1000.0*1000.0*1000.0))>1.0{
                    println!("Sending Data:  {:.2} Gigabytes", bytemark/(1024.0*1024.0*1024.0));
                }else if (bytemark /(1000.0*1000.0))>1.0{
                    println!("Sending Data:  {:.2} Megabytes", bytemark/(1024.0*1024.0));
                }else if (bytemark /(1000.0))>1.0{
                    println!("Sending Data:  {:.2} Kilobytes", bytemark/1024.0);
                }
                last_print = Instant::now();
    }//printBlock
        
        // 3. Tell the Windows client to close and save the file
        let _ = tx.send(Message::Text("EOF".to_string())).await;
    }

    println!("Upload complete!");
    (StatusCode::OK, "File Sent!").into_response()
}