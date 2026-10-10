use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use igloo_api::problem::Problem;
use igloo_api::terminal::{
    TerminalClientFrame, TerminalEndReason, TerminalRequest, TerminalServerFrame,
};
use reqwest::Url;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::client::Error;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// An open terminal in a sandbox: the process's output arrives from [`TerminalOutput`], and
/// [`TerminalInput`] types into it and resizes its screen.
///
/// Invariant: the terminal's process ends when the terminal is dropped or closed.
#[derive(Debug)]
pub struct Terminal {
    input: TerminalInput,
    output: TerminalOutput,
}

/// The sending half of a [`Terminal`].
#[derive(Debug)]
pub struct TerminalInput {
    sink: SplitSink<Socket, Message>,
}

/// The receiving half of a [`Terminal`].
#[derive(Debug)]
pub struct TerminalOutput {
    stream: SplitStream<Socket>,
    exited: bool,
}

/// What a terminal produced.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TerminalEvent {
    /// Bytes the process printed.
    Output(Vec<u8>),
    /// The process ended; the last event.
    Exit(TerminalExit),
}

/// How a terminal's process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TerminalExit {
    /// The process exited with this code (128 plus the signal when a signal killed it).
    Code(i32),
    /// The terminal ended without an exit code.
    Failed(TerminalEndReason),
}

impl Terminal {
    pub(crate) async fn connect(
        mut url: Url,
        token: &str,
        command: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<Self, Error> {
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|()| Error::Protocol("the API URL cannot carry a WebSocket".to_owned()))?;
        {
            let mut query = url.query_pairs_mut();
            for argument in command {
                query.append_pair("command", argument);
            }
            query.append_pair("cols", &cols.to_string());
            query.append_pair("rows", &rows.to_string());
        }
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| Error::Protocol(error.to_string()))?;
        let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|error| Error::Protocol(error.to_string()))?;
        request.headers_mut().insert(AUTHORIZATION, bearer);
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static(TerminalRequest::PROTOCOL),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|error| Self::refused(&error))?;
        let (sink, stream) = socket.split();
        Ok(Self {
            input: TerminalInput { sink },
            output: TerminalOutput {
                stream,
                exited: false,
            },
        })
    }

    /// The error for a connection that did not open: the server's problem when it refused the
    /// upgrade with one.
    fn refused(error: &tungstenite::Error) -> Error {
        if let tungstenite::Error::Http(response) = error {
            let problem = response
                .body()
                .as_deref()
                .and_then(|body| serde_json::from_slice::<Problem>(body).ok());
            if let Some(problem) = problem {
                return Error::Api(Box::new(problem));
            }
            return Error::Protocol(format!("the terminal was refused: {}", response.status()));
        }
        Error::Protocol(error.to_string())
    }

    /// Splits the terminal so input and output can be driven at once.
    #[must_use]
    pub fn split(self) -> (TerminalInput, TerminalOutput) {
        (self.input, self.output)
    }
}

impl TerminalInput {
    /// Types `bytes` into the process.
    pub async fn send(&mut self, bytes: Vec<u8>) -> Result<(), Error> {
        self.write(Message::Binary(bytes.into())).await
    }

    /// Tells the process its screen is now `cols` by `rows` characters, each 1 to 65535.
    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        let frame = TerminalClientFrame::Resize { cols, rows };
        let json =
            serde_json::to_string(&frame).map_err(|error| Error::Protocol(error.to_string()))?;
        self.write(Message::Text(json.into())).await
    }

    /// Closes the terminal; its process is killed unless it ended already.
    pub async fn close(&mut self) -> Result<(), Error> {
        self.sink
            .close()
            .await
            .map_err(|error| Error::Protocol(error.to_string()))
    }

    async fn write(&mut self, message: Message) -> Result<(), Error> {
        self.sink
            .send(message)
            .await
            .map_err(|error| Error::Protocol(error.to_string()))
    }
}

