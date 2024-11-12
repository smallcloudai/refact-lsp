use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::ChildStdin;
use std::time::Instant;
use tracing::error;


pub async fn write_to_stdin_and_flush(stdin: &mut ChildStdin, text_to_write: &str) -> Result<(), String>
{
    stdin.write_all(format!("{}\n", text_to_write).as_bytes()).await.map_err(|e| {
        error!("Failed to write to pdb stdin: {}", e);
        e.to_string()
    })?;
    stdin.flush().await.map_err(|e| {
        error!("Failed to flush pdb stdin: {}", e);
        e.to_string()
    })?;

    Ok(())
}

pub async fn blocking_read_until_token_or_timeout<R>(buffer: &mut R, timeout_ms: u64, token: &str) -> (String, bool)
where
    R: AsyncReadExt + Unpin,
{
    //
    // WARNING: this will block forever if timeout_ms==0 and stream does not end (no EOF)
    //
    // TODO: check what will happen in both stdout and stderr have a lot of data. Will one block the entire process when we're reading the other?
    //
    assert!(timeout_ms > 0);
    let start_time = Instant::now();
    let timeout_duration = tokio::time::Duration::from_millis(timeout_ms);
    let mut output = Vec::new();
    let mut buf = [0u8; 1024];
    let mut have_the_token = false;

    loop {
        if timeout_ms > 0 && start_time.elapsed() >= timeout_duration {
            error!("timeout reached while reading from buffer");
            break;
        }

        let read_result = if timeout_ms > 0 {
            tokio::time::timeout(tokio::time::Duration::from_millis(timeout_ms), buffer.read(&mut buf)).await
        } else {
            Ok(buffer.read(&mut buf).await)
        };

        match read_result {
            Ok(Ok(0)) | Err(_) => break, // End of stream or timeout
            Ok(Ok(bytes_read)) => {
                output.extend_from_slice(&buf[..bytes_read]);
                if !token.is_empty() && output.trim_ascii_end().ends_with(token.as_bytes()) {
                    have_the_token = true;
                    break;
                }
            }
            Ok(Err(e)) => {
                error!("Error reading from buffer: {}", e);
                break;
            }
        }
    }

    (String::from_utf8_lossy(&output).to_string(), have_the_token)
}

pub async fn is_someone_listening_on_that_tcp_port(port: u16, timeout: tokio::time::Duration) -> bool {
    match tokio::time::timeout(timeout, TcpStream::connect(&format!("127.0.0.1:{}", port))).await {
        Ok(Ok(_)) => true,    // Connection successful
        Ok(Err(_)) => false,  // Connection failed, refused
        Err(e) => {  // Timeout occurred
            tracing::error!("Timeout occurred while checking port {}: {}", port, e);
            false             // still no one is listening, as far as we can tell
        }
    }
}

pub fn first_n_chars(msg: &str, n: usize) -> String {
    let mut last_n_chars: String = msg.chars().take(n).collect();
    if last_n_chars.len() == n {
        last_n_chars.push_str("...");
    }
    return last_n_chars;
}

pub fn last_n_chars(msg: &str, n: usize) -> String {
    let mut last_n_chars: String = msg.chars().rev().take(n).collect::<String>().chars().rev().collect();
    if last_n_chars.len() == n {
        last_n_chars.insert_str(0, "...");
    }
    return last_n_chars;
}

pub fn last_n_lines(msg: &str, n: usize) -> String {
    let lines: Vec<&str> = msg.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = if lines.len() > n { lines.len() - n } else { 0 };

    let mut output = if start > 0 { "...\n" } else { "" }.to_string();
    output.push_str(&lines[start..].join("\n"));
    output.push('\n');

    output
}
