// Pseudo-code for the Host
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::Mutex;

// Shared state to hold online clients
type ClientMap = Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Message>>>>;

#[tokio::main]
async fn main() {
    // 1. Define the Tailscale IP and Port
    // REPLACE this with your actual Tailscale IP
    let addr: SocketAddr = "100.75.10.5:3000".parse().unwrap(); 

    let clients = ClientMap::default();

    let app = Router::new()
        .route("/ws", get(ws_handler)) // Clients connect here
        .route("/", get(dashboard_handler)) // Browser UI to see clients
        .with_state(clients);

    println!("Host listening strictly on Tailscale: {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// Handle new WebSocket connections
async fn ws_handler(ws: WebSocketUpgrade, State(clients): State<ClientMap>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, clients))
}

async fn handle_socket(mut socket: WebSocket, clients: ClientMap) {
    // Logic: 
    // 1. Wait for a "Hello" message with the Client's ID (e.g., "Laptop").
    // 2. Add to `clients` HashMap.
    // 3. Listen for binary messages (files) and route them to the target client.
}