use std::sync::Arc;

use futures_util::{SinkExt as _, StreamExt as _};
use rmcp::{
    RoleServer,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{Transport, async_rw::JsonRpcMessageCodec},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, Stdin, Stdout},
    sync::Mutex,
};
use tokio_util::codec::{FramedRead, FramedWrite};

pub const MAX_INBOUND_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

type ServerWriter<W> = FramedWrite<W, JsonRpcMessageCodec<TxJsonRpcMessage<RoleServer>>>;

pub struct BoundedStdioTransport<R, W> {
    reader: FramedRead<R, JsonRpcMessageCodec<RxJsonRpcMessage<RoleServer>>>,
    writer: Arc<Mutex<Option<ServerWriter<W>>>>,
}

impl<R, W> BoundedStdioTransport<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    #[must_use]
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: FramedRead::new(
                reader,
                JsonRpcMessageCodec::new_with_max_length(MAX_INBOUND_MESSAGE_BYTES),
            ),
            writer: Arc::new(Mutex::new(Some(FramedWrite::new(
                writer,
                JsonRpcMessageCodec::new(),
            )))),
        }
    }
}

#[must_use]
pub fn bounded_stdio() -> BoundedStdioTransport<Stdin, Stdout> {
    BoundedStdioTransport::new(tokio::io::stdin(), tokio::io::stdout())
}

impl<R, W> Transport<RoleServer> for BoundedStdioTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = Arc::clone(&self.writer);
        async move {
            let mut guard = writer.lock().await;
            let Some(writer) = guard.as_mut() else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "SOMA MCP stdio transport is closed",
                ));
            };
            writer.send(item).await.map_err(Into::into)
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        match self.reader.next().await {
            Some(Ok(message)) => Some(message),
            Some(Err(error)) => {
                if matches!(
                    error,
                    rmcp::transport::async_rw::JsonRpcMessageCodecError::MaxLineLengthExceeded
                ) {
                    eprintln!(
                        "soma-mcp: inbound MCP message exceeded {MAX_INBOUND_MESSAGE_BYTES} bytes"
                    );
                } else {
                    eprintln!("soma-mcp: invalid inbound MCP message: {error}");
                }
                None
            }
            None => None,
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let mut guard = self.writer.lock().await;
        if let Some(mut writer) = guard.take() {
            writer.close().await.map_err(Into::into)
        } else {
            Ok(())
        }
    }
}
