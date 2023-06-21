use crate::{Chart, Record, User};
use anyhow::{bail, Result};
use phira_mp_common::{ClientRoomState, Message, RoomId, RoomState, ServerCommand};
use rand::{seq::SliceRandom, thread_rng};
use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
};
use tokio::sync::RwLock;
use tracing::{debug, info};

const ROOM_MAX_USERS: usize = 8;

#[derive(Default, Debug)]
pub enum InternalRoomState {
    #[default]
    SelectChart,
    WaitForReady {
        started: HashSet<i32>,
    },
    Playing {
        results: HashMap<i32, Record>,
        aborted: HashSet<i32>,
    },
}

impl InternalRoomState {
    pub fn to_client(&self, chart: Option<i32>) -> RoomState {
        match self {
            Self::SelectChart => RoomState::SelectChart(chart),
            Self::WaitForReady { .. } => RoomState::WaitingForReady,
            Self::Playing { .. } => RoomState::Playing,
        }
    }
}

pub struct Room {
    pub id: RoomId,
    pub host: RwLock<Weak<User>>,
    pub state: RwLock<InternalRoomState>,

    live_room: AtomicBool,

    users: RwLock<Vec<Weak<User>>>,
    pub chart: RwLock<Option<Chart>>,
}

impl Room {
    pub fn new(id: RoomId, host: Weak<User>) -> Self {
        Self {
            id,
            host: host.clone().into(),
            state: RwLock::default(),

            live_room: AtomicBool::new(false),

            users: vec![host].into(),
            chart: RwLock::default(),
        }
    }

    pub fn is_live_room(&self) -> bool {
        self.live_room.load(Ordering::SeqCst)
    }

    pub async fn client_room_state(&self) -> RoomState {
        self.state
            .read()
            .await
            .to_client(self.chart.read().await.as_ref().map(|it| it.id))
    }

    pub async fn client_state(&self, user: &User) -> ClientRoomState {
        ClientRoomState {
            id: self.id.clone(),
            state: self.client_room_state().await,
            live_room: self.is_live_room(),
            is_host: self.check_host(user).await.is_ok(),
            is_ready: matches!(&*self.state.read().await, InternalRoomState::WaitForReady { started } if started.contains(&user.id)),
        }
    }

    pub async fn on_state_change(&self) {
        self.broadcast(ServerCommand::ChangeState(self.client_room_state().await))
            .await;
    }

    pub async fn add_user(&self, user: Weak<User>) -> bool {
        let mut guard = self.users.write().await;
        guard.retain(|it| it.strong_count() > 0);
        if guard.len() >= ROOM_MAX_USERS {
            false
        } else {
            guard.push(user);
            true
        }
    }

    pub async fn users(&self) -> Vec<Arc<User>> {
        self.users
            .read()
            .await
            .iter()
            .filter_map(|it| it.upgrade())
            .collect()
    }

    pub async fn check_host(&self, user: &User) -> Result<()> {
        if self.host.read().await.upgrade().map(|it| it.id) != Some(user.id) {
            bail!("only host can do this");
        }
        Ok(())
    }

    #[inline]
    pub async fn send(&self, msg: Message) {
        self.broadcast(ServerCommand::Message(msg)).await;
    }

    pub async fn broadcast(&self, cmd: ServerCommand) {
        for session in self.users().await {
            session.try_send(cmd.clone()).await;
        }
    }

    #[inline]
    pub async fn send_as(&self, user: &User, content: String) {
        self.send(Message::Chat {
            user_id: user.id,
            user: user.name.clone(),
            content,
        })
        .await;
    }

    /// Return: should the room be dropped
    #[must_use]
    pub async fn on_user_leave(&self, user: &User) -> bool {
        self.send(Message::LeaveRoom {
            user: user.name.clone(),
        })
        .await;
        *user.room.write().await = None;
        self.users
            .write()
            .await
            .retain(|it| it.upgrade().map_or(false, |it| it.id != user.id));
        if self.check_host(user).await.is_ok() {
            info!("host disconnected!");
            let users = self.users().await;
            if users.is_empty() {
                info!("room users all disconnected, dropping room");
                return true;
            } else {
                let user = users.choose(&mut thread_rng()).unwrap();
                debug!("selected {} as host", user.id);
                *self.host.write().await = Arc::downgrade(user);
                self.send(Message::NewHost {
                    user: user.name.clone(),
                })
                .await;
                user.try_send(ServerCommand::ChangeHost(true)).await;
            }
        }
        self.check_all_ready().await;
        false
    }

    pub async fn check_all_ready(&self) {
        let guard = self.state.read().await;
        match guard.deref() {
            InternalRoomState::WaitForReady { started } => {
                if self
                    .users()
                    .await
                    .into_iter()
                    .all(|it| started.contains(&it.id))
                {
                    drop(guard);
                    info!(room = self.id.to_string(), "game start");
                    self.send(Message::StartPlaying).await;
                    *self.state.write().await = InternalRoomState::Playing {
                        results: HashMap::new(),
                        aborted: HashSet::new(),
                    };
                    self.on_state_change().await;
                }
            }
            InternalRoomState::Playing { results, aborted } => {
                if self
                    .users()
                    .await
                    .into_iter()
                    .all(|it| results.contains_key(&it.id) || aborted.contains(&it.id))
                {
                    drop(guard);
                    // TODO print results
                    self.broadcast(ServerCommand::GameEnd).await;
                    self.send(Message::GameEnd).await;
                    *self.state.write().await = InternalRoomState::SelectChart;
                    self.on_state_change().await;
                }
            }
            _ => {}
        }
    }
}
