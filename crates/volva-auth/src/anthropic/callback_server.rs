use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::time::Instant;
use url::Url;
use volva_core::AuthTarget;

use super::oauth;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:0";
const CALLBACK_HOST: &str = "localhost";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackPayload {
    pub code: String,
    pub state: String,
}

enum CallbackAttempt {
    Success(CallbackPayload),
    Retry(anyhow::Error),
    Fatal(anyhow::Error),
}

#[derive(Debug)]
pub struct CallbackServer {
    listener: TcpListener,
    target: AuthTarget,
}

impl CallbackServer {
    pub async fn bind(target: AuthTarget) -> Result<Self> {
        let listener = TcpListener::bind(DEFAULT_BIND_ADDR)
            .await
            .context("failed to bind Anthropic OAuth callback server")?;

        Ok(Self { listener, target })
    }

    pub fn callback_url(&self) -> Result<String> {
        let port = self
            .listener
            .local_addr()
            .context("failed to read Anthropic OAuth callback address")?
            .port();
        Ok(format!("http://{CALLBACK_HOST}:{port}/callback"))
    }

    pub async fn wait_for_callback(
        self,
        expected_state: &str,
        timeout: Duration,
    ) -> Result<CallbackPayload> {
        let deadline = Instant::now() + timeout;

        loop {
            let (stream, _) = tokio::time::timeout_at(deadline, self.listener.accept())
                .await
                .context("timed out waiting for Anthropic OAuth browser callback")?
                .context("failed to accept Anthropic OAuth callback connection")?;

            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .await
                .context("failed to read Anthropic OAuth callback request")?;

            loop {
                let mut header_line = String::new();
                reader
                    .read_line(&mut header_line)
                    .await
                    .context("failed to read Anthropic OAuth callback headers")?;
                if header_line.trim().is_empty() {
                    break;
                }
            }

            match parse_callback(&request_line, expected_state) {
                CallbackAttempt::Success(payload) => {
                    write_browser_response(&mut writer, self.target, &Ok(payload.clone())).await?;
                    return Ok(payload);
                }
                CallbackAttempt::Retry(error) => {
                    write_browser_response(
                        &mut writer,
                        self.target,
                        &Err(anyhow!(error.to_string())),
                    )
                    .await?;
                }
                CallbackAttempt::Fatal(error) => {
                    write_browser_response(
                        &mut writer,
                        self.target,
                        &Err(anyhow!(error.to_string())),
                    )
                    .await?;
                    return Err(error);
                }
            }
        }
    }
}

fn parse_callback(request_line: &str, expected_state: &str) -> CallbackAttempt {
    let Some(path) = request_line.split_whitespace().nth(1) else {
        return CallbackAttempt::Retry(anyhow!(
            "received malformed Anthropic OAuth callback request"
        ));
    };
    let parsed = Url::parse(&format!("http://localhost{path}"));
    let Ok(parsed) = parsed else {
        return CallbackAttempt::Retry(anyhow!("failed to parse Anthropic OAuth callback URL"));
    };

    if let Some(error_code) = query_value(&parsed, "error") {
        let description = query_value(&parsed, "error_description")
            .unwrap_or_else(|| "unknown_oauth_error".to_string())
            .replace('+', " ");
        return CallbackAttempt::Fatal(anyhow!(
            "Anthropic authorization was rejected: {error_code} ({description})"
        ));
    }

    let Some(received_state) = query_value(&parsed, "state") else {
        return CallbackAttempt::Retry(anyhow!("Anthropic OAuth callback did not include state"));
    };
    if received_state != expected_state {
        return CallbackAttempt::Retry(anyhow!("Anthropic OAuth state mismatch"));
    }

    let Some(code) = query_value(&parsed, "code") else {
        return CallbackAttempt::Retry(anyhow!(
            "Anthropic OAuth callback did not include an authorization code"
        ));
    };

    CallbackAttempt::Success(CallbackPayload {
        code,
        state: received_state,
    })
}

fn query_value(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find_map(|(candidate, value)| (candidate == key).then(|| value.into_owned()))
}

