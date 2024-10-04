use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::ChildStdin;
use tokio::time::{timeout, Duration};
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

pub async fn read_until_token_or_timeout<R>(buffer: &mut R, timeout_ms: u64, token: &str) -> Result<String, String>
where
    R: AsyncReadExt + Unpin,
{
    let mut output = String::new();
    let mut buf = [0u8; 1024];

    loop {
        let read_result = if timeout_ms > 0 {
            timeout(Duration::from_millis(timeout_ms), buffer.read(&mut buf)).await
        } else {
            Ok(buffer.read(&mut buf).await)
        };

        let bytes_read = match read_result {
            Ok(Ok(bytes)) => bytes,                      // Successfully read
            Ok(Err(e)) => return Err(e.to_string()),     // Read error
            Err(_) => return Ok(output),                 // Timeout, return current output
        };

        if bytes_read == 0 { break; }
        
        output.push_str(&String::from_utf8_lossy(&buf[..bytes_read]));

        if !token.is_empty() && output.trim_end().ends_with(token) { break; }
    }

    Ok(output)
}

pub fn first_n_chars(msg: &String, n: usize) -> String {
    let mut last_n_chars: String = msg.chars().take(n).collect();
    if last_n_chars.len() == n {
        last_n_chars.push_str("...");
    }
    return last_n_chars;
}

pub fn last_n_lines(msg: &String, n: usize) -> String 
{
    let lines = msg.lines();
    let mut lines_iter = lines.rev().take(n);
    let mut output = String::new();
    while let Some(line) = lines_iter.next() {
        output.push_str(line);
        output.push('\n');
    }
    output
}