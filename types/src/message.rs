use tokio::sync::mpsc::Sender;

use crate::primitives::{Broadcast, DeliveryType, Peer, PeerId, StreamType};
use crate::rpc::RpcParam;
pub use chamomile_types::message::{StateRequest, StateResponse};

#[cfg(not(feature = "single"))]
use crate::group::GroupId;

/// channel message send to TDN Group.
#[derive(Debug)]
pub enum SendType {
    /// when need stable connect to a peer, send to TDN from outside.
    /// params: `delivery_id`, `peer` and `join_data`.
    Connect(u64, Peer, Vec<u8>),
    /// when peer request for stable, outside decide connect or not.
    /// params: `delivery_id`, `peer_id`, `is_connect`, `is_force_close`, `result_data`.
    /// if `is_connect` is true, it will add to allow directly list.
    /// we want to build a better network, add a `is_force_close`.
    /// if `is_connect` is false, but `is_force_close` if true, we
    /// will use this peer to build our DHT for better connection.
    /// if false, we will force close it.
    Result(u64, Peer, bool, bool, Vec<u8>),
    /// when outside want to close a connectioned peer. use it force close.
    /// params: `peer_id`.
    Disconnect(PeerId),
    /// when need send a data to a peer, only need know the peer_id,
    /// the TDN will help you send data to there.
    /// params: `delivery_id`, `peer_id`, `data_bytes`.
    Event(u64, PeerId, Vec<u8>),
    /// Apply for build a stream between nodes.
    /// params: `u32` stream symbol, and `StreamType`.
    Stream(u32, StreamType, Vec<u8>),
}

/// channel message receive from TDN Group.
#[derive(Debug)]
pub enum RecvType {
    /// when peer what a stable connection, send from TDN to outside.
    /// params: `peer`, and peer `connect_info`.
    Connect(Peer, Vec<u8>),
    /// when peer a stable connect result.
    /// params: `peer`, `is_ok` and `result_data`.
    Result(Peer, bool, Vec<u8>),
    /// when peer agree a connect, but network is closed,
    /// create a result connect to it.
    ResultConnect(Peer, Vec<u8>),
    /// when a stable connected peer leave, send from TDN to outside.
    /// params: `peer`.
    Leave(Peer),
    /// when received a data from a trusted peer, send to outside.
    /// params: `peer_id` and `data_bytes`.
    Event(PeerId, Vec<u8>),
    /// Apply for build a stream between nodes.
    /// params: `u32` stream symbol, and `StreamType`.
    Stream(u32, StreamType, Vec<u8>),
    /// Message sended delivery feedback. type has Event, StableConnect, StableResult.
    /// params: `delivery_type`, `delivery_id`, `is_sended`.
    Delivery(DeliveryType, u64, bool),
}

/// rpc message receive from TDN
#[derive(Debug)]
pub enum RpcRecvType {
    /// when a websocket connected, send from TDN to outside.
    /// params: connection uid, peer and peer `connect_info`.
    Connect(u64, PeerId, String),
    /// when a websocket disconnect, send from TDN to outside.
    /// params: connection uid.
    Leave(u64),
    /// when received a data from a connection, include http and websocket
    /// params: connection uid, request params
    Event(u64, PeerId, RpcParam),
}

/// rpc message receive from TDN
#[derive(Debug)]
pub enum RpcSendType {
    /// when want to connect a websocke, send from TDN to outside.
    /// params: url, peer, signature and peer `connect_info`.
    Connect(String, PeerId, String, String),
    /// when want to close websocket, send from TDN to outside.
    /// params: connection uid.
    Leave(u64),
    /// when received a data from a connection, include http and websocket
    /// params: connection uid, request params
    Event(u64, RpcParam),
}

/// channel message send to chamomile network.
#[derive(Debug)]
pub enum NetworkType {
    /// when outside want to add a peer to bootstrap and DHT.
    /// if connected, TDN will add it to boostrap and DHT.
    /// params: `peer`.
    Connect(Peer),
    /// when outside donnot want to remove peer. use it to force close.
    /// params: `Peer(socket)`.
    DisConnect(Peer),
    /// when need broadcast a data to all network, TDN support some
    /// common algorithm, use it, donnot worry.
    /// params: `broadcast_type` and `data_bytes`
    Broadcast(Broadcast, Vec<u8>),
    /// Request for return the network current state info.
    /// params: request type, and return channel's sender (async).
    NetworkState(StateRequest, Sender<StateResponse>),
    /// When receive `ReceiveMessage::NetworkLost`, want to reboot network, it can use.
    NetworkReboot,
    /// Stop & close p2p network.
    NetworkStop,
    /// add group to TDN control. multiple group use.
    #[cfg(any(feature = "multiple", feature = "full"))]
    AddGroup(GroupId),
    /// remove group from TDN control. multiple group use.
    #[cfg(any(feature = "multiple", feature = "full"))]
    DelGroup(GroupId),
}

