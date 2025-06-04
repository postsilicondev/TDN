use rand_chacha::{
    rand_core::{RngCore, SeedableRng},
    ChaChaRng,
};
use tdn_types::{
    primitives::{PeerId, Result},
    rpc::RpcParam,
};
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender},
};

use super::{rpc_channel, ChannelMessage, RpcMessage};

enum FutureResult {
    Out(RpcMessage),
    Stream(ChannelMessage),
}

pub(super) async fn channel_listen(
    send: Sender<RpcMessage>,
    out_send: Sender<RpcParam>,
    mut my_recv: Receiver<ChannelMessage>,
) -> Result<()> {
    let mut rng = ChaChaRng::from_entropy();
    let id: u64 = rng.next_u64();
    let (s_send, mut s_recv) = rpc_channel();

    let peer = PeerId::default();
    let data = String::new();
    send.send(RpcMessage::Open(id, peer, data, s_send)).await?;

    loop {
        let res = select! {
            v = async { s_recv.recv().await.map(FutureResult::Out) } => v,
            v = async { my_recv.recv().await.map(FutureResult::Stream) } => v,
        };

        match res {
            Some(FutureResult::Out(msg)) => {
                let param = match msg {
                    RpcMessage::Response(param) => param,
                    _ => Default::default(),
                };
                let _ = out_send.send(param).await;
            }
            Some(FutureResult::Stream(msg)) => match msg {
                ChannelMessage::Sync(msg, tx) => {
                    let id: u64 = rng.next_u64();
                    send.send(RpcMessage::Request(id, peer, msg, Some(tx)))
                        .await?;
                }
                ChannelMessage::Async(msg) => {
                    send.send(RpcMessage::Request(id, peer, msg, None)).await?;
                }
            },
            None => break,
        }
    }

    send.send(RpcMessage::Close(id)).await?;
    Ok(())
}
