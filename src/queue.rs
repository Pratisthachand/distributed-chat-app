use crate::Message;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use tokio::sync::RwLock;

/// Information about a peer server for tracking purposes
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub id: u32,
    pub address: String,
}

impl PeerInfo {
    pub fn new(id: u32, address: String) -> Self {
        Self { id, address }
    }
}

/// Holds messages until we're sure they can be delivered in the correct order. 
/// Safe delivery rule: A message with timestamp T_M can be delivered when:
/// 1. It has the smallest timestamp in the queue (first in line - thus oldest)
/// 2. We've received something (message or heartbeat) from ALL other servers
///    with timestamp > T_M (i.e. a newer timestamp)
/// This way, we know no earlier message is still on its way

pub struct PendingMessageQueue {
    /// Min-heap of messages sorted by Lamport timestamp
    queue: RwLock<BinaryHeap<Reverse<Message>>>,
    
    /// Track the latest timestamp seen from each peer server
    /// Key: peer server ID
    /// Value: latest timestamp from that peer
    peer_timestamps: RwLock<HashMap<u32, u64>>,
    
    /// IDs of all peer servers we know about
    known_peers: Vec<u32>,
    
    /// Our own server ID (we don't track ourselves)
    server_id: u32,
}

impl PendingMessageQueue {
    /// Create a new queue for holding messages until they're ready to deliver
    pub fn new(server_id: u32, peers: Vec<PeerInfo>) -> Self {
        let known_peers: Vec<u32> = peers.iter().map(|p| p.id).collect();
        let mut peer_timestamps = HashMap::new();
        
        // Start tracking all peers at timestamp 0
        for peer_id in &known_peers {
            peer_timestamps.insert(*peer_id, 0);
        }
        
        Self {
            queue: RwLock::new(BinaryHeap::new()),
            peer_timestamps: RwLock::new(peer_timestamps),
            known_peers,
            server_id,
        }
    }

    /// Add a message to the queue (it'll wait here until safe to deliver)
    pub async fn enqueue(&self, message: Message) {
        let mut queue = self.queue.write().await;
        queue.push(Reverse(message));
    }

    /// Update what we've last heard from a peer (from either a message or heartbeat)
    pub async fn update_peer_timestamp(&self, peer_id: u32, timestamp: u64) {
        let mut timestamps = self.peer_timestamps.write().await;
        timestamps
            .entry(peer_id)
            .and_modify(|t| *t = (*t).max(timestamp))  // Keep the maximum
            .or_insert(timestamp);
    }

    /// Check if it's safe to deliver a message with this timestamp.
    /// We need all peers to have moved past it first.
    async fn can_deliver(&self, msg_timestamp: u64) -> bool {
        let timestamps = self.peer_timestamps.read().await;
        
        // Check that ALL peers have sent something with a higher timestamp
        for peer_id in &self.known_peers {
            if let Some(&peer_ts) = timestamps.get(peer_id) {
                if peer_ts <= msg_timestamp {
                    return false;  // This peer might still have older messages coming
                }
            } else {
                return false;  // Haven't heard from this peer yet
            }
        }
        
        true // All clear
    }
    
    /// Deliver all messages that are ready, in the correct order
    pub async fn try_deliver(&self) -> Vec<Message> {
        let mut delivered = Vec::new();
        
        loop {
            // Check the oldest message
            let queue = self.queue.read().await;
            if queue.is_empty() {
                break;
            }
            
            let next_msg = queue.peek().unwrap();
            let msg_timestamp = next_msg.0.timestamp;
            drop(queue);
            
            // Can we deliver it?
            if self.can_deliver(msg_timestamp).await {
                let mut queue = self.queue.write().await;
                if let Some(Reverse(msg)) = queue.pop() {
                    delivered.push(msg);
                }
            } else {
                break;  // If we can't deliver this one, we can't deliver any after it
            }
        }
        
        delivered
    }

    /// Stop waiting for a crashed peer (set its timestamp to infinity)
    pub async fn mark_peer_failed(&self, peer_id: u32) {
        let mut timestamps = self.peer_timestamps.write().await;
        timestamps.insert(peer_id, u64::MAX);
    }

    /// Resume tracking a peer that came back online
    pub async fn restore_peer(&self, peer_id: u32, last_timestamp: u64) {
        let mut timestamps = self.peer_timestamps.write().await;
        timestamps.insert(peer_id, last_timestamp);
    }

    /// How many messages are waiting?
    pub async fn pending_count(&self) -> usize {
        let queue = self.queue.read().await;
        queue.len()
    }

    /// Get current timestamp state for all peers (useful for debugging)
    pub async fn get_peer_timestamps(&self) -> HashMap<u32, u64> {
        let timestamps = self.peer_timestamps.read().await;
        timestamps.clone()
    }
}