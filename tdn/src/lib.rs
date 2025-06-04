//! `TDN` - Trusted Distributed Network.
//!
//! Blockchain infrastructure framework for security and trusted distributed interactive.
//!
//! TDN is underlying network (including p2p, rpc, and other special transports)
//! and application framework built on Groups and Layers, we built this framework
//! because we feel that the blockchain is very limited. If you want a more open
//! and free distributed application development technology, and Pluggable,
//! lightweight application framework, TDN can satisfy you.

#![recursion_limit = "1024"]

// check features conflict
#[cfg(any(
    all(
        feature = "std",
        any(feature = "single", feature = "multiple", feature = "full")
    ),
    all(
        feature = "single",
        any(feature = "std", feature = "multiple", feature = "full")
    ),
    all(
        feature = "multiple",
        any(feature = "std", feature = "single", feature = "full")
    ),
    all(
        feature = "full",
        any(feature = "std", feature = "single", feature = "multiple")
    ),
))]
panic!("feature conflict, only one feature at one time.");

#[macro_use]
extern crate tracing;

mod config;
mod group;
mod rpc;

#[cfg(any(feature = "std", feature = "full"))]
mod layer;

// public mod
pub mod error;

// re-export tdn_types
pub use tdn_types as types;

// public struct
pub mod prelude {
    pub use super::config::Config;
    pub use super::rpc::{
        channel_rpc_channel, ChannelAddr, ChannelMessage, ChannelRpcSender, RpcConfig, RpcMessage,
    };
    pub use chamomile::prelude::Config as P2pConfig;
    pub use tdn_types::{
        group::{GroupId, GROUP_BYTES_LENGTH},
        message::{
            NetworkType, ReceiveMessage, RecvType, SendMessage, SendType, StateRequest,
            StateResponse,
        },
        primitives::{Broadcast, HandleResult, Peer, PeerId, PeerKey, Result},
    };

    use chamomile::prelude::{
        start as chamomile_start, start_with_key as chamomile_start_with_key,
        ReceiveMessage as ChamomileReceiveMessage, SendMessage as ChamomileSendMessage,
    };
    use std::{path::PathBuf, sync::Arc};
    use tdn_types::message::RpcSendType;
    use tokio::{
        sync::mpsc::{self, Receiver, Sender},
        sync::RwLock,
    };

    use super::group::*;
    use super::rpc::start as rpc_start;

    #[cfg(any(feature = "std", feature = "full"))]
    use super::layer::*;

    /// new a channel, send message to TDN Message. default capacity is 1024.
    pub fn new_send_channel() -> (Sender<SendMessage>, Receiver<SendMessage>) {
        mpsc::channel(1024)
    }

    /// new a channel, send message to TDN Message. default capacity is 1024.
    pub fn new_receive_channel() -> (Sender<ReceiveMessage>, Receiver<ReceiveMessage>) {
        mpsc::channel(1024)
    }

    /// start a service, use config.toml file.
    /// send a Sender<Message>, and return the peer_id, and service Sender<Message>.
    pub async fn start() -> Result<(PeerId, Sender<SendMessage>, Receiver<ReceiveMessage>)> {
        let (send_send, send_recv) = new_send_channel();
        let (recv_send, recv_recv) = new_receive_channel();

        let config = Config::load(PathBuf::from("./")).await;

        let (_secret, ids, p2p_config, rpc_config) = config.split();
        let rpc_send = start_rpc(rpc_config, recv_send.clone()).await?;
        let peer_id =
            start_main(ids, p2p_config, recv_send, send_recv, Some(rpc_send), None).await?;

        Ok((peer_id, send_send, recv_recv))
    }

    /// start a service with config.
    pub async fn start_with_config(
        config: Config,
    ) -> Result<(PeerId, Sender<SendMessage>, Receiver<ReceiveMessage>)> {
        let (send_send, send_recv) = new_send_channel();
        let (recv_send, recv_recv) = new_receive_channel();

        let (_secret, ids, p2p_config, rpc_config) = config.split();
        let rpc_send = start_rpc(rpc_config, recv_send.clone()).await?;
        let peer_id =
            start_main(ids, p2p_config, recv_send, send_recv, Some(rpc_send), None).await?;

        Ok((peer_id, send_send, recv_recv))
    }

    /// start a service with config and PeerKey.
    pub async fn start_with_config_and_key(
        config: Config,
        key: PeerKey,
    ) -> Result<(PeerId, Sender<SendMessage>, Receiver<ReceiveMessage>)> {
        let (send_send, send_recv) = new_send_channel();
        let (recv_send, recv_recv) = new_receive_channel();

        let (_secret, ids, p2p_config, rpc_config) = config.split();
        let rpc_send = start_rpc(rpc_config, recv_send.clone()).await?;
        let peer_id = start_main(
            ids,
            p2p_config,
            recv_send,
            send_recv,
            Some(rpc_send),
            Some(key),
        )
        .await?;

        Ok((peer_id, send_send, recv_recv))
    }

