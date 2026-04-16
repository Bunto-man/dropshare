
use native_tls::TlsConnector;
use tungstenite::{client, Message};
use url::Url;

use std::{
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    net::TcpStream, // Standard standard library TCP!
    path::Path,
    process,
    time::Instant,
};
//----------------------------------------------------------------
//for the client and host name
struct ClientConfig {
    host_ip: String,
    device_name: String,
}
///Loads a config file, or creates a config file if there is not a config file available
/// 
/// * `ClientConfig` - creates a struct for the client.
/// * `file` - The cli_config.ini. appears in the directory.
///
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
    // You have to fill in cli_config.ini!
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

fn main() {
    // 1. File tracking variables (Now using BufWriter for high-speed, low-overhead saving)
    let mut active_file: Option<BufWriter<File>> = None;
    let mut total_bytes_received: u64 = 0;
    let mut expected_file_size: u64 = 0; 
    let mut last_print = Instant::now(); // Moved timer initialization here!

    let config = load_config();

    println!("Loaded Configuration:");
    println!(" Host IP: {}", config.host_ip);
    println!(" Client Name: {}", config.device_name);

    let connect_addr = format!("wss://{}:3000/ws", config.host_ip);
    let url = Url::parse(&connect_addr).expect("Bad URL format");
    let print_url = &config.host_ip;
    println!("Connecting to {} (Ignoring Cert Errors)...", url);
    println!("\nConnect by visiting: https://{}:3000\n",print_url);
    // 2. Standard Blocking TCP Connection
    let tcp_stream = TcpStream::connect(format!("{}:3000", config.host_ip))
        .expect("Failed to open TCP connection");

    // 3. Standard Blocking TLS Connector
    let tls_connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    // 4. Wrap TCP in TLS synchronously
    let tls_stream = tls_connector.connect(&config.host_ip, tcp_stream)
        .expect("TLS Handshake failed");

    // 5. Start WebSocket over the TLS stream using the base tungstenite crate
    let (mut socket, _) = client::client(url.to_string(), tls_stream)
        .expect("Failed to start WebSocket");

    println!("Connected Securely to {} !", config.host_ip);
    
    let my_id = config.device_name; 
    
    // Notice: No .await needed! It sends instantly.
    socket.send(Message::Text(format!("ID:{}", my_id).into())).expect("Login Failed");

    let save_dir = "files";

    fs::create_dir_all(save_dir).unwrap();
    println!("Client {} Active. Files will be saved to: {}", my_id, save_dir);

    // 6. The Synchronous Blocking Loop
    // Instead of `read.next().await`, we just run a loop and block on `socket.read()`
    loop {
        let msg = match socket.read() {
            Ok(m) => m,
            Err(e) => {
                println!("\nConnection closed or error: {}", e);
                break; // Exit the loop if the host disconnects
            }
        };

        match msg {
            Message::Text(text) => {
                let text = text.trim();
                
                if text == "EOF" {
                    println!("\nTransfer complete! File closed.\n");
                    active_file = None; // Dropping the BufWriter flushes remaining data and closes it
                    total_bytes_received = 0; 
                    expected_file_size = 0;
                    } else {
                    let parts: Vec<&str> = text.split('|').collect();
                    let actual_filename = parts[0];
            
                    if parts.len() > 1 {
                        expected_file_size = parts[1].parse::<u64>().unwrap_or(0);
                    }

                    println!("Incoming file: {} (Expecting {:.2} Megabytes)", 
                             actual_filename,expected_file_size as f64 / (1024.0 * 1024.0));
                             
                    let path = Path::new(&save_dir).join(actual_filename);
                    
                    match File::create(&path) {
                        Ok(file) => {
                            // Wrap the file in a 64KB BufWriter
                            active_file = Some(BufWriter::with_capacity(64 * 1024, file)); 
                            println!("Created file, waiting for data...");
                        }
                        Err(e) => {
                            println!("Error: Could not create file at {:?}: {}", path, e);
                            active_file = None; 
                        }
                    }
                }
            }
            Message::Binary(data) => {
                if let Some(file) = active_file.as_mut() {
                    // This writes to the RAM buffer first. It only hits the disk when the 64KB buffer is full!
                    if let Err(e) = file.write_all(&data) {
                        println!("\nError: Failed to write chunk: {}", e);
                    } else {
                        total_bytes_received += data.len() as u64;

                        if expected_file_size > 0 {
                            if last_print.elapsed().as_millis() > 200 {
                                let percentage = (total_bytes_received as f64 / expected_file_size as f64) * 100.0;
                                print!("\rDownloading... {:.2}%  ({:.2}/{:.2} MB received)", 
                                       percentage, 
                                       total_bytes_received as f64 / (1024.0 * 1024.0), 
                                       expected_file_size as f64 / (1024.0 * 1024.0));
                           
                                std::io::stdout().flush().unwrap(); 
                                last_print = Instant::now();
                            }
                        }
                    }
                } else {
                    println!("\nError: Received file data, but no file is open!");
                }
            }
            Message::Close(_) => {
                println!("\nHost closed the connection.");
                break;
            }
            // Tungstenite handles Ping/Pong automatically behind the scenes
            _ => {}
        }
    }
}
