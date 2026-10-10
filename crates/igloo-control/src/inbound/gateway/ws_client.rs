//! A minimal WebSocket client for tests: enough of RFC 6455 to open a connection, exchange
//! single-frame text and binary messages, and read a close.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A message received from the server.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Frame {
    Text(String),
    Binary(Vec<u8>),
    Close(u16),
}

/// The server refused the upgrade.
#[derive(Debug)]
pub(super) struct Refused {
    pub(super) status: u16,
    pub(super) body: String,
}

/// An open WebSocket connection.
#[derive(Debug)]
pub(super) struct WsClient {
    stream: TcpStream,
    pending: Vec<u8>,
    /// The subprotocol the server selected.
    pub(super) protocol: Option<String>,
}

impl WsClient {
    const MASK: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Requests `target` with the extra `headers`; the connection if the server switched
    /// protocols, else its refusal.
    pub(super) async fn connect(
        address: SocketAddr,
        target: &str,
        headers: &[(&str, &str)],
    ) -> Result<Self, Refused> {
        let mut stream = TcpStream::connect(address).await.expect("connect");
        let mut request = format!(
            "GET {target} HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n"
        );
        for (name, value) in headers {
            request.extend([name, ": ", value, "\r\n"]);
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.expect("request");

        let mut response = Vec::new();
        let head_end = loop {
            if let Some(end) = response.windows(4).position(|window| window == b"\r\n\r\n") {
                break end + 4;
            }
            let mut chunk = [0u8; 1024];
            let read = tokio::time::timeout(Self::TIMEOUT, stream.read(&mut chunk))
                .await
                .expect("a response within 5 s")
                .expect("read");
            assert!(read > 0, "the server closed before answering");
            response.extend_from_slice(&chunk[..read]);
        };
        let head = String::from_utf8_lossy(&response[..head_end]).into_owned();
        let status: u16 = head
            .split_whitespace()
            .nth(1)
            .and_then(|status| status.parse().ok())
            .expect("a status line");
        if status != 101 {
            let mut body = response[head_end..].to_vec();
            let _ = tokio::time::timeout(Duration::from_millis(300), stream.read_to_end(&mut body))
                .await;
            return Err(Refused {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        let protocol = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("sec-websocket-protocol")
                .then(|| value.trim().to_owned())
        });
        Ok(Self {
            stream,
            pending: response[head_end..].to_vec(),
            protocol,
        })
    }

    pub(super) async fn send_binary(&mut self, data: &[u8]) {
        self.send(0x2, data).await;
    }

    pub(super) async fn send_text(&mut self, text: &str) {
        self.send(0x1, text.as_bytes()).await;
    }

    /// Closes normally, as a browser does when its page goes away.
    pub(super) async fn close(&mut self) {
        self.send(0x8, &1000u16.to_be_bytes()).await;
    }

    async fn send(&mut self, opcode: u8, payload: &[u8]) {
        let mut frame = vec![0x80 | opcode];
        match payload.len() {
            len if len < 126 => frame.push(0x80 | u8::try_from(len).expect("short")),
            len if u16::try_from(len).is_ok() => {
                frame.push(0x80 | 0x7e);
                frame.extend_from_slice(&u16::try_from(len).expect("medium").to_be_bytes());
            }
            len => {
                frame.push(0x80 | 0x7f);
                frame.extend_from_slice(&(len as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(&Self::MASK);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ Self::MASK[index % 4]),
        );
        self.stream.write_all(&frame).await.expect("send");
    }

    /// The next text, binary or close message.
    pub(super) async fn recv(&mut self) -> Frame {
        loop {
            let (opcode, payload) = self.frame().await;
            match opcode {
                0x1 => return Frame::Text(String::from_utf8(payload).expect("utf-8 text")),
                0x2 => return Frame::Binary(payload),
                0x8 => {
                    let code = payload
                        .get(..2)
                        .map_or(1005, |code| u16::from_be_bytes([code[0], code[1]]));
                    return Frame::Close(code);
                }
                _ => {}
            }
        }
    }

    async fn frame(&mut self) -> (u8, Vec<u8>) {
        let head = self.take(2).await;
        let opcode = head[0] & 0x0f;
        let len = match head[1] & 0x7f {
            126 => usize::from(u16::from_be_bytes(
                self.take(2).await.try_into().expect("two"),
            )),
            127 => usize::try_from(u64::from_be_bytes(
                self.take(8).await.try_into().expect("eight"),
            ))
            .expect("fits"),
            len => usize::from(len),
        };
        (opcode, self.take(len).await)
    }

    async fn take(&mut self, count: usize) -> Vec<u8> {
        while self.pending.len() < count {
            let mut chunk = [0u8; 4096];
            let read = tokio::time::timeout(Self::TIMEOUT, self.stream.read(&mut chunk))
                .await
                .expect("a frame within 5 s")
                .expect("read");
            assert!(read > 0, "the connection closed mid-frame");
            self.pending.extend_from_slice(&chunk[..read]);
        }
        self.pending.drain(..count).collect()
    }
}