impl TerminalOutput {
    /// The next thing the process produced. `None` once the terminal ended: after an
    /// [`TerminalEvent::Exit`], or when the server closed the connection without one.
    pub async fn next(&mut self) -> Result<Option<TerminalEvent>, Error> {
        if self.exited {
            return Ok(None);
        }
        while let Some(message) = self.stream.next().await {
            match message.map_err(|error| Error::Protocol(error.to_string()))? {
                Message::Binary(data) => return Ok(Some(TerminalEvent::Output(data.to_vec()))),
                Message::Text(text) => {
                    let frame: TerminalServerFrame = serde_json::from_str(text.as_str())
                        .map_err(|error| Error::Protocol(error.to_string()))?;
                    let TerminalServerFrame::Exit { code, failure } = frame else {
                        continue;
                    };
                    self.exited = true;
                    let exit = match (code, failure) {
                        (Some(code), _) => TerminalExit::Code(code),
                        (None, Some(reason)) => TerminalExit::Failed(reason),
                        (None, None) => TerminalExit::Failed(TerminalEndReason::Lost),
                    };
                    return Ok(Some(TerminalEvent::Exit(exit)));
                }
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

    use super::*;
    use crate::Client;

    /// What the fake server saw of the upgrade request.
    #[derive(Default)]
    struct Seen {
        uri: String,
        authorization: Option<String>,
        protocol: Option<String>,
    }

    /// Runs `script` as the server side of the first terminal connection while `scenario` runs
    /// as its client; returns what the server saw of the upgrade request.
    async fn session<S: Future<Output = ()>, C: Future<Output = ()>>(
        script: impl FnOnce(WebSocketStream<TcpStream>) -> S,
        scenario: impl FnOnce(Client) -> C,
    ) -> Seen {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let seen = Arc::new(Mutex::new(Seen::default()));
        let recorded = Arc::clone(&seen);
        let server = async move {
            let (stream, _) = listener.accept().await.expect("accept");
            #[allow(
                clippy::result_large_err,
                reason = "the handshake callback's signature is tungstenite's"
            )]
            let callback = move |request: &Request, mut response: Response| {
                let mut seen = recorded.lock().expect("lock");
                seen.uri = request.uri().to_string();
                let header = |name| {
                    request
                        .headers()
                        .get(name)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned)
                };
                seen.authorization = header("authorization");
                seen.protocol = header("sec-websocket-protocol");
                response.headers_mut().insert(
                    "sec-websocket-protocol",
                    HeaderValue::from_static(TerminalRequest::PROTOCOL),
                );
                Ok(response)
            };
            let socket = tokio_tungstenite::accept_hdr_async(stream, callback)
                .await
                .expect("handshake");
            script(socket).await;
        };
        let client = Client::new(&format!("http://{address}"), "secret").expect("client");
        tokio::join!(server, scenario(client));
        let mut seen = seen.lock().expect("lock");
        std::mem::take(&mut *seen)
    }

    #[tokio::test]
    async fn a_terminal_carries_input_output_resizes_and_the_exit_code() {
        let seen = session(
            |mut socket| async move {
                let first = socket.next().await.expect("message").expect("ok");
                assert_eq!(first, Message::Binary(b"ls\n".to_vec().into()));
                let resize = socket.next().await.expect("message").expect("ok");
                assert_eq!(
                    resize,
                    Message::Text(r#"{"type":"resize","cols":120,"rows":40}"#.into())
                );
                socket
                    .send(Message::Binary(b"file\r\n".to_vec().into()))
                    .await
                    .expect("output");
                socket
                    .send(Message::Text(r#"{"type":"exit","code":3}"#.into()))
                    .await
                    .expect("exit");
                socket.close(None).await.ok();
            },
            |client| async move {
                let command = ["bash".to_owned(), "-l".to_owned()];
                let terminal = client
                    .open_terminal("sbx_1", &command, 100, 30)
                    .await
                    .expect("open");
                let (mut input, mut output) = terminal.split();
                input.send(b"ls\n".to_vec()).await.expect("send");
                input.resize(120, 40).await.expect("resize");
                assert_eq!(
                    output.next().await.expect("next"),
                    Some(TerminalEvent::Output(b"file\r\n".to_vec()))
                );
                assert_eq!(
                    output.next().await.expect("next"),
                    Some(TerminalEvent::Exit(TerminalExit::Code(3)))
                );
                assert_eq!(output.next().await.expect("next"), None, "nothing follows");
            },
        )
        .await;

        assert_eq!(
            seen.uri,
            "/v1/sandboxes/sbx_1/terminal?command=bash&command=-l&cols=100&rows=30"
        );
        assert_eq!(seen.authorization.as_deref(), Some("Bearer secret"));
        assert_eq!(seen.protocol.as_deref(), Some(TerminalRequest::PROTOCOL));
    }

    #[tokio::test]
    async fn a_connection_closed_without_an_exit_ends_the_output() {
        session(
            |mut socket| async move {
                socket.close(None).await.ok();
            },
            |client| async move {
                let terminal = client
                    .open_terminal("sbx_1", &[], 80, 24)
                    .await
                    .expect("open");
                let (_, mut output) = terminal.split();
                assert_eq!(output.next().await.expect("next"), None);
            },
        )
        .await;
    }

    #[tokio::test]
    async fn a_failure_frame_reports_why_the_terminal_ended() {
        session(
            |mut socket| async move {
                socket
                    .send(Message::Text(r#"{"type":"exit","failure":"lost"}"#.into()))
                    .await
                    .expect("exit");
            },
            |client| async move {
                let terminal = client
                    .open_terminal("sbx_1", &[], 80, 24)
                    .await
                    .expect("open");
                let (_, mut output) = terminal.split();
                assert_eq!(
                    output.next().await.expect("next"),
                    Some(TerminalEvent::Exit(TerminalExit::Failed(
                        TerminalEndReason::Lost
                    )))
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn a_refused_upgrade_reports_the_servers_problem() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let server = async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await;
            let body = r#"{"type":"about:blank","title":"The sandbox is not running","status":409,"code":"sandbox.not_running"}"#;
            let response = format!(
                "HTTP/1.1 409 Conflict\r\ncontent-type: application/problem+json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.expect("write");
        };
        let client = Client::new(&format!("http://{address}"), "secret").expect("client");
        let ((), error) = tokio::join!(server, client.open_terminal("sbx_1", &[], 80, 24));
        let error = error.expect_err("refused");
        assert!(
            matches!(&error, crate::Error::Api(problem) if problem.code == "sandbox.not_running"),
            "{error:?}"
        );
    }
}
