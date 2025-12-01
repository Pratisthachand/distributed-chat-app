use dischat::{
    LamportClock, Message, MessageLog, PeerInfo, PeerManager, PeerMessage,
    PendingMessageQueue,
};
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{self, Duration};

const CHANNEL_SIZE: usize = 128;

#[tokio::main]
async fn main() -> io::Result<()> {
    let (server_id, client_port, peer_port, peers) = parse_args();
    
    println!("Server {} starting...", server_id);
    println!("  Client port: {}", client_port);
    println!("  Peer port: {}", peer_port);
    println!("  Connected to {} peers\n", peers.len());

    run_server(server_id, client_port, peer_port, peers).await
}

/// Parse command line arguments
/// Returns: (server_id, client_port, peer_port, peers)
fn parse_args() -> (u32, u16, u16, Vec<PeerInfo>) {
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 4 {
        eprintln!("Usage: {} <server_id> <client_port> <peer_port> [peer_addrs...]", args[0]);
        eprintln!("Example: {} 1 8081 9001 127.0.0.1:9002 127.0.0.1:9003", args[0]);
        std::process::exit(1);
    }

    let server_id: u32 = args[1].parse().expect("Invalid server_id");
    let client_port: u16 = args[2].parse().expect("Invalid client_port");
    let peer_port: u16 = args[3].parse().expect("Invalid peer_port");

    // Build peer list (assign sequential IDs, skip our own)
    let mut peers = Vec::new();
    for (i, addr) in args.iter().skip(4).enumerate() {
        let peer_id = if i + 1 < server_id as usize {
            (i + 1) as u32
        } else {
            (i + 2) as u32
        };
        peers.push(PeerInfo::new(peer_id, addr.clone()));
    }

    (server_id, client_port, peer_port, peers)
}

