#!/usr/bin/env python3
"""
Distributed Chat Monitor
Connects to all 3 servers and displays messages in real-time with Lamport timestamps
Deduplicates by timestamp only (since all servers deliver the same timestamp)
"""
import socket
import threading
import sys
from datetime import datetime

class ChatMonitor:
    def __init__(self, hide_system=True):
        self.servers = {}
        self.seen_timestamps = set()  # Track only timestamps to avoid duplicates
        self.lock = threading.Lock()
        self.ports = [8081, 8082, 8083]
        self.hide_system = hide_system
    
    def connect_to_server(self, port, server_id):
        """Connect to a server and start reading messages"""
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.connect(('127.0.0.1', port))
            self.servers[server_id] = sock
            sock.recv(256)  # Discard welcome
            
            print(f"✅ Connected to Server {server_id} (port {port})")
            
            threading.Thread(
                target=self.receive_messages,
                args=(server_id, sock),
                daemon=True
            ).start()
        except Exception as e:
            print(f"❌ Failed to connect to Server {server_id}: {e}")
    
    def receive_messages(self, server_id, sock):
        """Receive messages from a server"""
        while True:
            try:
                msg = sock.recv(1024).decode().strip()
                if not msg:
                    break
                
                with self.lock:
                    # Extract timestamp from message (e.g., "[T:74]")
                    if msg.startswith("[T:"):
                        timestamp_end = msg.find("]")
                        timestamp_str = msg[:timestamp_end+1]  # e.g., "[T:74]"
                        content = msg[timestamp_end+2:]        # Rest of message
                        
                        # Extract just the number for deduplication
                        try:
                            timestamp_num = int(timestamp_str[3:-1])
                        except:
                            timestamp_num = None
                        
                        # Filter system messages if requested
                        skip_system = self.hide_system and "SYSTEM:" in content
                        
                        if not skip_system and timestamp_num is not None:
                            # Only display if we haven't seen this timestamp yet
                            if timestamp_num not in self.seen_timestamps:
                                self.seen_timestamps.add(timestamp_num)
                                self.display_message(timestamp_str, content)
                        elif not self.hide_system and timestamp_num is not None:
                            if timestamp_num not in self.seen_timestamps:
                                self.seen_timestamps.add(timestamp_num)
                                self.display_message(timestamp_str, content)
            except:
                break
    
    def display_message(self, timestamp, content):
        """Display a message with formatting"""
        print(f"  {timestamp} | {content}")
    
    def run(self):
        """Main loop"""
        print("\n" + "=" * 80)
        print("DISTRIBUTED CHAT MONITOR - Real-time Message Display")
        print("=" * 80)
        print(f"Time: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n")
        print("Connecting to all servers...\n")
        
        for i, port in enumerate(self.ports, 1):
            self.connect_to_server(port, i)
        
        print("\n" + "-" * 80)
        filter_msg = "(system messages hidden)" if self.hide_system else "(all messages)"
        print(f"Messages (in delivery order) {filter_msg}:\n")
        
        try:
            while True:
                threading.Event().wait(1)
        except KeyboardInterrupt:
            print("\n\n✅ Monitor stopped")
            sys.exit(0)

if __name__ == "__main__":
    # Run with --show-system to include system join/leave messages
    hide_system = "--show-system" not in sys.argv
    monitor = ChatMonitor(hide_system=hide_system)
    monitor.run()
