use crate::{Chart, Record, User};
use anyhow::{Result, bail};
use phira_mp_common::{ClientRoomState, Message, RoomId, RoomState, ServerCommand};
use rand::seq::IndexedRandom;
use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
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
    // 房主选择了本地谱面，处于本地分享阶段；started = 已同步完成的玩家 id 集合
    LocalChart {
        started: HashSet<i32>,
    },
}

impl InternalRoomState {
    pub fn to_client(&self, chart: Option<i32>) -> RoomState {
        match self {
            Self::SelectChart => RoomState::SelectChart(chart),
            Self::WaitForReady { .. } => RoomState::WaitingForReady,
            Self::Playing { .. } => RoomState::Playing,
            Self::LocalChart { .. } => RoomState::LocalChart,
        }
    }
}

pub struct Room {
    pub id: RoomId,
    pub host: RwLock<Weak<User>>,
    pub state: RwLock<InternalRoomState>,

    pub live: AtomicBool,
    pub locked: AtomicBool,
    pub cycle: AtomicBool,

    users: RwLock<Vec<Weak<User>>>,
    monitors: RwLock<Vec<Weak<User>>>,
    pub chart: RwLock<Option<Chart>>,
    // 正在分享的本地谱面 (UUID id, 名称)
    pub local_chart: RwLock<Option<(String, String)>>,
    // 房主是否已上传本地谱面包到服务端缓存（玩家须等上传完成后才能就绪/下载）
    pub chart_uploaded: AtomicBool,
}