    /// start a separate rpc service.
    pub async fn start_rpc(
        config: RpcConfig,
        out_send: Sender<ReceiveMessage>,
    ) -> Result<Sender<RpcSendType>> {
        rpc_start(config, out_send).await
    }

    /// start a separate p2p service and unified tdn channel.
    pub async fn start_main(
        group_ids: Vec<GroupId>,
        p2p_config: P2pConfig,
        out_send: Sender<ReceiveMessage>,
        mut self_recv: Receiver<SendMessage>,
        rpc_send: Option<Sender<RpcSendType>>,
        key: Option<PeerKey>,
    ) -> Result<PeerId> {
        // start chamomile network & inner rpc.
        let res1 = if let Some(key) = key {
            chamomile_start_with_key(p2p_config, key).await
        } else {
            chamomile_start(p2p_config).await
        };

        let (peer_id, p2p_send, mut p2p_recv) = res1?;

        debug!("chamomile & jsonrpc service started");
        let my_groups = Arc::new(RwLock::new(group_ids));
        let my_groups_1 = my_groups.clone();

        // handle chamomile send msg.
        let listen_task = tokio::spawn(async move {
            // if group's inner message, from_group in our groups.
            // if layer's message,       from_group not in our groups.

            while let Some(message) = p2p_recv.recv().await {
                match message {
                    ChamomileReceiveMessage::StableConnect(peer, mut data) => {
                        if data.len() < GROUP_BYTES_LENGTH * 2 {
                            continue;
                        }
                        let mut fgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        let mut tgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        fgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        tgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        let fgid = GroupId::from_be_bytes(fgid_bytes);
                        let tgid = GroupId::from_be_bytes(tgid_bytes);

                        let group_lock = my_groups.read().await;
                        if group_lock.len() == 0 {
                            continue;
                        }

                        if fgid == tgid && group_lock.contains(&fgid) {
                            drop(group_lock);
                            let _ = group_handle_connect(&fgid, &out_send, peer.into(), data).await;
                        } else {
                            drop(group_lock);
                            // layer handle it.
                            #[cfg(any(feature = "std", feature = "full"))]
                            let _ = layer_handle_connect(fgid, tgid, &out_send, peer.into(), data)
                                .await;
                        }
                    }
                    ChamomileReceiveMessage::ResultConnect(peer, mut data) => {
                        if data.len() < GROUP_BYTES_LENGTH * 2 {
                            continue;
                        }
                        let mut fgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        let mut tgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        fgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        tgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        let fgid = GroupId::from_be_bytes(fgid_bytes);
                        let tgid = GroupId::from_be_bytes(tgid_bytes);

                        let group_lock = my_groups.read().await;
                        if group_lock.len() == 0 {
                            continue;
                        }

                        if fgid == tgid && group_lock.contains(&fgid) {
                            drop(group_lock);
                            let _ =
                                group_handle_result_connect(&fgid, &out_send, peer.into(), data)
                                    .await;
                        } else {
                            drop(group_lock);
                            // layer handle it.
                            #[cfg(any(feature = "std", feature = "full"))]
                            let _ = layer_handle_result_connect(
                                fgid,
                                tgid,
                                &out_send,
                                peer.into(),
                                data,
                            )
                            .await;
                        }
                    }
                    ChamomileReceiveMessage::StableResult(peer, is_ok, mut data) => {
                        if data.len() < GROUP_BYTES_LENGTH * 2 {
                            continue;
                        }
                        let mut fgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        let mut tgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        fgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        tgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        let fgid = GroupId::from_be_bytes(fgid_bytes);
                        let tgid = GroupId::from_be_bytes(tgid_bytes);

                        let group_lock = my_groups.read().await;
                        if group_lock.len() == 0 {
                            continue;
                        }

                        if fgid == tgid && group_lock.contains(&fgid) {
                            drop(group_lock);
                            let _ = group_handle_result(&fgid, &out_send, peer.into(), is_ok, data)
                                .await;
                        } else {
                            drop(group_lock);
                            // layer handle it.
                            #[cfg(any(feature = "std", feature = "full"))]
                            let _ = layer_handle_result(
                                fgid,
                                tgid,
                                &out_send,
                                peer.into(),
                                is_ok,
                                data,
                            )
                            .await;
                        }
                    }
                    ChamomileReceiveMessage::StableLeave(peer) => {
                        let group_lock = my_groups.read().await;
                        for gid in group_lock.iter() {
                            let _ = group_handle_leave(&gid, &out_send, peer).await;
                            #[cfg(any(feature = "std", feature = "full"))]
                            let _ = layer_handle_leave(*gid, &out_send, peer).await;
                        }
                        drop(group_lock);
                    }
                    ChamomileReceiveMessage::Data(peer_id, mut data) => {
                        if data.len() < GROUP_BYTES_LENGTH * 2 {
                            continue;
                        }
                        let mut fgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        let mut tgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        fgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        tgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        let fgid = GroupId::from_be_bytes(fgid_bytes);
                        let tgid = GroupId::from_be_bytes(tgid_bytes);

                        let group_lock = my_groups.read().await;
                        if group_lock.len() == 0 {
                            continue;
                        }

                        if fgid == tgid && group_lock.contains(&fgid) {
                            drop(group_lock);
                            let _ = group_handle_data(&fgid, &out_send, peer_id, data).await;
                        } else {
                            drop(group_lock);
                            // layer handle it.
                            #[cfg(any(feature = "std", feature = "full"))]
                            let _ = layer_handle_data(fgid, tgid, &out_send, peer_id, data).await;
                        }
                    }
                    ChamomileReceiveMessage::Stream(id, stream, mut data) => {
                        if data.len() < GROUP_BYTES_LENGTH * 2 {
                            continue;
                        }
                        let mut fgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        let mut tgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        fgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        tgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        let fgid = GroupId::from_be_bytes(fgid_bytes);
                        let tgid = GroupId::from_be_bytes(tgid_bytes);

                        let group_lock = my_groups.read().await;
                        if group_lock.len() == 0 {
                            continue;
                        }

                        if fgid == tgid && group_lock.contains(&fgid) {
                            drop(group_lock);
                            let _ = group_handle_stream(&fgid, &out_send, id, stream, data).await;
                        } else {
                            drop(group_lock);
                            // layer handle it.
                            #[cfg(any(feature = "std", feature = "full"))]
                            let _ =
                                layer_handle_stream(fgid, tgid, &out_send, id, stream, data).await;
                        }
                    }
                    ChamomileReceiveMessage::Delivery(t, tid, is_ok, mut data) => {
                        if data.len() < GROUP_BYTES_LENGTH * 2 {
                            continue;
                        }
                        let mut fgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        let mut tgid_bytes = [0u8; GROUP_BYTES_LENGTH];
                        fgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        tgid_bytes.copy_from_slice(data.drain(..GROUP_BYTES_LENGTH).as_slice());
                        let fgid = GroupId::from_be_bytes(fgid_bytes);
                        let tgid = GroupId::from_be_bytes(tgid_bytes);

                        let group_lock = my_groups.read().await;
                        if group_lock.len() == 0 {
                            continue;
                        }

                        if fgid == tgid && group_lock.contains(&fgid) {
                            drop(group_lock);
                            let _ =
                                group_handle_delivery(&fgid, &out_send, t.into(), tid, is_ok).await;
                        } else {
                            drop(group_lock);
                            // layer handle it.
                            #[cfg(any(feature = "std", feature = "full"))]
                            let _ = layer_handle_delivery(
                                tgid, // Assuming it is remote sended.
                                fgid,
                                &out_send,
                                t.into(),
                                tid,
                                is_ok,
                            )
                            .await;
                        }
                    }
                    ChamomileReceiveMessage::NetworkLost => {
                        out_send
                            .send(ReceiveMessage::NetworkLost)
                            .await
                            .expect("Outside channel missing");
                    }
                    ChamomileReceiveMessage::OwnConnect(peer) => {
                        let assist_id = peer.assist;
                        let mut new_peer: Peer = peer.into();
                        new_peer.id = assist_id;
                        out_send
                            .send(ReceiveMessage::Own(RecvType::Connect(new_peer, vec![])))
                            .await
                            .expect("Outside channel missing");
                    }
                    ChamomileReceiveMessage::OwnLeave(peer) => {
                        let assist_id = peer.assist;
                        let mut new_peer: Peer = peer.into();
                        new_peer.id = assist_id;
                        out_send
                            .send(ReceiveMessage::Own(RecvType::Leave(new_peer)))
                            .await
                            .expect("Outside channel missing");
                    }
                    ChamomileReceiveMessage::OwnEvent(aid, data) => {
                        out_send
                            .send(ReceiveMessage::Own(RecvType::Event(aid, data)))
                            .await
                            .expect("Outside channel missing");
                    }
                }
            }

            warn!("Chamomile network is stopped");
        });

        // handle outside msg.
        tokio::spawn(async move {
            while let Some(message) = self_recv.recv().await {
                match message {
                    SendMessage::Own(msg) => match msg {
                        SendType::Connect(tid, peer, data) => {
                            p2p_send
                                .send(ChamomileSendMessage::StableConnect(tid, peer.into(), data))
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        SendType::Event(_, _, data) => {
                            p2p_send
                                .send(ChamomileSendMessage::OwnEvent(data))
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        SendType::Stream(id, stream, data) => {
                            p2p_send
                                .send(ChamomileSendMessage::Stream(id, stream, data))
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        SendType::Disconnect(..) => {
                            warn!("Own message has no DisConnect");
                        }
                        SendType::Result(..) => {
                            warn!("Own message has no Result");
                        }
                    },
                    #[cfg(any(feature = "single", feature = "std"))]
                    SendMessage::Group(msg) => {
                        let groups_lock = my_groups_1.read().await;
                        if groups_lock.len() == 0 {
                            drop(groups_lock);
                            continue;
                        }
                        let default_group_id = groups_lock[0].clone();
                        drop(groups_lock);

                        group_handle_send(default_group_id, &p2p_send, msg)
                            .await
                            .map_err(|e| error!("Chamomile channel: {:?}", e))
                            .expect("Chamomile channel closed");
                    }
                    #[cfg(any(feature = "multiple", feature = "full"))]
                    SendMessage::Group(group_id, msg) => {
                        group_handle_send(group_id, &p2p_send, msg)
                            .await
                            .map_err(|e| error!("Chamomile channel: {:?}", e))
                            .expect("Chamomile channel closed");
                    }
                    SendMessage::Rpc(msg) => {
                        if let Some(ref rpc_send) = rpc_send {
                            rpc_send
                                .send(msg)
                                .await
                                .map_err(|e| error!("Rpc channel: {:?}", e))
                                .expect("Rpc channel closed");
                        }
                    }
                    #[cfg(feature = "std")]
                    SendMessage::Layer(tgid, msg) => {
                        let groups_lock = my_groups_1.read().await;
                        if groups_lock.len() == 0 {
                            drop(groups_lock);
                            continue;
                        }
                        let default_group_id = groups_lock[0].clone();
                        drop(groups_lock);

                        layer_handle_send(default_group_id, tgid, &p2p_send, msg)
                            .await
                            .map_err(|e| error!("Chamomile channel: {:?}", e))
                            .expect("Chamomile channel closed");
                    }
                    #[cfg(feature = "full")]
                    SendMessage::Layer(fgid, tgid, msg) => {
                        layer_handle_send(fgid, tgid, &p2p_send, msg)
                            .await
                            .map_err(|e| error!("Chamomile channel: {:?}", e))
                            .expect("Chamomile channel closed");
                    }
                    SendMessage::Network(nmsg) => match nmsg {
                        NetworkType::Broadcast(broadcast, data) => {
                            // broadcast use default_group_id.
                            let mut bytes = vec![];
                            let groups_lock = my_groups_1.read().await;
                            if groups_lock.len() == 0 {
                                drop(groups_lock);
                                continue;
                            }
                            bytes.extend(&groups_lock[0].to_be_bytes());
                            drop(groups_lock);
                            bytes.extend(data);
                            p2p_send
                                .send(ChamomileSendMessage::Broadcast(broadcast, bytes))
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        NetworkType::Connect(peer) => {
                            p2p_send
                                .send(ChamomileSendMessage::Connect(peer.into()))
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        NetworkType::DisConnect(peer) => {
                            p2p_send
                                .send(ChamomileSendMessage::DisConnect(peer.into()))
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        NetworkType::NetworkState(req, sender) => {
                            p2p_send
                                .send(ChamomileSendMessage::NetworkState(req, sender))
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        NetworkType::NetworkReboot => {
                            p2p_send
                                .send(ChamomileSendMessage::NetworkReboot)
                                .await
                                .map_err(|e| error!("Chamomile channel: {:?}", e))
                                .expect("Chamomile channel closed");
                        }
                        NetworkType::NetworkStop => {
                            warn!("Start stop chamomile...");
                            let _ = p2p_send.send(ChamomileSendMessage::NetworkStop).await;
                            listen_task.abort();
                            break;
                        }
                        #[cfg(any(feature = "multiple", feature = "full"))]
                        NetworkType::AddGroup(gid) => {
                            let mut group_lock = my_groups_1.write().await;
                            if !group_lock.contains(&gid) {
                                group_lock.push(gid);
                            }
                            drop(group_lock);
                        }
                        #[cfg(any(feature = "multiple", feature = "full"))]
                        NetworkType::DelGroup(gid) => {
                            let mut group_lock = my_groups_1.write().await;
                            let mut need_remove: Vec<usize> = vec![];
                            for (k, i) in group_lock.iter().enumerate() {
                                if i == &gid {
                                    need_remove.push(k);
                                }
                            }
                            for i in need_remove.iter().rev() {
                                group_lock.remove(*i);
                            }
                            drop(group_lock);
                        }
                    },
                }
            }
        });

        Ok(peer_id)
    }
}
