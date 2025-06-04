use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use serde_json::json;
pub type RpcParam = Value;

use crate::primitives::{HandleResult, Result, PeerId};

#[derive(Debug, Clone)]
pub enum RpcError {
    ParseError,
    InvalidRequest,
    InvalidVersion,
    InvalidResponse,
    MethodNotFound(String),
    Custom(String),
}

impl Into<std::io::Error> for RpcError {
    fn into(self) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Other, "RPC Error")
    }
}

impl From<std::io::Error> for RpcError {
    fn from(e: std::io::Error) -> RpcError {
        RpcError::Custom(format!("{}", e))
    }
}

impl From<bincode::Error> for RpcError {
    fn from(e: bincode::Error) -> RpcError {
        RpcError::Custom(format!("{}", e))
    }
}

impl From<anyhow::Error> for RpcError {
    fn from(e: anyhow::Error) -> RpcError {
        RpcError::Custom(format!("{}", e))
    }
}

impl RpcError {
    pub fn json(&self, id: u64) -> RpcParam {
        match self {
            RpcError::ParseError => json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32700,
                    "message": "Parse error"
                }
            }),
            RpcError::MethodNotFound(method) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method {} not found", method)
                }
            }),
            RpcError::InvalidRequest => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": "Invalid Request"
                }
            }),
            RpcError::InvalidVersion => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": "Unsupported JSON-RPC protocol version"
                }
            }),
            RpcError::InvalidResponse => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": "Invalid Response"
                }
            }),
            RpcError::Custom(m) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": m
                }
            }),
        }
    }
}

pub fn parse_jsonrpc(json_string: String) -> std::result::Result<(PeerId, RpcParam), (RpcError, u64)> {
    match serde_json::from_str::<RpcParam>(&json_string) {
        Ok(mut value) => {
            let id_res = value
                .get("id")
                .map(|id| {
                    id.as_u64()
                        .or(id.as_str().map(|sid| sid.parse::<u64>().ok()).flatten())
                })
                .flatten();

            if id_res.is_none() {
                return Err((RpcError::ParseError, 0));
            }
            let id = id_res.unwrap();
            *value.get_mut("id").unwrap() = id.into();

            // check if json is response
            if value.get("result").is_some() || value.get("error").is_some() {
                return Err((RpcError::InvalidResponse, id));
            }

            if value.get("method").is_none() || value.get("method").unwrap().as_str().is_none() {
                return Err((RpcError::InvalidRequest, id));
            }

            if value.get("params").is_none() {
                value["params"] = RpcParam::Array(vec![]);
            }

            let peer = value.get("peer")
                .and_then(|s| s.as_str())
                .and_then(|s| PeerId::from_hex(s).ok()).unwrap_or(PeerId::default());

            let jsonrpc = value
                .get("jsonrpc")
                .map(|v| {
                    v.as_str()
                        .map(|s| if s == "2.0" { Some(2) } else { None })
                        .flatten()
                })
                .flatten();

            if jsonrpc.is_none() {
                return Err((RpcError::InvalidVersion, id));
            }

            Ok((peer, value))
        }
        Err(_e) => Err((RpcError::ParseError, 0)),
    }
}

/// Helpe better handle rpc. Example.
/// ``` ignore
/// use tdn_types::{primitives::HandleResult, rpc::{RpcParam, RpcHandler, json}};
/// use std::sync::Arc;
///
/// struct State(u32); // Global State share in all rpc request.
///
/// let mut rpc_handler = RpcHandler::new(State(1));
/// rpc_handler.add_method("echo", |params: Vec<RpcParam>, state: Arc<State>|
///     async move {
///         assert_eq!(1, state.0);
///         Ok(HandleResult::rpc(json!(params)))
///    }
/// );
///
/// let params = json!({"method": "echo", "params": [""]});
/// async {
///     rpc_handler.handle(params).await;
/// };
///
/// // when match Message
/// match msg {
///     Message::Rpc(uid, params) => Message::Rpc(uid, params) => {
///         if let Ok(HandleResult {owns, mut rpcs, groups, layers}) = rpc_handler.handle(params).await {
///             loop {
///                 if rpcs.len() != 0 {
///                     let msg = rpcs.remove(0);
///                     sender.send(SendMessage::Rpc(uid, msg, is_ws)).await.expect("TDN channel closed");
///                 } else {
///                     break;
///             }
///         }
///     }
///     _ => {}
/// }
/// ````
pub struct RpcHandler<S: Send + Sync> {
    state: Arc<S>,
    fns: HashMap<&'static str, Box<DynFutFn<S>>>,
    //fns: HashMap<&'static str, BoxFuture<'static, RpcResult<'static>>>,
}

