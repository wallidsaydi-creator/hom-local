use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgressEvent {
    Token { seq: u64, text: String },
    SwitchingProvider { provider_id: String },
    Error { code: String, message: String },
}

#[derive(Clone)]
pub struct ProgressSink {
    tx: mpsc::Sender<ProgressEvent>,
}

impl ProgressSink {
    pub fn new(tx: mpsc::Sender<ProgressEvent>) -> Self {
        Self { tx }
    }

    pub async fn token(&self, seq: u64, text: impl Into<String>) {
        let _ = self
            .tx
            .send(ProgressEvent::Token {
                seq,
                text: text.into(),
            })
            .await;
    }

    pub async fn error(&self, code: impl Into<String>, message: impl Into<String>) {
        let _ = self
            .tx
            .send(ProgressEvent::Error {
                code: code.into(),
                message: message.into(),
            })
            .await;
    }
}

pub fn parse_sse_lines(buffer: &mut String, chunk: &str) -> Vec<String> {
    buffer.push_str(chunk);
    let mut events = Vec::new();
    while let Some(pos) = buffer.find("\n\n") {
        let raw = buffer[..pos].to_string();
        buffer.drain(..pos + 2);
        for line in raw.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("data:") {
                let data = rest.trim();
                if !data.is_empty() {
                    events.push(data.to_string());
                }
            }
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_data_frames() {
        let mut buffer = String::new();
        let events = parse_sse_lines(&mut buffer, "data: {\"x\":1}\n\ndata: [DONE]\n\n");
        assert_eq!(events, vec![r#"{"x":1}"#, "[DONE]"]);
    }
}
