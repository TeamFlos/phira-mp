use crate::{vacant_entry, IdMap, Room, SafeMap, Session, User};
use anyhow::Result;
use phira_mp_common::RoomId;
use serde::Deserialize;
use std::sync::Arc;
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct Chart {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Record {
    pub id: i32,
    pub player: i32,
    pub score: i32,
    pub perfect: i32,
    pub good: i32,
    pub bad: i32,
    pub miss: i32,
    pub max_combo: i32,
    pub accuracy: f32,
    pub full_combo: bool,
    pub std: f32,
    pub std_score: f32,
}

pub struct ServerState {
    pub sessions: IdMap<Arc<Session>>,
    pub users: SafeMap<i32, Arc<User>>,

    pub rooms: SafeMap<RoomId, Arc<Room>>,

    pub lost_con_tx: mpsc::Sender<Uuid>,
}

pub struct Server {
    state: Arc<ServerState>,
    listener: TcpListener,

    lost_con_handle: JoinHandle<()>,
}

impl From<TcpListener> for Server {
    fn from(listener: TcpListener) -> Self {
        let (lost_con_tx, mut lost_con_rx) = mpsc::channel(16);
        let state = Arc::new(ServerState {
            sessions: IdMap::default(),
            users: SafeMap::default(),

            rooms: SafeMap::default(),

            lost_con_tx,
        });
        let lost_con_handle = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                while let Some(id) = lost_con_rx.recv().await {
                    warn!("lost connection with {id}");
                    if let Some(session) = state.sessions.write().await.remove(&id) {
                        if session
                            .user
                            .session
                            .read()
                            .await
                            .as_ref()
                            .map_or(false, |it| it.ptr_eq(&Arc::downgrade(&session)))
                        {
                            Arc::clone(&session.user).dangle().await;
                        }
                    }
                }
            }
        });

        Self {
            listener,
            state,

            lost_con_handle,
        }
    }
}

impl Server {
    pub async fn accept(&self) -> Result<()> {
        let (stream, addr) = self.listener.accept().await?;
        let mut guard = self.state.sessions.write().await;
        let entry = vacant_entry(&mut guard);
        let session = Session::new(*entry.key(), stream, Arc::clone(&self.state)).await?;
        info!(
            "received connections from {addr} ({}), version: {}",
            session.id,
            session.version()
        );
        entry.insert(session);
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.lost_con_handle.abort();
    }
}