type RpcResult = std::result::Result<HandleResult, RpcError>;
type BoxFuture<RpcResult> = Pin<Box<dyn Future<Output = RpcResult> + Send>>;

pub trait FutFn<S>: Send + Sync + 'static {
    #[cfg(any(feature = "single", feature = "std"))]
    fn call(&self, params: Vec<RpcParam>, s: Arc<S>) -> BoxFuture<RpcResult>;

    #[cfg(any(feature = "multiple", feature = "full"))]
    fn call(
        &self,
        gid: crate::group::GroupId,
        params: Vec<RpcParam>,
        s: Arc<S>,
    ) -> BoxFuture<RpcResult>;
}

pub(crate) type DynFutFn<S> = dyn FutFn<S>;

#[cfg(any(feature = "single", feature = "std"))]
impl<S, F: Send + Sync + 'static, Fut> FutFn<S> for F
where
    F: Fn(Vec<RpcParam>, Arc<S>) -> Fut,
    Fut: Future<Output = RpcResult> + Send + 'static,
{
    fn call(&self, params: Vec<RpcParam>, s: Arc<S>) -> BoxFuture<RpcResult> {
        let fut = (self)(params, s);
        Box::pin(async move { fut.await })
    }
}

#[cfg(any(feature = "multiple", feature = "full"))]
impl<S, F: Send + Sync + 'static, Fut> FutFn<S> for F
where
    F: Fn(crate::group::GroupId, Vec<RpcParam>, Arc<S>) -> Fut,
    Fut: Future<Output = RpcResult> + Send + 'static,
{
    fn call(
        &self,
        gid: crate::group::GroupId,
        params: Vec<RpcParam>,
        s: Arc<S>,
    ) -> BoxFuture<RpcResult> {
        let fut = (self)(gid, params, s);
        Box::pin(async move { fut.await })
    }
}

