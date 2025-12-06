#!/usr/bin/env python3
import socket
import threading
import sys
import time

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 chat_client.py <port> [custom_name]")
        sys.exit(1)
    
    port = int(sys.argv[1])
    custom_name = sys.argv[2] if len(sys.argv) > 2 else None
    
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
    
    # Extract client name from welcome (e.g., "Welcome Client#0@S1!")
    client_name = "Unknown"
    if "Client#" in welcome:
        start = welcome.find("Client#")
        end = welcome.find("!", start)
        if start >= 0 and end > start:
            client_name = welcome[start:end]
    
    if custom_name:
        display_name = custom_name
    else:
        display_name = client_name
    
    print(f"📤 Your name: {display_name}")
    print("=" * 60)
    print("Messages will appear below. Type to send.\n")
    
    # Lock for thread-safe printing
    print_lock = threading.Lock()
    
    # Spawn receiver thread
    def receive():
        buffer = b""
        while True:
            try:
                chunk = sock.recv(1024)
                if not chunk:
                    break
                buffer += chunk
                
                # Process complete lines
                while b"\n" in buffer:
                    line, buffer = buffer.split(b"\n", 1)
                    msg = line.decode().strip()
                    if msg:
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
                sock.sendall((msg + "\n").encode())
    except KeyboardInterrupt:
        print("\n✅ Disconnected")
    finally:
        sock.close()

if __name__ == "__main__":
    main()
