// Use AtomicU64 for thread-safe counter
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[derive(Debug)]
pub struct LamportClock {
    counter: AtomicU64,
}

impl LamportClock{
    //Create a new Lamport Clock starting at 0
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }

    /// Increment the clock before sending a message
    /// Rule: C ← C + 1
    /// Returns the NEW incremented value
    pub fn increment(&self)-> u64{
        // fetch_add returns the OLD value, so we add 1 to get the NEW value in line 21
        let old_value = self.counter.fetch_add(1,AtomicOrdering::SeqCst);//AtomicOrdering::SeqCst = "All threads see operations in the same order"
        old_value + 1
    }

    /// Update the clock upon receiving a message with timestamp T_M
    /// Rule: C ← max(C, T_M) + 1
    /// Returns the NEW updated value
    /// This ensures our clock is always ahead of any timestamp we've seen,
    /// maintaining the causality property.
    pub fn update(&self, received_timestamp: u64)-> u64{
        let mut current = self.counter.load(AtomicOrdering::SeqCst);

        loop {
            // Calculate the new value
            let new_value = std::cmp::max(current, received_timestamp) + 1;
            
            // Try to update atomically
            match self.counter.compare_exchange(
                current,                    // Expected value
                new_value,                  // New value to set
                AtomicOrdering::SeqCst,     // Success ordering
                AtomicOrdering::SeqCst,     // Failure ordering
            ) {
                Ok(_) => return new_value,  // Success! Return the new value
                Err(actual) => {
                    // Someone else changed it, retry with the actual value
                    current = actual;
                }
            }
        }
    }

    /// Get the current clock value without modifying it
    pub fn get(&self)-> u64{
        self.counter.load(AtomicOrdering::SeqCst)
    }   
    
    /// Set the clock to a specific value
    pub fn set(&self, value:u64) {
        self.counter.store(value, AtomicOrdering::SeqCst);
    }
}

// Implement Default trait for convenience
impl Default for LamportClock {
    fn default() -> Self {
        Self::new()
    }
}