impl<S: 'static + Send + Sync> RpcHandler<S> {
    pub fn new(state: S) -> RpcHandler<S> {
        Self {
            state: Arc::new(state),
            fns: HashMap::new(),
        }
    }

    pub fn new_with_state(state: Arc<S>) -> RpcHandler<S> {
        Self {
            state: state,
            fns: HashMap::new(),
        }
    }

    pub fn add_method(&mut self, name: &'static str, f: impl FutFn<S>) {
        self.fns.insert(name, Box::new(f));
    }

    pub async fn handle(&self, mut param: RpcParam) -> Result<HandleResult> {
        let id = param["id"].take().as_u64().unwrap();
        let method_s = param["method"].take();
        let method = method_s.as_str().unwrap();
        let mut new_results = HandleResult::new();

        #[cfg(any(feature = "multiple", feature = "full"))]
        let group = if let Some(group_id) = param.get("gid").and_then(|gid_v| gid_v.as_u64()) {
            group_id as crate::group::GroupId
        } else {
            new_results.rpcs.push(RpcError::InvalidRequest.json(id));
            return Ok(new_results);
        };

        if method == "rpcs" {
            let mut methods: Vec<&str> = self.fns.keys().map(|v| *v).collect();
            methods.sort();
            let params = json!(methods);

            #[cfg(any(feature = "single", feature = "std"))]
            new_results.rpcs.push(rpc_response(id, method, params));

            #[cfg(any(feature = "multiple", feature = "full"))]
            new_results
                .rpcs
                .push(rpc_response(id, method, params, group));

            return Ok(new_results);
        }

        if let RpcParam::Array(params) = param["params"].take() {
            match self.fns.get(method) {
                Some(f) => {
                    #[cfg(any(feature = "single", feature = "std"))]
                    let res = f.call(params, self.state.clone()).await;

                    #[cfg(any(feature = "multiple", feature = "full"))]
                    let res = f.call(group, params, self.state.clone()).await;

                    #[cfg(any(feature = "single", feature = "multiple"))]
                    match res {
                        Ok(HandleResult {
                            owns,
                            rpcs,
                            groups,
                            networks,
                        }) => {
                            new_results.owns = owns;
                            new_results.groups = groups;
                            new_results.networks = networks;

                            for params in rpcs {
                                // check when params is complete jsonrpc result.
                                if params.is_object() && params.get("jsonrpc").is_some() {
                                    new_results.rpcs.push(params);
                                    continue;
                                }

                                #[cfg(feature = "single")]
                                new_results.rpcs.push(rpc_response(id, method, params));

                                #[cfg(feature = "multiple")]
                                new_results
                                    .rpcs
                                    .push(rpc_response(id, method, params, group));
                            }
                        }
                        Err(err) => {
                            let mut res = err.json(id);
                            res["method"] = method.into();
                            #[cfg(feature = "multiple")]
                            let _ = res.as_object_mut().map(|v| {
                                v.insert("gid".into(), group.into());
                            });
                            new_results.rpcs.push(res);
                        }
                    }

                    #[cfg(any(feature = "full", feature = "std"))]
                    match res {
                        Ok(HandleResult {
                            owns,
                            rpcs,
                            groups,
                            layers,
                            networks,
                        }) => {
                            new_results.owns = owns;
                            new_results.groups = groups;
                            new_results.layers = layers;
                            new_results.networks = networks;

                            for params in rpcs {
                                // check when params is complete jsonrpc result.
                                if params.is_object() && params.get("jsonrpc").is_some() {
                                    new_results.rpcs.push(params);
                                    continue;
                                }

                                #[cfg(feature = "std")]
                                new_results.rpcs.push(rpc_response(id, method, params));

                                #[cfg(feature = "full")]
                                new_results
                                    .rpcs
                                    .push(rpc_response(id, method, params, group));
                            }
                        }
                        Err(err) => {
                            let mut res = err.json(id);
                            res["method"] = method.into();
                            #[cfg(feature = "full")]
                            let _ = res.as_object_mut().map(|v| {
                                v.insert("gid".into(), group.into());
                            });
                            new_results.rpcs.push(res);
                        }
                    }
                }
                None => new_results
                    .rpcs
                    .push(RpcError::MethodNotFound(method.to_owned()).json(id)),
            }
        } else {
            new_results.rpcs.push(RpcError::InvalidRequest.json(id))
        }

        Ok(new_results)
    }
}

#[cfg(any(feature = "single", feature = "std"))]
pub fn rpc_response(id: u64, method: &str, params: RpcParam) -> RpcParam {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "result": params
    })
}

#[cfg(any(feature = "multiple", feature = "full"))]
pub fn rpc_response(
    id: u64,
    method: &str,
    params: RpcParam,
    group_id: crate::group::GroupId,
) -> RpcParam {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "gid": group_id,
        "method": method,
        "result": params
    })
}

#[cfg(any(feature = "single", feature = "std"))]
pub fn rpc_request(id: u64, method: &str, params: Vec<RpcParam>) -> RpcParam {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
}

#[cfg(any(feature = "multiple", feature = "full"))]
pub fn rpc_request(
    id: u64,
    method: &str,
    params: Vec<RpcParam>,
    group_id: crate::group::GroupId,
) -> RpcParam {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "gid": group_id,
        "method": method,
        "params": params
    })
}

/// parse the result from jsonrpc response
pub fn parse_response(mut value: RpcParam) -> std::result::Result<Value, Value> {
    if value.get("result").is_some() {
        Ok(value["result"].take())
    } else {
        Err(value["error"].take())
    }
}
