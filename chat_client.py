#!/usr/bin/env python3
import socket
import threading
import sys
import time

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 chat_client.py <port> [name]")
        sys.exit(1)
    
    port = int(sys.argv[1])
    
    # Connect to server
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        sock.connect(('127.0.0.1', port))
    except:
        print(f"❌ Could not connect to localhost:{port}")
        sys.exit(1)
    
    # Read welcome message
    welcome = sock.recv(256).decode().strip()
    print(f"✅ {welcome}\n")
    print("=" * 60)
    print("Messages will appear below. Type to send.\n")
    
    # Lock for thread-safe printing
    print_lock = threading.Lock()
    
    # Spawn receiver thread
    def receive():
        while True:
            try:
                msg = sock.recv(1024).decode().strip()
                if not msg:
                    break
                with print_lock:
                    print(f"{msg}")
                    print("> ", end='', flush=True)
            except:
                break
    
    receiver = threading.Thread(target=receive, daemon=True)
    receiver.start()
    
    # Send messages from stdin
    try:
        while True:
            msg = input("> ")
            if msg.strip():
                sock.send((msg + "\n").encode())
    except KeyboardInterrupt:
        sock.close()
        print("\n\n✅ Disconnected")

if __name__ == "__main__":
    main()
