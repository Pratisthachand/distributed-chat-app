# Distributed Chat Application with Lamport Logical Clocks

A fault-tolerant distributed chat system that demonstrates **Lamport logical clocks** for ordering messages across multiple servers. Built in Rust with async networking (Tokio) and Python client utilities.

## Key Features

- **Lamport Clock Implementation**: Total message ordering across distributed servers
- **Multi-Server Replication**: 3-server hierarchical topology (S1 → S2 → S3)
- **Causality Preservation**: Respects happened-before relationships between events
- **Safe Message Delivery**: Messages only delivered when all servers acknowledge receipt
- **Persistent Storage**: JSON-based message logs with durability guarantees
- **Real-Time Monitoring**: Live message tracking across all servers
- **Network Resilience**: Heartbeat mechanism with peer failure detection (5-second timeout)

## System Architecture

### Lamport Clock Rules

```
On SEND:    C ← C + 1, send message with timestamp
On RECEIVE: C ← max(C, T_message) + 1, add to queue
```

### Server Topology

```
        Client 1 ──────┐
                        │
        Client 2 ───→ Server 1 (port 8081/9001)
                        │ ├──→ Server 2 (port 8082/9002)
        Client 3 ───────┤     └──→ Server 3 (port 8083/9003)
                        │
                    (Replication)
```

- **Server 1**: Primary aggregator (clients 8081, peers 9001)
- **Server 2**: Peer replicator (clients 8082, peers 9002)
- **Server 3**: Leaf peer (clients 8083, peers 9003)
- **Heartbeat**: 100ms interval, 5-second peer timeout

### Safe Delivery Condition

Message is delivered only when:

1. It's at the head of the priority queue (smallest timestamp)
2. All peer servers have acknowledged with a higher timestamp

This ensures:

- Total ordering (all servers deliver messages in same order)
- Causality (message effects visible before message delivery)
- No message loss (requires acknowledgment from all peers)

## Building & Running

### Prerequisites

- **Rust 2021 Edition** (install from [rustup.rs](https://rustup.rs))
- **Python 3.6+** (for client and monitor tools)

### Build

```bash
cd /Users/pratisthachand/Desktop/disChat
cargo build --release
```

### Clean Up (before starting fresh demo)

```bash
# Kill all running servers
killall chat-server 2>/dev/null

# Clear all log files
rm -f server_*.log
```

### Run Servers (3 terminals)

```bash
# Terminal 1: Server 1 (Primary)
./target/release/chat-server 1 8081 9001 127.0.0.1:9002 127.0.0.1:9003

# Terminal 2: Server 2
./target/release/chat-server 2 8082 9002 127.0.0.1:9001 127.0.0.1:9003

# Terminal 3: Server 3
./target/release/chat-server 3 8083 9003 127.0.0.1:9001 127.0.0.1:9002
```

**Arguments**: `chat-server <server_id> <client_port> <peer_port> <peer1_addr> <peer2_addr>`

### Run Monitor (optional, Terminal 4)

Real-time message display with Lamport timestamps:

```bash
python3 monitor.py
```

Output shows messages in total-order delivery sequence:

```
✅ Connected to Server 1 (port 8081)
✅ Connected to Server 2 (port 8082)
✅ Connected to Server 3 (port 8083)

Messages (in delivery order):

  [T:1] | Client 1: Hello everyone
  [T:2] | Client 2: Hi there!
  [T:3] | Client 3: How are you?
```

### Run Clients (3 terminals)

```bash
# Terminal 5: Client 1 (connects to Server 1)
python3 chat_client.py 8081 "Client 1"

# Terminal 6: Client 2 (connects to Server 2)
python3 chat_client.py 8082 "Client 2"

# Terminal 7: Client 3 (connects to Server 3)
python3 chat_client.py 8083 "Client 3"
```

**Usage**: `chat_client.py <port> [custom_name]`

- `port`: Server port (8081, 8082, or 8083)
- `custom_name`: Optional display name (default: assigned server name)

## Verifying System Properties

### 1. Check Total Ordering

All 3 servers should have **identical logs** in the same order:

```bash
cat server_1.log
cat server_2.log
cat server_3.log
```

Expected output (identical on all servers):

```json
{"timestamp":1,"server_id":1,"client_name":"Client 1","content":"Hello everyone","msg_id":1}
{"timestamp":2,"server_id":2,"client_name":"Client 2","content":"Hi there!","msg_id":2}
{"timestamp":3,"server_id":3,"client_name":"Client 3","content":"How are you?","msg_id":3}
```

### 2. Check Clock Progression

Timestamps should increment: `T:1 → T:2 → T:3 → ...`

Each message from any client increments the logical clock of that server, and affects other servers' clocks through the Lamport clock rule.

### 3. Verify Replication

- Send message from Server 1 → appears on all 3 servers
- Send message from Server 2 → appears on all 3 servers
- Send message from Server 3 → appears on all 3 servers

### 4. Verify Durability

Messages persist to disk even after server restart:

```bash
# Stop servers (Ctrl+C in each terminal)
# Restart servers
# Check logs still exist with all previous messages
cat server_1.log | wc -l  # Should show all previous messages
```

## Project Structure

```
disChat/
├── Cargo.toml                 # Rust project config & dependencies
├── README.md                  # This file
├── server/
│   └── main.rs               # Lamport clock implementation, peer replication
├── client/
│   └── main.rs               # Rust client (Tokio-based)
├── chat_client.py            # Python client with custom names
├── monitor.py                # Real-time message monitor
├── server_1.log              # Persistent message log for Server 1
├── server_2.log              # Persistent message log for Server 2
├── server_3.log              # Persistent message log for Server 3
└── target/
    └── release/
        └── chat-server       # Compiled binary
```

## 🔧 Implementation Details

### Lamport Clock Algorithm

```rust
// On sending a message from server S
timestamp = local_clock;
local_clock += 1;
broadcast(message, timestamp);

// On receiving a message from peer P
received_timestamp = message.timestamp;
local_clock = max(local_clock, received_timestamp) + 1;
queue.insert(message);  // Add to priority queue
```

### Message Delivery Queue

- **Data Structure**: `BinaryHeap<Reverse<Message>>`
- **Key**: Lamport timestamp (min-heap for FIFO ordering)
- **Delivery Rule**: Message at front only delivered when all peers have acknowledged with **higher** timestamp

### Replication Topology

- **Server 1** replicates to Server 2 and Server 3
- **Server 2** replicates to Server 3 only
- **Server 3** doesn't replicate (leaf node)
- **Acknowledgments** flow back up the tree

This ensures:

- No circular replication
- All servers eventually receive all messages
- Latency minimized (hierarchical delivery)

### Persistence

- **Format**: JSON (one message per line)
- **File**: `server_X.log` (X = server ID)
- **Fsync**: Each write forced to disk with `fsync()` for durability
- **Recovery**: On restart, server replays all logs

## Key Files

| File             | Purpose                                                   |
| ---------------- | --------------------------------------------------------- |
| `server/main.rs` | Lamport clock logic, peer replication, message queue      |
| `chat_client.py` | Python client with proper TCP buffering & name display    |
| `monitor.py`     | Real-time monitor showing messages with timestamps        |
| `server_X.log`   | Persistent JSON logs proving total ordering & replication |

## License

This is an educational project demonstrating distributed systems concepts.

---

**Author**: Pratistha Chand  
**Date**: December 2025  
**Project**: Distributed Chat with Lamport Logical Clocks
