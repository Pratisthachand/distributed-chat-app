use crate::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, RwLock, mpsc};
use tokio::time;

/// Messages that servers send to each other
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerMessage {
    /// Replicate a chat message to all other server so that all servers have the same message history
    ReplicateMessage(Message),
    
    /// Heartbeat with current Lamport timestamp
    /// This lets peers know we're alive and updates their pending queue timestamps
    Heartbeat { 
        server_id: u32, 
        timestamp: u64 
    },
    
    /// Request message history (used after crash recovery to catch up on messages it missed)
    RequestHistory { 
        server_id: u32, 
        last_known_timestamp: u64 
    },
    
    /// Response with message history
    HistoryResponse { 
        messages: Vec<Message> 
    },
}

/// Status of a peer server
#[derive(Debug, Clone, PartialEq)]
pub enum PeerStatus {
    Active,   // Receiving heartbeats normally
    Failed,   // Haven't heard from them in 5+ seconds
}

/// Tracks state of one peer server
#[derive(Debug, Clone)]
pub struct PeerState {
    pub id: u32,
    pub address: String,
    pub status: PeerStatus,
    pub last_heartbeat: Instant,      // When did we last hear from them?
    pub last_timestamp: u64,          // What was their latest Lamport timestamp?
}

/// Manages all connections to peer servers
/// We need this to:
/// - Keep TCP connections alive to all peers
/// - Broadcast messages when we receive them from clients
/// - Detect when peers crash
/// - Receive messages from peers
pub struct PeerManager {
    server_id: u32,
    
    /// Current state of all peers
    peers: Arc<RwLock<HashMap<u32, PeerState>>>,
    
    /// Channel for sending messages TO peers
    /// WHY: When we want to broadcast, we send here and all peer connections receive it
    outbound_tx: broadcast::Sender<PeerMessage>,
    
    /// Channel for receiving messages FROM peers
    /// WHY: When a peer sends us something, it comes through here
    inbound_tx: mpsc::UnboundedSender<PeerMessage>,
    
    heartbeat_interval: Duration,
    heartbeat_timeout: Duration,
}

impl PeerManager {
    /// Create a new peer manager
    pub fn new(
        server_id: u32,
        peers: Vec<crate::queue::PeerInfo>,
        heartbeat_interval_ms: u64,
        heartbeat_timeout_ms: u64,
    ) -> (Self, mpsc::UnboundedReceiver<PeerMessage>) {
        let (outbound_tx, _) = broadcast::channel(100);
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        
        let mut peer_map = HashMap::new();
        for peer in peers {
            peer_map.insert(
                peer.id,
                PeerState {
                    id: peer.id,
                    address: peer.address,
                    status: PeerStatus::Active,
                    last_heartbeat: Instant::now(),
                    last_timestamp: 0,
                },
            );
        }
        
        (
            Self {
                server_id,
                peers: Arc::new(RwLock::new(peer_map)),
                outbound_tx,
                inbound_tx,
                heartbeat_interval: Duration::from_millis(heartbeat_interval_ms),
                heartbeat_timeout: Duration::from_millis(heartbeat_timeout_ms),
            },
            inbound_rx,
        )
    }

    /// Start connecting to all peers and begin heartbeat/failure detection
    /// WHY: This spawns background tasks that:
    /// 1. Connect to each peer (and keep reconnecting if they crash)
    /// 2. Send heartbeats every second
    /// 3. Check for peer failures every second
    pub async fn start(&self) {
        let peers = self.peers.read().await;
        let peer_list: Vec<(u32, String)> = peers
            .values()
            .map(|p| (p.id, p.address.clone()))
            .collect();
        drop(peers);
        
        // Connect to each peer
        for (peer_id, address) in peer_list {
            self.connect_to_peer(peer_id, address).await;
        }
        
        // Start heartbeat sender
        self.start_heartbeat_sender();
        
        // Start failure detector
        self.start_failure_detector();
    }

