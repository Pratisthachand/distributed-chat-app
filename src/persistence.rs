use crate::Message;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

/// Saves messages to disk so they survive server crashes
pub struct MessageLog {
    file_path: String,
}

impl MessageLog {
    /// Create a new message log (creates the file if it doesn't exist)
    pub fn new(file_path: String) -> io::Result<Self> {
        // Open/create the file immediately to verify we have write permissions
        // Better to fail now than when we try to save our first message
        OpenOptions::new()
            .create(true)    // Create if doesn't exist
            .append(true)    // Don't overwrite existing data
            .open(&file_path)?;
        
        Ok(Self { file_path })
    }

    /// Write a message to the log file
    /// WHY: Every message must be written to disk BEFORE we add it to the pending queue.
    /// This way, if the server crashes, we can replay the log and restore our state.
    pub fn append(&self, message: &Message) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;
        
        // Convert message to JSON and write it
        let json = serde_json::to_string(message)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        
        writeln!(file, "{}", json)?;
        file.sync_all()?;  // Force write to disk immediately - without this, the message might sit in memory and be lost if we crash
        
        Ok(())
    }

    /// Read all messages from the log (used when server restarts) and returns (messages, highest_timestamp)
    /// WHY: When a server crashes and restarts, we need to:
    /// 1. Restore all the messages we had
    /// 2. Find the highest timestamp to restore our Lamport clock
    pub fn replay(&self) -> io::Result<(Vec<Message>, u64)> {
        let mut messages = Vec::new();
        let mut max_timestamp = 0u64;
        
        if !Path::new(&self.file_path).exists() {
            return Ok((messages, max_timestamp));
        }
        
        let file = File::open(&self.file_path)?;
        let reader = BufReader::new(file);
        
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue; // Skip blank lines
            }
            
            // Try to parse the JSON back into a Message
            match serde_json::from_str::<Message>(&line) {
                Ok(message) => {
                    //We'll restore the Lamport clock to the highest timestamp we've seen
                    max_timestamp = max_timestamp.max(message.timestamp);
                    messages.push(message);
                }
                Err(e) => {
                    // Don't crash if one line is corrupted - just skip it
                    eprintln!("Warning: Couldn't read message: {}", e);
                }
            }
        }
        
        Ok((messages, max_timestamp))
    }

    /// Clear the log file (for testing)
    pub fn clear(&self) -> io::Result<()> {
        File::create(&self.file_path)?;
        Ok(())
    }
}