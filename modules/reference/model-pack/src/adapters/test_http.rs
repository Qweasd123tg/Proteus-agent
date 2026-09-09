//! Loopback-only HTTP fixtures shared by auth and provider wire tests.
use std::collections::BTreeMap;

use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub(super) struct Request {
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

pub(super) async fn read(socket: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let end = loop {
        let mut chunk = [0; 4096];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        assert!(bytes.len() < 100_000);
        if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break offset + 4;
        }
    };
    let text = std::str::from_utf8(&bytes[..end]).unwrap();
    let target = text.lines().next().unwrap().to_owned();
    let headers: BTreeMap<_, _> = text
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.to_lowercase(), value.trim().to_owned()))
        })
        .collect();
    let length = headers
        .get("content-length")
        .map(|n| n.parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() < end + length {
        let mut chunk = [0; 4096];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
    }
    Request {
        target,
        headers,
        body: String::from_utf8(bytes[end..end + length].to_vec()).unwrap(),
    }
}

pub(super) async fn reply(socket: &mut TcpStream, status: u16, kind: &str, body: &str) {
    socket.write_all(format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len(),
    ).as_bytes()).await.unwrap();
}

pub(super) async fn server(
    responses: Vec<(u16, &'static str, String)>,
) -> (String, JoinHandle<Vec<Request>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, kind, body) in responses {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            requests.push(read(&mut socket).await);
            reply(&mut socket, status, kind, &body).await;
        }
        requests
    });
    (url, task)
}

pub(super) fn sse(response: Value) -> (u16, &'static str, String) {
    (
        200,
        "text/event-stream",
        format!(
            "event: response.completed\ndata: {}\n\n",
            serde_json::json!({"response": response})
        ),
    )
}