async fn run_server(
    server_id: u32,
    client_port: u16,
    peer_port: u16,
    peers: Vec<PeerInfo>,
) -> io::Result<()> {
    // Initialize core components
    let lamport_clock = Arc::new(LamportClock::new());
    let log_file = format!("server_{}.log", server_id);
    let message_log = Arc::new(MessageLog::new(log_file)?);
    let pending_queue = Arc::new(PendingMessageQueue::new(server_id, peers.clone()));

    // Crash recovery: replay log to restore messages and Lamport clock
    println!("[Server {}] Replaying message log...", server_id);
    match message_log.replay() {
        Ok((messages, max_timestamp)) => {
            println!("[Server {}] Recovered {} messages (max timestamp: {})", 
                server_id, messages.len(), max_timestamp);
            // Restore clock to highest seen timestamp
            lamport_clock.set(max_timestamp);
            // Re-add all messages to pending queue
            for msg in messages {
                pending_queue.enqueue(msg).await;
            }
        }
        Err(e) => eprintln!("[Server {}] Error replaying log: {}", server_id, e),
    }

    // Setup communication channels
    let (client_tx, _) = broadcast::channel::<String>(CHANNEL_SIZE);  // Broadcast to clients
    let (new_msg_tx, mut new_msg_rx) = mpsc::unbounded_channel::<Message>();  // From clients

    // Initialize peer manager (handles server-to-server communication)
    let (peer_manager, mut peer_rx) = PeerManager::new(
        server_id, 
        peers.clone(), 
        1000,  // Send heartbeat every 1 second
        5000   // Mark peer dead after 5 seconds
    );
    let peer_manager = Arc::new(peer_manager);
    peer_manager.start().await;  // Start connecting to peers

    // Listen for incoming peer connections from lower-numbered servers
    let peer_listener = TcpListener::bind(format!("0.0.0.0:{}", peer_port)).await?;
    println!("[Server {}] Listening for peers on port {}", server_id, peer_port);

    let peer_mgr_for_incoming = Arc::clone(&peer_manager);
    let peer_server_id = server_id;
    
    tokio::spawn(async move {
        loop {
            if let Ok((stream, _addr)) = peer_listener.accept().await {
                let peer_mgr = Arc::clone(&peer_mgr_for_incoming);
                let inbound_tx = peer_mgr.get_inbound_tx();
                let mut outbound_rx = peer_mgr.get_outbound_rx();
                
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut buf_reader = tokio::io::BufReader::new(reader);    
                                         
                    // Read peer ID
                    let mut line = String::new();
                    if buf_reader.read_line(&mut line).await.is_err() {
                        return;
                    }
                    let peer_id: u32 = line.trim().parse().unwrap_or(0);
                    
                    // Spawn reader task
                    let inbound_tx_clone = inbound_tx.clone();
                    let reader_task = tokio::spawn(async move {
                        let mut line = String::new();
                        loop {
                            line.clear();
                            match buf_reader.read_line(&mut line).await {
                                Ok(0) => break,
                                Ok(_) => {
                                    if let Ok(msg) = serde_json::from_str::<PeerMessage>(line.trim()) {
                                        let _ = inbound_tx_clone.send(msg);
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    });
                    
                    // Spawn writer task
                    let writer_task = tokio::spawn(async move {
                        while let Ok(msg) = outbound_rx.recv().await {
                            let json = serde_json::to_string(&msg).unwrap();
                            let line = format!("{}\n", json);
                            if writer.write_all(line.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    });
                    
                    tokio::select! {
                        _ = reader_task => {},
                        _ = writer_task => {},
                    }
                });
            }
        }
    });

    // Task: Process messages from clients
    // Flow: timestamp → persist to log → enqueue → replicate to peers
    let clock_clone = Arc::clone(&lamport_clock);
    let log_clone = Arc::clone(&message_log);
    let queue_clone = Arc::clone(&pending_queue);
    let peer_mgr_clone = Arc::clone(&peer_manager);
    let msg_id_counter = Arc::new(AtomicU64::new(1));

    tokio::spawn(async move {
        while let Some(mut message) = new_msg_rx.recv().await {
            // Assign Lamport timestamp (increment before sending)
            message.timestamp = clock_clone.increment();
            message.msg_id = msg_id_counter.fetch_add(1, Ordering::SeqCst);

            println!("[Server {}] New message [T:{}]: {}", 
                server_id, message.timestamp, message.content);

            // Persist FIRST (ensures durability if we crash)
            if let Err(e) = log_clone.append(&message) {
                eprintln!("[Server {}] Failed to persist: {}", server_id, e);
            }

            // Add to pending queue (waits for safe delivery)
            queue_clone.enqueue(message.clone()).await;

            // Broadcast to all peer servers
            peer_mgr_clone.broadcast(PeerMessage::ReplicateMessage(message));
        }
    });

    // Task: Process messages from peer servers
    // Flow: update clock → persist → enqueue
    let clock_clone = Arc::clone(&lamport_clock);
    let log_clone = Arc::clone(&message_log);
    let queue_clone = Arc::clone(&pending_queue);

    tokio::spawn(async move {
        while let Some(peer_msg) = peer_rx.recv().await {
            match peer_msg {
                PeerMessage::ReplicateMessage(message) => {
                    println!("[Server {}] Received replicated [T:{}] from Server {}", 
                        server_id, message.timestamp, message.server_id);
                    
                    // Update our clock: C = max(C, T_M) + 1
                    clock_clone.update(message.timestamp);
                    
                    // Persist the replicated message
                    if let Err(e) = log_clone.append(&message) {
                        eprintln!("[Server {}] Failed to persist: {}", server_id, e);
                    }
                    
                    // Add to pending queue
                    queue_clone.enqueue(message).await;
                }
                PeerMessage::Heartbeat { server_id: peer_id, timestamp } => {
                    // Update peer timestamp for safe delivery check
                    queue_clone.update_peer_timestamp(peer_id, timestamp).await;
                }
                _ => {}
            }
        }
    });

    // Task: Periodic message delivery (every 100ms)
    // Delivers messages that satisfy safe delivery condition
    let queue_clone = Arc::clone(&pending_queue);
    let client_tx_clone = client_tx.clone();

    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(100));
        loop {
            interval.tick().await;
            
            // Try to deliver pending messages
            let delivered = queue_clone.try_deliver().await;
            for msg in delivered {
                let display = format!("[T:{}] {}: {}", msg.timestamp, msg.client_name, msg.content);
                println!("[Server {}] DELIVERING: {}", server_id, display);
                // Broadcast to all local clients
                let _ = client_tx_clone.send(display);
            }
        }
    });

    // Task: Send heartbeats to peers (every 1 second)
    // Heartbeats include current Lamport timestamp
    let clock_clone = Arc::clone(&lamport_clock);
    let peer_mgr_clone = Arc::clone(&peer_manager);

    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(1000));
        loop {
            interval.tick().await;
            let timestamp = clock_clone.get();
            peer_mgr_clone.broadcast(PeerMessage::Heartbeat { server_id, timestamp });
        }
    });

    // Listen for client connections
    let client_listener = TcpListener::bind(format!("0.0.0.0:{}", client_port)).await?;
    println!("[Server {}] Listening for clients on port {}", server_id, client_port);

    let client_counter = Arc::new(AtomicU64::new(1));

    loop {
        let (stream, addr) = client_listener.accept().await?;
        println!("[Server {}] New client from {}", server_id, addr);

        let client_id = client_counter.fetch_add(1, Ordering::SeqCst);
        let client_rx = client_tx.subscribe();
        let new_msg_tx_clone = new_msg_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, client_rx, new_msg_tx_clone, server_id, client_id).await {
                eprintln!("[Server {}] Client error: {:?}", server_id, e);
            }
        });
    }
}

