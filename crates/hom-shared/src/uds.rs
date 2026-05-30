use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::types::{JsonRpcRequest, JsonRpcResponse};

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub async fn write_json_line<T: Serialize>(
    writer: &mut OwnedWriteHalf,
    value: &T,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

pub async fn read_json_line<T: DeserializeOwned>(
    reader: &mut BufReader<OwnedReadHalf>,
) -> std::io::Result<Option<T>> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "UDS JSON-RPC frame exceeds 8MiB",
        ));
    }
    Ok(Some(
        serde_json::from_str(line.trim_end()).map_err(std::io::Error::other)?,
    ))
}

pub async fn send_request(
    socket_path: impl AsRef<std::path::Path>,
    request: &JsonRpcRequest,
) -> std::io::Result<JsonRpcResponse> {
    let stream = UnixStream::connect(socket_path).await?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    write_json_line(&mut write_half, request).await?;
    read_json_line(&mut reader)
        .await?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "empty response"))
}
