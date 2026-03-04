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
//----------------------------------------------------------------
struct ClientConfig {
    host_ip: String,
    device_name: String,
}

fn load_config() -> ClientConfig {
    let config_path = "cli_config.ini";

    // 1. If the client config file does not exist, create it and exit the program.
    if !Path::new(config_path).exists() {
        println!("Configuration file not found.");
        println!("Creating '{}'...", config_path);

        let mut file = File::create(config_path).expect("Failed to create config file");
        writeln!(file, "host_tailscale_ip = \"\"").unwrap();
        writeln!(file, "client_device_name = \"\"").unwrap();

        //Nice and pretty, see?
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

            //get the host device name
        if line.starts_with("host_tailscale_ip") {
            // Extract content between quotes
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if start < end {
                        host_ip = line[start+1..end].to_string();
                    }
                }
            }
            //get the client device
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
    //make a cheeky struct for the client.
    ClientConfig {
        host_ip,
        device_name,
    }
}







#[tokio::main]
async fn main() {
    //update 1.1: Add these variables here for file tracking.
    let mut active_file: Option<std::fs::File> = None;
    let mut total_bytes_received: u64 = 0;
    let mut expected_file_size: u64 = 0; // We will need the host to tell us this!

    // 1. LOAD CONFIG (Will exit here if file is new)
    let config = load_config();

    println!("Loaded Configuration:");
    println!(" Host IP: {}", config.host_ip);
    println!(" Client Name: {}", config.device_name);

    let connect_addr = format!("wss://{}:3000/ws", config.host_ip);
    let url = Url::parse(&connect_addr).expect("Bad URL format");

    println!("Connecting to {} (Ignoring Cert Errors)...", url);
    println!("\nVisit the page using https://{}:3000/",config.host_ip); //Change IP

    //linux devices cannot use ports like 443 by default. they are protected. Doesn't matter though.
    
    
    // 1. Create a TCP stream first
    let tcp_stream = tokio::net::TcpStream::connect(format!("{}:3000", config.host_ip)) //change IP
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

    println!("!~Connected Securely to {}~!",config.host_ip);
    
    let (mut write, mut read) = ws_stream.split();
    
    // 1. Identify ourselves by this name.
    let my_id = config.device_name; 

    write.send(Message::Text(format!("ID:{}", my_id).into())).await.expect("Login Failed");

    let save_dir = "files"; //put the save directory wherever the program runs

    fs::create_dir_all(save_dir).unwrap();
    println!("Client {} Active. Files will be saved to: {}",my_id, save_dir);

    
   

    //This block is used to send and append the file being sent.

   while let Some(msg) = read.next().await {

    match msg {
        Ok(Message::Text(text)) => {
            let text = text.trim();
            
            // Check if the host is telling us the file is done
            if text == "EOF" {
                println!("\nTransfer complete! File closed.");
                active_file = None; // Dropping the file handle closes it safely
                total_bytes_received = 0; // Reset for the next file
                expected_file_size = 0;   // Reset for the next file
            } else {
            // Split the incoming "filename|size" text
                let parts: Vec<&str> = text.split('|').collect();
                let actual_filename = parts[0];
        
            // Grab the size if it was included
                if parts.len() > 1 {
                    expected_file_size = parts[1].parse::<u64>().unwrap_or(0);
                }

                println!("Incoming file: {} (Expecting {} bytes)", actual_filename, expected_file_size);
                let path = Path::new(&save_dir).join(actual_filename);
                
                // Create the file (overwriting if it already exists by default)
                match File::create(&path) {
                    Ok(file) => {
                        active_file = Some(file); // Keep it open for chunks
                        println!("Created file, waiting for data...");
                    }
                    Err(e) => {
                        println!("Error: Could not create file at {:?}: {}", path, e);
                        active_file = None; //change the active file to prevent a softlock.
                    }
                }
            }
        }
        //lots of if statements. could this be redone to be a match?
        Ok(Message::Binary(data)) => {
    if let Some(file) = active_file.as_mut() {
        if let Err(e) = file.write_all(&data) {
            println!("\nError: Failed to write chunk: {}", e);
        } else {
            // 1. Add the new chunk's size to our running total
            total_bytes_received += data.len() as u64;

            // 2. Calculate and display the progress
            use std::io::Write; // Required for the flush() command below

            //DO NOT ALLOW IT TO DIVIDE BY ZERO
            if expected_file_size > 0 {
                let percentage = (total_bytes_received as f64 / expected_file_size as f64) * 100.0;
                
                // The '\r' moves the cursor back to the start of the line to overwrite it. how lovely.
                print!("\rDownloading... {:.1}%  ({}/{} bytes)", percentage, total_bytes_received, expected_file_size);
            } else {
                // Fallback if the host hasn't told us the total size yet
                //change it to megabytes to make it actually readable
                let total_megabytes_received = total_bytes_received/(1024*1024);
                print!("\rDownloading... {} total megabytes received", total_megabytes_received);
            }
            
            // Force the terminal to update the line immediately
            std::io::stdout().flush().unwrap(); 
        }
    } else {
        println!("\nError: Received file data, but no file is open!");
    }
}
Err(e) => {
    println!("\nWebSocket Error: {}", e); 
}
_ => {}
    }
}
}