/// channel message send to TDN for std version.
#[cfg(feature = "std")]
#[derive(Debug)]
pub enum SendMessage {
    /// P2P network with same PeerId.
    Own(SendType),
    /// Group: GroupMessage.
    Group(SendType),
    /// Layer: LayerMessage.
    Layer(GroupId, SendType),
    /// RPC: RpcSendType
    Rpc(RpcSendType),
    /// Network: Control the Network state.
    Network(NetworkType),
}

/// channel message receive from TDN for std version.
#[cfg(feature = "std")]
#[derive(Debug)]
pub enum ReceiveMessage {
    /// P2P network with same PeerId.
    Own(RecvType),
    /// Group: GroupMessage.
    Group(RecvType),
    /// Layer: LayerMessage. Take care of `Leave`.
    Layer(GroupId, GroupId, RecvType),
    /// RPC: RpcRecvType
    Rpc(RpcRecvType),
    /// when network lost all DHT network and direct stables. will tell outside.
    NetworkLost,
}

/// channel message send to TDN for single version.
#[cfg(feature = "single")]
#[derive(Debug)]
pub enum SendMessage {
    /// P2P network with same PeerId.
    Own(SendType),
    /// Group: GroupMessage.
    Group(SendType),
    /// RPC: RpcSendType
    Rpc(RpcSendType),
    /// Network: Control the Network state.
    Network(NetworkType),
}

/// channel message receive from TDN for single version.
#[cfg(feature = "single")]
#[derive(Debug)]
pub enum ReceiveMessage {
    /// P2P network with same PeerId.
    Own(RecvType),
    /// Group: GroupMessage.
    Group(RecvType),
    /// RPC: RpcRecvType
    Rpc(RpcRecvType),
    /// when network lost all DHT network and direct stables. will tell outside.
    NetworkLost,
}

/// channel message send to TDN for multiple version.
#[cfg(feature = "multiple")]
#[derive(Debug)]
pub enum SendMessage {
    /// P2P network with same PeerId.
    Own(SendType),
    /// Group: GroupMessage.
    Group(GroupId, SendType),
    /// RPC: RpcSendType
    Rpc(RpcSendType),
    /// Network: Control the Network state.
    Network(NetworkType),
}

/// channel message receive from TDN for multiple version.
#[cfg(feature = "multiple")]
#[derive(Debug)]
pub enum ReceiveMessage {
    /// P2P network with same PeerId.
    Own(RecvType),
    /// Group: GroupMessage.
    Group(GroupId, RecvType),
    /// RPC: RpcRecvType
    Rpc(RpcRecvType),
    /// when network lost all DHT network and direct stables. will tell outside.
    NetworkLost,
}

/// channel message send to TDN for full version.
#[cfg(feature = "full")]
#[derive(Debug)]
pub enum SendMessage {
    /// P2P network with same PeerId.
    Own(SendType),
    /// Group: GroupMessage.
    Group(GroupId, SendType),
    /// Layer: LayerMessage.
    /// params: sender's id, receiver's, msg. Take care of `Leave`.
    Layer(GroupId, GroupId, SendType),
    /// RPC: RpcSendType
    Rpc(RpcSendType),
    /// Network: Control the Network state.
    Network(NetworkType),
}

/// channel message receive from TDN for full version.
#[cfg(feature = "full")]
#[derive(Debug)]
pub enum ReceiveMessage {
    /// P2P network with same PeerId.
    Own(RecvType),
    /// Group: GroupMessage.
    Group(GroupId, RecvType),
    /// Layer: LayerMessage.
    /// params: sender's, receiver's, msg.
    Layer(GroupId, GroupId, RecvType),
    /// RPC: RpcRecvType
    Rpc(RpcRecvType),
    /// when network lost all DHT network and direct stables. will tell outside.
    NetworkLost,
}