async fn write_browser_response(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    target: AuthTarget,
    callback_result: &Result<CallbackPayload>,
) -> Result<()> {
    let response = match callback_result {
        Ok(_) => format!(
            "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            oauth::success_redirect_url(target)
        ),
        Err(error) => {
            let message = escape_html(&error.to_string());
            let body = format!(
                "<html><body><h1>Authentication failed</h1><p>{message}</p><p>You can close this window and return to volva.</p></body></html>"
            );
            format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        }
    };

    writer
        .write_all(response.as_bytes())
        .await
        .context("failed to write Anthropic OAuth browser response")
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::{CallbackPayload, CallbackServer};
    use volva_core::AuthTarget;

    #[tokio::test]
    async fn callback_server_accepts_valid_code_and_state() {
        let server = CallbackServer::bind(AuthTarget::ClaudeAi)
            .await
            .expect("callback server should bind");
        let callback_url = server.callback_url().expect("callback url");

        let handle = tokio::spawn(async move {
            server
                .wait_for_callback("expected-state", Duration::from_secs(3))
                .await
        });

        let address = callback_url
            .strip_prefix("http://")
            .expect("http callback prefix");
        let mut stream = TcpStream::connect(address.replace("/callback", ""))
            .await
            .expect("callback client should connect");
        stream
            .write_all(
                b"GET /callback?code=auth-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .expect("callback write should succeed");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("callback response should be readable");

        let payload = handle
            .await
            .expect("callback join")
            .expect("callback result");
        assert_eq!(
            payload,
            CallbackPayload {
                code: "auth-code".to_string(),
                state: "expected-state".to_string(),
            }
        );
        assert!(response.contains("302 Found"));
    }

    #[tokio::test]
    async fn callback_server_rejects_state_mismatch() {
        let server = CallbackServer::bind(AuthTarget::Console)
            .await
            .expect("callback server should bind");
        let callback_url = server.callback_url().expect("callback url");

        let handle = tokio::spawn(async move {
            server
                .wait_for_callback("expected-state", Duration::from_secs(3))
                .await
        });

        let address = callback_url
            .strip_prefix("http://")
            .expect("http callback prefix");
        let mut stream = TcpStream::connect(address.replace("/callback", ""))
            .await
            .expect("callback client should connect");
        stream
            .write_all(
                b"GET /callback?code=auth-code&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .expect("callback write should succeed");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("callback response should be readable");

        let error = handle
            .await
            .expect("callback join")
            .expect_err("state mismatch should eventually time out");
        assert!(error.to_string().contains("timed out"));
        assert!(response.contains("400 Bad Request"));
    }

    #[tokio::test]
    async fn callback_server_allows_valid_second_callback_after_bad_first_request() {
        let server = CallbackServer::bind(AuthTarget::ClaudeAi)
            .await
            .expect("callback server should bind");
        let callback_url = server.callback_url().expect("callback url");

        let handle = tokio::spawn(async move {
            server
                .wait_for_callback("expected-state", Duration::from_secs(3))
                .await
        });

        let address = callback_url
            .strip_prefix("http://")
            .expect("http callback prefix")
            .replace("/callback", "");

        let mut bad_stream = TcpStream::connect(&address)
            .await
            .expect("bad callback client should connect");
        bad_stream
            .write_all(
                b"GET /callback?code=bad-code&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .expect("bad callback write should succeed");
        let mut bad_response = String::new();
        bad_stream
            .read_to_string(&mut bad_response)
            .await
            .expect("bad callback response should be readable");

        let mut good_stream = TcpStream::connect(&address)
            .await
            .expect("good callback client should connect");
        good_stream
            .write_all(
                b"GET /callback?code=good-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .expect("good callback write should succeed");
        let mut good_response = String::new();
        good_stream
            .read_to_string(&mut good_response)
            .await
            .expect("good callback response should be readable");

        let payload = handle
            .await
            .expect("callback join")
            .expect("callback result");
        assert_eq!(
            payload,
            CallbackPayload {
                code: "good-code".to_string(),
                state: "expected-state".to_string(),
            }
        );
        assert!(bad_response.contains("400 Bad Request"));
        assert!(good_response.contains("302 Found"));
    }
}
