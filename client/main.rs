use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt},
    net::TcpStream,
};
use std::process;

const SERVER_ADDR: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() -> io::Result<()> {
    println!("Attempting to connect to server at {}...", SERVER_ADDR);
    
    //Establish the connection
    let stream = match TcpStream::connect(SERVER_ADDR).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to server: {}", e);
            process::exit(1);
        }
    };
    println!("Successfully connected! Start typing.");

    //Split the stream for concurrent reading and writing
    let (reader, mut writer) = stream.into_split();
    let mut reader = io::BufReader::new(reader);

    //Task A: Read messages from the server and print to stdout
    let server_to_stdout = tokio::spawn(async move {
        let mut line = String::new();
        loop {
            //Read a line from the server
            match reader.read_line(&mut line).await {
                Ok(0) => { //Connection closed by server
                    println!("\n[Server disconnected. Press Enter to exit.]");
                    break;
                }
                Ok(_) => {
                    print!("{}", line); 
                    line.clear();
                }
                Err(e) => {
                    eprintln!("Error reading from server: {}", e);
                    break;
                }
            }
        }
    });

    //Task B: Read user input from stdin and send it to the server
    let stdin_to_server = tokio::spawn(async move {
        let mut stdin = io::BufReader::new(io::stdin());
        let mut line = String::new();
        loop {
            //Read a line from the user
            if let Err(e) = stdin.read_line(&mut line).await {
                eprintln!("Error reading stdin: {}", e);
                break;
            }
            
            //Send the line to the server
            if let Err(e) = writer.write_all(line.as_bytes()).await {
                eprintln!("Error writing to server: {}", e);
                break;
            }
            line.clear();
        }
    });

    //Wait for either task to finish (e.g., server disconnects or user quits)
    tokio::select! {
        _ = server_to_stdout => {},
        _ = stdin_to_server => {},
    }

    Ok(())
}