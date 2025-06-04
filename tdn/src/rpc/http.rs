use rand_chacha::{
    rand_core::{RngCore, SeedableRng},
    ChaChaRng,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt, Result},
    net::{TcpListener, TcpStream},
    sync::{mpsc::Sender, oneshot, RwLock},
};
//use std::time::Duration;

use tdn_types::{primitives::PeerId, rpc::parse_jsonrpc};

use super::RpcMessage;

pub(crate) async fn http_listen(
    index: Option<PathBuf>,
    send: Sender<RpcMessage>,
    listener: TcpListener,
) -> Result<()> {
    let homepage = if let Some(path) = index {
        fs::read_to_string(path)
            .await
            .unwrap_or("Error Homepage.".to_owned())
    } else {
        "No Homepage.".to_owned()
    };
    let homelink = Arc::new(RwLock::new(homepage));

    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(http_connection(
            homelink.clone(),
            send.clone(),
            stream,
            addr,
        ));
    }

    Ok(())
}

enum HTTP<'a> {
    Ok(usize, httparse::Request<'a, 'a>),
    NeedMore(usize, usize),
}

fn parse_req<'a>(
    src: &'a [u8],
    req_parsed_headers: &'a mut [httparse::Header<'a>],
) -> std::result::Result<HTTP<'a>, &'a str> {
    let mut req = httparse::Request::new(req_parsed_headers);
    let status = req.parse(&src).map_err(|_| "HTTP parse error")?;

    let content_length_headers: Vec<httparse::Header> = req
        .headers
        .iter()
        .filter(|header| {
            let name = header.name.to_ascii_lowercase();
            name == "content-length"
        })
        .cloned()
        .collect();

    if content_length_headers.len() < 1 {
        return Err("HTTP header is invalid");
    }

    let length_bytes = content_length_headers.first().unwrap().value;
    let mut length_string = String::new();

    for b in length_bytes {
        length_string.push(*b as char);
    }

    let length = length_string
        .parse::<usize>()
        .map_err(|_| "HTTP length is invalid")?;

    let amt = match status {
        httparse::Status::Complete(amt) => amt,
        httparse::Status::Partial => return Err("HTTP parse error"),
    };

    if src[amt..].len() >= length {
        return Ok(HTTP::Ok(amt, req));
    }

    Ok(HTTP::NeedMore(amt, length))
}

async fn http_connection(
    _homelink: Arc<RwLock<String>>,
    send: Sender<RpcMessage>,
    mut stream: TcpStream,
    addr: SocketAddr,
) -> Result<()> {
    debug!("DEBUG: HTTP connection established: {}", addr);
    let mut rng = ChaChaRng::from_entropy();
    let id: u64 = rng.next_u64();
    let (s_send, s_recv) = oneshot::channel();

    let mut buf = vec![];

    // TODO add timeout
    let mut tmp_buf = vec![0u8; 1024];
    let n = stream.read(&mut tmp_buf).await?;
    let mut h_peer = PeerId::default();
    let mut req_parsed_headers = [httparse::EMPTY_HEADER; 16];
    let body = match parse_req(&tmp_buf[..n], &mut req_parsed_headers) {
        Ok(HTTP::NeedMore(amt, len)) => {
            buf.extend(&tmp_buf[amt..n]);
            loop {
                let mut tmp = vec![0u8; 1024];
                let n = stream.read(&mut tmp).await?;
                buf.extend(&tmp[..n]);
                if buf.len() >= len {
                    break;
                }
            }
            &buf[..]
        }
        Ok(HTTP::Ok(amt, req)) => {
            for header in req.headers {
                let name = header.name.to_ascii_uppercase();
                if name == "X-PEER" {
                    if let Some(p) = String::from_utf8(header.value.to_vec())
                        .ok()
                        .and_then(|s| PeerId::from_hex(&s).ok())
                    {
                        h_peer = p;
                    }
                }
            }

            &tmp_buf[amt..n]
        }
        Err(e) => {
            info!("TDN: HTTP JSONRPC parse error: {}", e);
            return Ok(());
        }
    };

    let msg = String::from_utf8_lossy(body);
    let res = "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin:*\r\nCross-Origin-Resource-Policy:cross-origin\r\nContent-Type:application/json;charset=UTF-8\r\n\r\n";

    match parse_jsonrpc((*msg).to_string()) {
        Ok((peer, rpc_param)) => {
            let real_peer = if peer != PeerId::default() {
                peer
            } else {
                h_peer
            };
            send.send(RpcMessage::Request(id, real_peer, rpc_param, Some(s_send)))
                .await
                .expect("Http to Rpc channel closed");
        }
        Err((err, id)) => {
            stream
                .write(format!("{}{}", res, err.json(id).to_string()).as_bytes())
                .await?;
            let _ = stream.flush().await;
            stream.shutdown().await?;
        }
    }

    if let Ok(msg) = s_recv.await {
        let param = match msg {
            RpcMessage::Response(param) => param,
            _ => Default::default(),
        };
        let _ = stream.write(format!("{}{}", res, param).as_bytes()).await?;
        let _ = stream.flush().await;
        stream.shutdown().await?;
    }

    Ok(())
}