    /// Connect to one peer (runs in background, auto-reconnects)
    async fn connect_to_peer(&self, peer_id: u32, address: String) {
        let server_id = self.server_id;
        let peers = Arc::clone(&self.peers);
        let outbound_rx = self.outbound_tx.subscribe();
        let inbound_tx = self.inbound_tx.clone();
        
        tokio::spawn(async move {
            loop {
                // Try to connect
                match TcpStream::connect(&address).await {
                    Ok(stream) => {
                        println!("[Server {}] Connected to peer {} at {}", 
                            server_id, peer_id, address);
                        
                        // Handle this connection
                        if let Err(e) = Self::handle_peer_connection(
                            stream,
                            server_id,
                            peer_id,
                            Arc::clone(&peers),
                            outbound_rx.resubscribe(),
                            inbound_tx.clone(),
                        ).await {
                            eprintln!("[Server {}] Peer {} error: {}", server_id, peer_id, e);
                        }
                    }
                    Err(e) => {
                        eprintln!("[Server {}] Can't connect to peer {}: {}", 
                            server_id, peer_id, e);
                    }
                }
                
                // Wait 3 seconds before reconnecting
                println!("[Server {}] Reconnecting to peer {} in 3s...", server_id, peer_id);
                time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    /// Handle one TCP connection to a peer
    /// This runs two tasks:
    /// 1. Reader: Receives messages from peer → sends to inbound channel
    /// 2. Writer: Receives from outbound channel → sends to peer
    async fn handle_peer_connection(
        stream: TcpStream,
        server_id: u32,
        peer_id: u32,
        peers: Arc<RwLock<HashMap<u32, PeerState>>>,
        mut outbound_rx: broadcast::Receiver<PeerMessage>,
        inbound_tx: mpsc::UnboundedSender<PeerMessage>,
    ) -> std::io::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        
        // Send our server ID first so peer knows who we are
        let id_msg = format!("{}\n", server_id);
        writer.write_all(id_msg.as_bytes()).await?;
        
        // Spawn reader task (receives from peer)
        let inbound_tx_clone = inbound_tx.clone();
        let peers_clone = Arc::clone(&peers);
        let reader_task = tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,  // Connection closed
                    Ok(_) => {
                        // Parse the JSON message
                        if let Ok(msg) = serde_json::from_str::<PeerMessage>(line.trim()) {
                            // Update heartbeat time if it's a heartbeat
                            if let PeerMessage::Heartbeat { timestamp, .. } = &msg {
                                let mut peers = peers_clone.write().await;
                                if let Some(peer) = peers.get_mut(&peer_id) {
                                    peer.last_heartbeat = Instant::now();
                                    peer.last_timestamp = *timestamp;
                                    peer.status = PeerStatus::Active;
                                }
                            }
                            
                            // Send to main server logic
                            let _ = inbound_tx_clone.send(msg);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        
        // Spawn writer task (sends to peer)
        let writer_task = tokio::spawn(async move {
            while let Ok(msg) = outbound_rx.recv().await {
                // Convert to JSON and send
                let json = serde_json::to_string(&msg).unwrap();
                let line = format!("{}\n", json);
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        
        // Wait for either task to finish (connection dies)
        tokio::select! {
            _ = reader_task => {},
            _ = writer_task => {},
        }
        
        println!("[Server {}] Lost connection to peer {}", server_id, peer_id);
        Ok(())
    }

    /// Start sending heartbeats every second
    fn start_heartbeat_sender(&self) {
        let server_id = self.server_id;
        let outbound_tx = self.outbound_tx.clone();
        let interval = self.heartbeat_interval;
        
        tokio::spawn(async move {
            let mut ticker = time::interval(interval);
            loop {
                ticker.tick().await;
                
                // Send heartbeat (timestamp will be filled by server)
                let heartbeat = PeerMessage::Heartbeat {
                    server_id,
                    timestamp: 0,  // Server will update with actual Lamport clock
                };
                
                let _ = outbound_tx.send(heartbeat);
            }
        });
    }

    /// Check for peer failures every second
    fn start_failure_detector(&self) {
        let peers = Arc::clone(&self.peers);
        let timeout = self.heartbeat_timeout;
        let server_id = self.server_id;
        
        tokio::spawn(async move {
            let mut ticker = time::interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                
                let mut peers = peers.write().await;
                let now = Instant::now();
                
                for (peer_id, peer) in peers.iter_mut() {
                    if peer.status == PeerStatus::Active {
                        // Check if we haven't heard from them
                        if now.duration_since(peer.last_heartbeat) > timeout {
                            println!("[Server {}] Peer {} marked as FAILED (timeout)", 
                                server_id, peer_id);
                            peer.status = PeerStatus::Failed;
                        }
                    }
                }
            }
        });
    }

    /// Broadcast a message to all peers
    /// WHY: When we receive a message from a client or need to send a heartbeat,
    /// we call this to send it to everyone
    pub fn broadcast(&self, message: PeerMessage) {
        let _ = self.outbound_tx.send(message);
    }

    /// Get status of all peers
    pub async fn get_peer_statuses(&self) -> HashMap<u32, PeerStatus> {
        let peers = self.peers.read().await;
        peers.iter().map(|(id, state)| (*id, state.status.clone())).collect()
    }

    /// Get latest timestamp from each peer
    /// WHY: Used by the pending queue for safe delivery checks
    pub async fn get_peer_timestamps(&self) -> HashMap<u32, u64> {
        let peers = self.peers.read().await;
        peers.iter().map(|(id, state)| (*id, state.last_timestamp)).collect()
    }
}