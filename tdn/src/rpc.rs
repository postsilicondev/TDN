mod channel;
mod http;
mod ws;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::{
    net::TcpListener,
    select,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot,
    },
    time::timeout,
};

use tdn_types::{
    message::{ReceiveMessage, RpcRecvType, RpcSendType},
    primitives::{new_io_error, PeerId, Result},
    rpc::RpcParam,
};

pub type ChannelAddr = (Sender<RpcParam>, Receiver<ChannelMessage>);

pub struct RpcConfig {
    pub http: Option<SocketAddr>,
    pub ws: Option<SocketAddr>,
    pub channel: Option<ChannelAddr>,
    pub index: Option<PathBuf>,
}

#[derive(Debug)]
pub enum RpcMessage {
    /// uid, peer, data, sender
    Open(u64, PeerId, String, Sender<RpcMessage>),
    Close(u64),
    Request(u64, PeerId, RpcParam, Option<oneshot::Sender<RpcMessage>>),
    Response(RpcParam),
}

fn rpc_channel() -> (Sender<RpcMessage>, Receiver<RpcMessage>) {
    mpsc::channel(128)
}

fn rpc_send_channel() -> (Sender<RpcSendType>, Receiver<RpcSendType>) {
    mpsc::channel(128)
}

pub fn channel_rpc_channel() -> (
    Sender<RpcParam>,
    Receiver<RpcParam>,
    ChannelRpcSender,
    Receiver<ChannelMessage>,
) {
    let (out_send, out_recv) = mpsc::channel(128);
    let (inner_send, inner_recv) = mpsc::channel(128);
    (out_send, out_recv, ChannelRpcSender(inner_send), inner_recv)
}

pub enum ChannelMessage {
    Sync(RpcParam, oneshot::Sender<RpcMessage>),
    Async(RpcParam),
}

/// sender for channel rpc. support sync and no-sync
#[derive(Clone, Debug)]
pub struct ChannelRpcSender(pub Sender<ChannelMessage>);

impl ChannelRpcSender {
    pub async fn send(&self, msg: RpcParam) {
        let _ = self.0.send(ChannelMessage::Async(msg)).await;
    }

    pub async fn send_timeout(&self, msg: RpcParam, timeout_millis: u64) {
        let _ = self
            .0
            .send_timeout(
                ChannelMessage::Async(msg),
                Duration::from_millis(timeout_millis),
            )
            .await;
    }

    pub async fn sync_send(&self, msg: RpcParam, timeout_millis: u64) -> Result<RpcParam> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(ChannelMessage::Sync(msg, tx)).await;

        if let Ok(msg) = timeout(Duration::from_millis(timeout_millis), rx).await {
            let msg = msg.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            match msg {
                RpcMessage::Response(param) => Ok(param),
                _ => Ok(Default::default()),
            }
        } else {
            Err(new_io_error("Timeout").into())
        }
    }
}

pub(crate) async fn start(
    config: RpcConfig,
    send: Sender<ReceiveMessage>,
) -> Result<Sender<RpcSendType>> {
    let (out_send, out_recv) = rpc_send_channel();

    let (self_send, self_recv) = rpc_channel();

    server(self_send, config).await?;
    listen(send, out_recv, self_recv).await?;

    Ok(out_send)
}

enum FutureResult {
    Out(RpcSendType),
    Stream(RpcMessage),
}

async fn listen(
    send: Sender<ReceiveMessage>,
    mut out_recv: Receiver<RpcSendType>,
    mut self_recv: Receiver<RpcMessage>,
) -> Result<()> {
    tokio::spawn(async move {
        let mut ws_connections: HashMap<u64, Sender<RpcMessage>> = HashMap::new();
        let mut sync_connections: HashMap<u64, oneshot::Sender<RpcMessage>> = HashMap::new();

        loop {
            let res = select! {
                v = async { out_recv.recv().await.map(FutureResult::Out) } => v,
                v = async { self_recv.recv().await.map(FutureResult::Stream) } => v
            };

            match res {
                Some(FutureResult::Out(msg)) => {
                    match msg {
                        RpcSendType::Connect(_url, _pid, _sig, _data) => {
                            // TODO start a new websocket
                        }
                        RpcSendType::Leave(id) => {
                            // Close a websocket
                            if let Some(s) = ws_connections.remove(&id) {
                                let _ = s.send(RpcMessage::Close(id)).await;
                            }
                        }
                        RpcSendType::Event(id, params) => {
                            if let Some(s) = ws_connections.get(&id) {
                                let _ = s.send(RpcMessage::Response(params)).await;
                            } else {
                                let s = sync_connections.remove(&id);
                                if s.is_some() {
                                    let _ = s.unwrap().send(RpcMessage::Response(params));
                                }
                            }
                        }
                    }
                }
                Some(FutureResult::Stream(msg)) => {
                    match msg {
                        RpcMessage::Request(id, peer, params, sender) => {
                            let is_ws = sender.is_none();
                            if !is_ws {
                                sync_connections.insert(id, sender.unwrap());
                            }
                            send.send(ReceiveMessage::Rpc(RpcRecvType::Event(id, peer, params)))
                                .await
                                .expect("Rpc to Outside channel closed");
                        }
                        RpcMessage::Open(id, peer, data, sender) => {
                            ws_connections.insert(id, sender);
                            send.send(ReceiveMessage::Rpc(RpcRecvType::Connect(id, peer, data)))
                                .await
                                .expect("Rpc to Outside channel closed");
                        }
                        RpcMessage::Close(id) => {
                            // clear this id
                            ws_connections.remove(&id);
                            sync_connections.remove(&id);
                            send.send(ReceiveMessage::Rpc(RpcRecvType::Leave(id)))
                                .await
                                .expect("Rpc to Outside channel closed");
                        }
                        _ => {} // others not handle
                    }
                }
                None => break,
            }
        }
    });

    Ok(())
}

async fn server(send: Sender<RpcMessage>, config: RpcConfig) -> Result<()> {
    // HTTP blind
    if let Some(http) = config.http {
        tokio::spawn(http::http_listen(
            config.index.clone(),
            send.clone(),
            TcpListener::bind(http).await.map_err(|e| {
                error!("RPC HTTP listen {:?}", e);
                std::io::Error::new(std::io::ErrorKind::Other, "TCP Listen")
            })?,
        ));
    }

    // ws
    if config.ws.is_some() {
        tokio::spawn(ws::ws_listen(
            send.clone(),
            TcpListener::bind(config.ws.unwrap()).await.map_err(|e| {
                error!("RPC WS listen {:?}", e);
                std::io::Error::new(std::io::ErrorKind::Other, "TCP Listen")
            })?,
        ));
    }

    // Channel
    if let Some((out_send, my_recv)) = config.channel {
        tokio::spawn(channel::channel_listen(send, out_send, my_recv));
    }

    Ok(())
}