impl Room {
    pub fn new(id: RoomId, host: Weak<User>) -> Self {
        Self {
            id,
            host: host.clone().into(),
            state: RwLock::default(),

            live: AtomicBool::new(false),
            locked: AtomicBool::new(false),
            cycle: AtomicBool::new(false),

            users: vec![host].into(),
            monitors: Vec::new().into(),
            chart: RwLock::default(),
            local_chart: RwLock::default(),
            chart_uploaded: AtomicBool::new(false),
        }
    }

    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    pub fn is_cycle(&self) -> bool {
        self.cycle.load(Ordering::SeqCst)
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
            live: self.is_live(),
            locked: self.is_locked(),
            cycle: self.is_cycle(),
            is_host: self.check_host(user).await.is_ok(),
            is_ready: matches!(&*self.state.read().await, InternalRoomState::WaitForReady { started } if started.contains(&user.id)),
            users: self
                .users
                .read()
                .await
                .iter()
                .chain(self.monitors.read().await.iter())
                .filter_map(|it| it.upgrade().map(|it| (it.id, it.to_info())))
                .collect(),
        }
    }

    pub async fn on_state_change(&self) {
        self.broadcast(ServerCommand::ChangeState(self.client_room_state().await))
            .await;
    }

    pub async fn add_user(&self, user: Weak<User>, monitor: bool) -> bool {
        if monitor {
            let mut guard = self.monitors.write().await;
            guard.retain(|it| it.strong_count() > 0);
            guard.push(user);
            true
        } else {
            let mut guard = self.users.write().await;
            guard.retain(|it| it.strong_count() > 0);
            if guard.len() >= ROOM_MAX_USERS {
                false
            } else {
                guard.push(user);
                true
            }
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

    pub async fn monitors(&self) -> Vec<Arc<User>> {
        self.monitors
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
        debug!("broadcast {cmd:?}");
        for session in self
            .users()
            .await
            .into_iter()
            .chain(self.monitors().await.into_iter())
        {
            session.try_send(cmd.clone()).await;
        }
    }

    pub async fn broadcast_monitors(&self, cmd: ServerCommand) {
        for session in self.monitors().await {
            session.try_send(cmd.clone()).await;
        }
    }

    #[inline]
    pub async fn send_as(&self, user: &User, content: String) {
        self.send(Message::Chat {
            user: user.id,
            content,
        })
        .await;
    }

    /// Return: should the room be dropped
    #[must_use]
    pub async fn on_user_leave(&self, user: &User) -> bool {
        self.send(Message::LeaveRoom {
            user: user.id,
            name: user.name.clone(),
        })
        .await;
        *user.room.write().await = None;
        (if user.monitor.load(Ordering::SeqCst) {
            &self.monitors
        } else {
            &self.users
        })
        .write()
        .await
        .retain(|it| it.upgrade().is_some_and(|it| it.id != user.id));
        if self.check_host(user).await.is_ok() {
            info!("host disconnected!");
            // 若游戏进行中房主退出，直接结束本局，回到选谱界面，
            // 避免因其他成员未完成而卡在 Playing 状态、无法进入下一轮。
            if matches!(*self.state.read().await, InternalRoomState::Playing { .. }) {
                info!(room = self.id.to_string(), "host left during playing, ending game");
                *self.local_chart.write().await = None;
                *self.state.write().await = InternalRoomState::SelectChart;
                self.on_state_change().await;
            }
            let users = self.users().await;
            if users.is_empty() {
                info!("room users all disconnected, dropping room");
                return true;
            } else {
                let user = users.choose(&mut rand::rng()).unwrap();
                debug!("selected {} as host", user.id);
                *self.host.write().await = Arc::downgrade(user);
                self.send(Message::NewHost { user: user.id }).await;
                user.try_send(ServerCommand::ChangeHost(true)).await;
            }
        }
        self.check_all_ready().await;
        false
    }

    pub async fn reset_game_time(&self) {
        for user in self.users().await {
            user.game_time
                .store(f32::NEG_INFINITY.to_bits(), Ordering::SeqCst);
        }
    }

    pub async fn check_all_ready(&self) {
        let guard = self.state.read().await;
        match guard.deref() {
            InternalRoomState::LocalChart { started } => {
                // 本地谱面分享阶段：所有需要下载的玩家（非房主、非 monitor 观察者）
                // 就绪后，通知房主并开始游戏。若房间只有房主一人，无需下载，直接视为就绪。
                // 前提：房主必须先上传谱面包到服务端缓存（chart_uploaded）。
                let users = self.users().await;
                let all_downloaded = self.chart_uploaded.load(Ordering::SeqCst)
                    && match self.host.read().await.upgrade() {
                        Some(host) => {
                            let hid = host.id;
                            users
                                .iter()
                                .filter(|it| it.id != hid && !it.monitor.load(Ordering::SeqCst))
                                .all(|it| started.contains(&it.id))
                        }
                        None => false,
                    };
                if all_downloaded {
                    drop(guard);
                    info!(room = self.id.to_string(), "local chart all downloaded, game start");
                    // 通知房主所有玩家已下载完成，可以关闭下载连接
                    if let Some(host) = self.host.read().await.upgrade() {
                        // 游戏即将开始：删除服务端缓存的谱面包，避免占用磁盘空间
                        if let Some((chart_id, _)) = self.local_chart.read().await.clone() {
                            host.server
                                .chart_cache
                                .write()
                                .await
                                .remove(&chart_id);
                            info!(room = self.id.to_string(), chart = %chart_id, "cleared chart cache after game start");
                        }
                        host.try_send(ServerCommand::HostReady).await;
                    }
                    self.chart_uploaded.store(false, Ordering::SeqCst);
                    self.send(Message::StartPlaying).await;
                    self.reset_game_time().await;
                    *self.state.write().await = InternalRoomState::Playing {
                        results: HashMap::new(),
                        aborted: HashSet::new(),
                    };
                    self.on_state_change().await;
                }
            }
            InternalRoomState::WaitForReady { started } => {
                if self
                    .users()
                    .await
                    .into_iter()
                    .chain(self.monitors().await.into_iter())
                    .all(|it| started.contains(&it.id))
                {
                    drop(guard);
                    info!(room = self.id.to_string(), "game start");
                    self.send(Message::StartPlaying).await;
                    self.reset_game_time().await;
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
                    self.send(Message::GameEnd).await;
                    // dbg!(2);
                    *self.state.write().await = InternalRoomState::SelectChart;
                    // 清空本地谱面残留状态并通知所有客户端退出本地谱面分享，
                    // 避免本地谱面游玩结束后切换到在线谱面时状态错乱（卡在转圈/无法开始）。
                    *self.local_chart.write().await = None;
                    self.chart_uploaded.store(false, Ordering::SeqCst);
                    self.broadcast(ServerCommand::ChangeLocalChart { local: false, chart_id: String::new() })
                        .await;
                    // dbg!(3);
                    if self.is_cycle() {
                        debug!(room = self.id.to_string(), "cycling");
                        let host = Weak::clone(&*self.host.read().await);
                        let new_host = {
                            let users = self.users().await;
                            let index = users
                                .iter()
                                .position(|it| host.ptr_eq(&Arc::downgrade(it)))
                                .map(|it| (it + 1) % users.len())
                                .unwrap_or_default();
                            users.into_iter().nth(index).unwrap()
                        };
                        *self.host.write().await = Arc::downgrade(&new_host);
                        self.send(Message::NewHost { user: new_host.id }).await;
                        if let Some(old) = host.upgrade() {
                            old.try_send(ServerCommand::ChangeHost(false)).await;
                        }
                        new_host.try_send(ServerCommand::ChangeHost(true)).await;
                    }
                    self.on_state_change().await;
                }
            }
            _ => {}
        }
    }
}
