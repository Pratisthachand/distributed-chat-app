use tokio::{
    net::{TcpListener, TcpStream},
    sync::broadcast,
    io::{self, AsyncBufReadExt, AsyncWriteExt},
};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

const CHANNEL_SIZE: usize = 32;
const SERVER_ADDR: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() -> io::Result<()> {
    let (tx, _rx) = broadcast::channel::<String>(CHANNEL_SIZE);

    let client_counter = Arc::new(AtomicUsize::new(1));

    let listener = TcpListener::bind(SERVER_ADDR).await?;
    println!("Chat Server running on {}", SERVER_ADDR);

    loop {
        //Wait for a new client connection
        let (stream, addr) = listener.accept().await?;
        println!("New connection from {}", addr);

        //Get a unique ID for this client and clone the necessary handles.
        let tx_clone = tx.clone();
        let rx_clone = tx.subscribe();
        
        //Atomically increment the counter and get the ID before spawning.
        let client_id = client_counter.fetch_add(1, Ordering::Relaxed);

        //Spawn an asynchronous task to handle this client
        tokio::spawn(async move {
            // Pass the unique client_id to the handler
            if let Err(e) = handle_client(stream, tx_clone, rx_clone, client_id).await {
                eprintln!("Error handling client {}: {:?}", addr, e);
            }
            println!("Client {} disconnected.", addr);
        });
    }
}

async fn handle_client(
    stream: TcpStream,
    tx: broadcast::Sender<String>,
    mut rx: broadcast::Receiver<String>,
    client_id: usize,
) -> io::Result<()> {
    
    let (reader, mut writer) = stream.into_split();

    let client_name = format!("Client #{}", client_id); 
    let tx_for_reader = tx.clone();
    let client_name_for_reader = client_name.clone(); 
    
    //Announce the new user to everyone
    let join_msg = format!(">>> {} has joined the chat!", client_name);
    let _ = tx.send(join_msg); 

    //Task A: Read messages from this client and broadcast them to all other clients
    let mut line_reader = tokio::io::BufReader::new(reader); 
    let mut line = String::new();
    
    let reader_task = tokio::spawn(async move {
        loop {
            line.clear();
            match line_reader.read_line(&mut line).await {
                Ok(0) => return Ok(()), 
                Ok(_) => {
                    let chat_msg = format!("{}: {}", client_name_for_reader, line.trim()); 
                    let _ = tx_for_reader.send(chat_msg); 
                    line.clear();
                }
                Err(e) => return Err(e), 
            }
        }
    });

    //Task B: Read messages from the broadcast channel and write them to this client
    let writer_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let full_msg = format!("{}\n", msg);
                    if let Err(e) = writer.write_all(full_msg.as_bytes()).await {
                        eprintln!("Write error: {}", e);
                        break; 
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    eprintln!("Client lagged and missed {} messages.", missed);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });
    
    //Wait for either the reader or writer task to complete
    tokio::select! {
        res = reader_task => {
            if let Ok(Err(e)) = res {
                eprintln!("Reader error: {}", e);
            }
        }
        _ = writer_task => {}
    }
    
    //Announce the user is leaving
    let leave_msg = format!("<<< {} has left the chat.", client_name);
    let _ = tx.send(leave_msg); 

    Ok(())
}