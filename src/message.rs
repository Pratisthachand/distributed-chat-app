use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// A chat message with Lamport timestamp for total ordering
/// 
/// This struct represents a single message in the distributed chat system.
/// Each message carries all the metadata needed for:
/// 1. Total ordering (timestamp + server_id)
/// 2. Display (client_name + content)
/// 3. Tracking (msg_id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Lamport timestamp (T_M) - the logical time when this message was created
    pub timestamp: u64,
    
    /// ID of the server that first received this message
    /// Used as a tiebreaker when timestamps are equal
    pub server_id: u32,
    
    /// Name of the client who sent the message
    pub client_name: String,
    
    /// The actual message content
    pub content: String,
    
    /// Unique message ID within this server (for additional tracking)
    pub msg_id: u64,
}

impl Message {
    /// Create a new message
    /// # Example
    /// ```
    /// let msg = Message::new(
    ///     42,                          // timestamp from Lamport clock
    ///     1,                           // server 1
    ///     "Alice".to_string(),         // client name
    ///     "Hello World!".to_string(),  // message content
    ///     1                            // first message from this server
    /// );
    pub fn new(
        timestamp: u64,
        server_id: u32,
        client_name: String,
        content: String,
        msg_id: u64,
    ) -> Self {
        Self {
            timestamp,
            server_id,
            client_name,
            content,
            msg_id,
        }
    }
    
    /// Format the message for display to clients
    /// Returns: "[T:timestamp] client_name: content"
    pub fn format_for_display(&self) -> String {
        format!("[T:{}] {}: {}", self.timestamp, self.client_name, self.content)
    }
}

// Implement equality based on timestamp and server_id
// Two messages are equal if they have the same timestamp AND server_id
impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp && self.server_id == other.server_id
    }
}

impl Eq for Message {}

// Implement ordering for messages
// This is CRITICAL for the pending message queue!
impl PartialOrd for Message {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Message {
    /// Compare messages for ordering
    /// 
    /// Ordering rules:
    /// 1. Primary: Compare by timestamp (lower timestamp comes first)
    /// 2. Tiebreaker: If timestamps are equal, compare by server_id
    /// 
    /// This ensures a deterministic total ordering even when
    /// multiple servers assign the same Lamport timestamp.
    
    fn cmp(&self, other: &Self) -> Ordering {
        // First compare by timestamp
        match self.timestamp.cmp(&other.timestamp) {
            Ordering::Equal => {
                // If timestamps are equal, use server_id as tiebreaker
                // This ensures deterministic ordering
                self.server_id.cmp(&other.server_id)
            }
            other_ordering => other_ordering,
        }
    }
}