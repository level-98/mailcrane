//! The byte stream under a session: reading whole responses within the
//! size limits, and writing commands.

use crate::error::ImapError;
use crate::parser::{frame_end, parse_response_with, Limits, ParseError, Response};
use std::future::Future;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// Runs `fut`, failing with [`ImapError::Timeout`] after `limit`.
pub(crate) async fn within<T>(
    limit: Duration,
    fut: impl Future<Output = T>,
) -> Result<T, ImapError> {
    tokio::time::timeout(limit, fut)
        .await
        .map_err(|_| ImapError::Timeout)
}

/// Opens the TCP connection within `limit`.
pub(crate) async fn connect_tcp(
    host: &str,
    port: u16,
    limit: Duration,
) -> Result<TcpStream, ImapError> {
    within(limit, TcpStream::connect((host, port)))
        .await?
        .map_err(|_| ImapError::CannotConnect)
}

pub(crate) struct Connection {
    io: Box<dyn Io>,
    buf: Vec<u8>,
    limits: Limits,
}

impl Connection {
    pub(crate) fn new(io: Box<dyn Io>, limits: Limits) -> Self {
        Self {
            io,
            buf: Vec::new(),
            limits,
        }
    }

    /// Whether bytes have arrived that nothing has read yet.
    pub(crate) fn has_buffered(&self) -> bool {
        !self.buf.is_empty()
    }

    pub(crate) fn into_io(self) -> Box<dyn Io> {
        self.io
    }

    /// Reads one complete response. Callers wrap this in a timeout.
    pub(crate) async fn read_response(&mut self) -> Result<Response, ImapError> {
        loop {
            let framed = frame_end(&self.buf, &self.limits).map_err(map_parse)?;
            if let Some(end) = framed {
                let (response, used) =
                    parse_response_with(self.buf.get(..end).unwrap_or_default(), &self.limits)
                        .map_err(map_parse)?;
                self.buf.drain(..used.min(self.buf.len()));
                return Ok(response);
            }
            let mut chunk = [0u8; 8192];
            let n = self
                .io
                .read(&mut chunk)
                .await
                .map_err(|_| ImapError::ConnectionClosed)?;
            if n == 0 {
                return Err(ImapError::ConnectionClosed);
            }
            self.buf
                .extend_from_slice(chunk.get(..n).unwrap_or_default());
        }
    }

    pub(crate) async fn write(&mut self, bytes: &[u8]) -> Result<(), ImapError> {
        self.io
            .write_all(bytes)
            .await
            .map_err(|_| ImapError::ConnectionClosed)?;
        self.io
            .flush()
            .await
            .map_err(|_| ImapError::ConnectionClosed)
    }

    pub(crate) async fn shutdown(&mut self) {
        let _ = self.io.shutdown().await;
    }
}

fn map_parse(error: ParseError) -> ImapError {
    match error {
        ParseError::LineTooLong => ImapError::LineTooLong,
        ParseError::TooLarge => ImapError::ResponseTooLarge,
        ParseError::Incomplete | ParseError::Invalid { .. } => ImapError::Protocol,
    }
}