/// Handle a single client connection
/// Runs two tasks: reader (client → server) and writer (server → client)
async fn handle_client(
    stream: TcpStream,
    mut client_rx: broadcast::Receiver<String>,
    new_msg_tx: mpsc::UnboundedSender<Message>,
    server_id: u32,
    client_id: u64,
) -> io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let client_name = format!("Client#{}@S{}", client_id, server_id);

    // Send welcome message
    let welcome = format!("Welcome {}! Connected to Server {}.\n", client_name, server_id);
    writer.write_all(welcome.as_bytes()).await?;

    // Announce join
    let join = Message::new(0, server_id, "SYSTEM".to_string(), format!("{} joined!", client_name), 0);
    let _ = new_msg_tx.send(join);

    // Reader task: read messages from client
    let client_name_clone = client_name.clone();
    let new_msg_tx_clone = new_msg_tx.clone(); 
    let reader_task = tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => return Ok(()),  // Client disconnected
                Ok(_) => {
                    let content = line.trim().to_string();
                    if !content.is_empty() {
                        let msg = Message::new(0, server_id, client_name_clone.clone(), content, 0);
                        let _ = new_msg_tx_clone.send(msg);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    });

    // Writer task: send messages to client
    let writer_task = tokio::spawn(async move {
        loop {
            match client_rx.recv().await {
                Ok(msg) => {
                    if writer.write_all(format!("{}\n", msg).as_bytes()).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = reader_task => {},
        _ = writer_task => {},
    }

    // Announce leave
    let leave = Message::new(0, server_id, "SYSTEM".to_string(), format!("{} left.", client_name), 0);
    let _ = new_msg_tx.send(leave);

    Ok(())
}

/// Handle incoming peer connection (just read their server ID for now)
async fn handle_incoming_peer(stream: TcpStream) -> io::Result<()> {
    let (reader, _) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let peer_id: u32 = line.trim().parse().unwrap_or(0);
    println!("Accepted peer connection from server {}", peer_id);
    Ok(())
}