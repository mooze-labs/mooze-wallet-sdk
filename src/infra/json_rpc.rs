use std::sync::{
    Arc, 
    atomic::{AtomicBool, Ordering}
};
use std::collections::HashMap;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{
    mpsc,
    oneshot,
    Mutex,
    Notify,
};
use thiserror::Error;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use uuid::Uuid;

type PendingWsRequests = Arc<Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>>;
type NotificationQueue = Arc<Mutex<Vec<serde_json::Value>>>;

#[derive(Debug, Error)]
pub enum RpcError {
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("Format error: {0}")]
    FormatError(String),
    #[error("Channel error: {0}")]
    ChannelError(String),
}

/// A JSON RPC client with automatic reconnection.
/// It keeps the connection alive and with exponential back-off
/// if the connection drops.
pub struct JsonRpcClient { 
    url: String,
    sender: Arc<Mutex<mpsc::UnboundedSender<Message>>>,
    pending_reqs: PendingWsRequests,
    notifications: NotificationQueue,
    notify: Arc<Notify>,
    is_connected: Arc<AtomicBool>
}

impl JsonRpcClient {
    pub fn new(url: &str) -> Result<Self, RpcError> {
        // Dummy sender. We replace this when spawning the connection manager.
        let (dummy_tx, _) = mpsc::unbounded_channel::<Message>();

        let client = JsonRpcClient {
            url: url.into(),
            sender: Arc::new(Mutex::new(dummy_tx)),
            pending_reqs: Arc::new(Mutex::new(HashMap::new())),
            notifications: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            is_connected: Arc::new(AtomicBool::new(false))
        };
        client.spawn_connection_manager();

        Ok(client)
    } 

    /// `true` if the *current* socket is up.  
    /// (May briefly be `false` while a reconnection attempt is underway.)
    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    /// Standard JSON-RPC call; returns the server's raw JSON answer.
    pub async fn call_method(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, RpcError> {
        let id = Uuid::new_v4().to_string();
        let request = json!({
            "id": id,
            "method": method,
            "params": params.unwrap_or(serde_json::Value::Null)
        });

        let msg = Message::Text(request.to_string().into());
        let (resp_tx, resp_rx) = oneshot::channel();

        // register before we send, so the read-loop can resolve immediately
        self.pending_reqs
            .lock()
            .await
            .insert(id.clone(), resp_tx);

        self.sender.lock()
            .await
            .send(msg)
            .map_err(|e| RpcError::ChannelError(e.to_string()))?;

        let response = resp_rx.await.map_err(|e| RpcError::ChannelError(e.to_string()))?;

        Ok(response)
    }

    /// Blocks until the next server notification arrives and returns its JSON.
    pub async fn wait_for_notification(&self) -> serde_json::Value {
        loop {
            {
                let mut q = self.notifications.lock().await;
                if let Some(notif) = q.pop() {
                    return notif;
                }
            }
            self.notify.notified().await;
        }
    }

    // ------------------------------------------------------------------------
    // Internal – connection manager
    // ------------------------------------------------------------------------

    /// Detached task that owns the reconnect loop forever.
    fn spawn_connection_manager(&self) {
        let url = self.url.clone();
        let sender_handle = self.sender.clone();
        let pending = self.pending_reqs.clone();
        let notifications = self.notifications.clone();
        let notify = self.notify.clone();
        let connected_flag = self.is_connected.clone();

        tokio::spawn(async move {
            // Exponential back-off (1 s → 2 s → … max 30 s).
            let mut backoff = Duration::from_secs(1);

            loop {
                match connect_async(&url).await {
                    Ok((ws_stream, _)) => {
                        connected_flag.store(true, Ordering::SeqCst);

                        // we have a socket – split it
                        let (mut write, mut read) = ws_stream.split();
                        // brand-new mpsc channel for *this* connection
                        let (tx, mut rx) = mpsc::unbounded_channel();
                        let pinger_tx = tx.clone();  // Clone before moving tx

                        // make the new sender available to everybody
                        *sender_handle.lock().await = tx.clone();

                        // TASK 1 – forward queued messages to the network
                        let writer = tokio::spawn(async move {
                            while let Some(msg) = rx.recv().await {
                                if let Err(e) = write.send(msg).await {
                                    eprintln!("WebSocket write error: {e}");
                                    break;
                                }
                            }
                        });

                        // clones for the reader task
                        let pending_read = pending.clone();
                        let notifications_r = notifications.clone();
                        let notify_r = notify.clone();

                        // TASK 2 – process messages coming *from* the network
                        let reader = tokio::spawn(async move {
                            while let Some(Ok(msg)) = read.next().await {
                                match msg {
                                    Message::Text(txt) => {
                                        match serde_json::from_str::<serde_json::Value>(&txt) {
                                            Ok(value) => {
                                                // response or notification?
                                                if let Some(id) =
                                                    value.get("id").and_then(|v| v.as_str())
                                                {
                                                    if let Some(tx) = pending_read
                                                        .lock()
                                                        .await
                                                        .remove(id)
                                                    {
                                                        let _ = tx.send(value);
                                                    }
                                                } else {
                                                    notifications_r.lock().await.push(value);
                                                    notify_r.notify_waiters();
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("Malformed JSON from server: {e}\n{txt}");
                                            }
                                        }
                                    }
                                    Message::Ping(p) => {
                                        // reply immediately
                                        let _ = tx.send(Message::Pong(p));
                                    }
                                    Message::Close(_) => break,
                                    _ => {}
                                }
                            }
                        });

                        // TASK 3 – keep-alive ping every 10 s
                        let pinger = tokio::spawn(async move {
                            let mut ticker = tokio::time::interval(Duration::from_secs(10));
                            loop {
                                ticker.tick().await;
                                if pinger_tx
                                    .send(Message::Ping(Vec::<u8>::new().into()))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        });

                        // Wait until *either* half fails; drop the rest.
                        tokio::select! {
                            _ = writer => {},
                            _ = reader => {},
                            _ = pinger => {},
                        }

                        connected_flag.store(false, Ordering::SeqCst);
                        // Let the server a moment to tear down ; then start reconnect loop.
                    }
                    Err(e) => {
                        eprintln!("Could not connect to {url}: {e}");
                    }
                }

                // back-off before next dial attempt
                sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        });
    }
}