use crate::{
    l10n::{Language, LANGUAGE},
    tl, Chart, InternalRoomState, Record, Room, ServerState,
};
use anyhow::{anyhow, bail, Result};
use phira_mp_common::{
    ClientCommand, Message, RoomState, ServerCommand, Stream, HEARTBEAT_DISCONNECT_TIMEOUT,
};
use serde::Deserialize;
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    ops::DerefMut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{oneshot, Mutex, Notify, OnceCell, RwLock},
    task::JoinHandle,
    time,
};
use tracing::{debug, debug_span, error, info, trace, warn, Instrument};
use uuid::Uuid;

const HOST: &str = "https://api.phira.cn";

pub struct User {
    pub id: i32,
    pub name: String,
    pub lang: Language,

    pub server: Arc<ServerState>,
    session: RwLock<Option<Weak<Session>>>,
    pub room: RwLock<Option<Arc<Room>>>,

    pub dangle_mark: Mutex<Option<Arc<()>>>,
}

impl User {
    pub fn new(id: i32, name: String, lang: Language, server: Arc<ServerState>) -> Self {
        Self {
            id,
            name,
            lang,

            server,
            session: RwLock::default(),
            room: RwLock::default(),

            dangle_mark: Mutex::default(),
        }
    }

    pub async fn set_session(&self, session: Weak<Session>) {
        *self.session.write().await = Some(session);
        *self.dangle_mark.lock().await = None;
    }

    pub async fn try_send(&self, cmd: ServerCommand) {
        if let Some(session) = self.session.read().await.as_ref().and_then(Weak::upgrade) {
            session.try_send(cmd).await;
        } else {
            warn!("sending {cmd:?} to dangling user {}", self.id);
        }
    }

    pub async fn dangle(self: Arc<Self>) {
        warn!(user = self.id, "user dangling");
        let dangle_mark = Arc::new(());
        *self.dangle_mark.lock().await = Some(Arc::clone(&dangle_mark));
        tokio::spawn(async move {
            time::sleep(Duration::from_secs(10)).await;
            if Arc::strong_count(&dangle_mark) > 1 {
                let room = self.room.read().await.as_ref().map(Arc::clone);
                if let Some(room) = room {
                    self.server.users.write().await.remove(&self.id);
                    if room.on_user_leave(&self).await {
                        self.server.rooms.write().await.remove(&room.id);
                    }
                }
            }
        });
    }
}

pub struct Session {
    pub id: Uuid,
    pub stream: Stream<ServerCommand, ClientCommand>,
    pub user: Arc<User>,

    monitor_task_handle: JoinHandle<()>,
}

impl Session {
    pub async fn new(id: Uuid, stream: TcpStream, server: Arc<ServerState>) -> Result<Arc<Self>> {
        stream.set_nodelay(true)?;
        let this = Arc::new(OnceCell::<Arc<Session>>::new());
        let this_inited = Arc::new(Notify::new());
        let (tx, rx) = oneshot::channel::<Arc<User>>();
        let last_recv: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
        let stream = Stream::<ServerCommand, ClientCommand>::new(
            None,
            stream,
            Box::new({
                let this = Arc::clone(&this);
                let this_inited = Arc::clone(&this_inited);
                let mut tx = Some(tx);
                let server = Arc::clone(&server);
                let last_recv = Arc::clone(&last_recv);
                let waiting_for_authorize = Arc::new(AtomicBool::new(true));
                let panicked = Arc::new(AtomicBool::new(false));
                move |send_tx, cmd| {
                    let this = Arc::clone(&this);
                    let this_inited = Arc::clone(&this_inited);
                    let tx = tx.take();
                    let server = Arc::clone(&server);
                    let last_recv = Arc::clone(&last_recv);
                    let waiting_for_authorize = Arc::clone(&waiting_for_authorize);
                    let panicked = Arc::clone(&panicked);
                    async move {
                        *last_recv.lock().await = Instant::now();
                        if panicked.load(Ordering::SeqCst) {
                            return;
                        }
                        if matches!(cmd, ClientCommand::Ping) {
                            let _ = send_tx.send(ServerCommand::Pong).await;
                            return;
                        }
                        if waiting_for_authorize.load(Ordering::SeqCst) {
                            if let ClientCommand::Authorize { token } = cmd {
                                let Some(tx) = tx else { return };
                                let res: Result<()> = {
                                    let this = Arc::clone(&this);
                                    let server = Arc::clone(&server);
                                    async move {
                                        let token = token.into_inner();
                                        if token.len() != 32 {
                                            bail!("invalid token");
                                        }
                                        debug!("session {id}: authorize {token}");
                                        #[derive(Debug, Deserialize)]
                                        struct UserInfo {
                                            id: i32,
                                            name: String,
                                            language: String,
                                        }
                                        let resp: Result<UserInfo> = async {
                                            Ok(reqwest::Client::new()
                                                .get(format!("{HOST}/me"))
                                                .header(
                                                    reqwest::header::AUTHORIZATION,
                                                    format!("Bearer {token}"),
                                                )
                                                .send()
                                                .await?
                                                .error_for_status()?
                                                .json()
                                                .await?)
                                        }
                                        .await;
                                        let resp = match resp {
                                            Ok(resp) => resp,
                                            Err(err) => {
                                                warn!("failed to fetch info: {err:?}");
                                                bail!("failed to fetch info");
                                            }
                                        };
                                        debug!("session {id} <- {resp:?}");
                                        let mut users_guard = server.users.write().await;
                                        if let Some(user) = users_guard.get(&resp.id) {
                                            info!("reconnect");
                                            let _ = tx.send(Arc::clone(user));
                                            this_inited.notified().await;
                                            user.set_session(Arc::downgrade(this.get().unwrap()))
                                                .await;
                                        } else {
                                            let user = Arc::new(User::new(
                                                resp.id,
                                                resp.name,
                                                resp.language
                                                    .parse()
                                                    .map(Language)
                                                    .unwrap_or_default(),
                                                Arc::clone(&server),
                                            ));
                                            let _ = tx.send(Arc::clone(&user));
                                            this_inited.notified().await;
                                            user.set_session(Arc::downgrade(this.get().unwrap()))
                                                .await;
                                            users_guard.insert(resp.id, user);
                                        }
                                        Ok(())
                                    }
                                }
                                .await;
                                if let Err(err) = res {
                                    warn!("failed to authorize: {err:?}");
                                    let _ = send_tx
                                        .send(ServerCommand::Authorize(Err(err.to_string())))
                                        .await;
                                    panicked.store(true, Ordering::SeqCst);
                                    if let Err(err) = server.lost_con_tx.send(id).await {
                                        error!("failed to mark lost connection ({id}): {err:?}");
                                    }
                                } else {
                                    let user = &this.get().unwrap().user;
                                    let room_state = match user.room.read().await.as_ref() {
                                        Some(room) => Some(room.client_state(user).await),
                                        None => None,
                                    };
                                    let _ = send_tx
                                        .send(ServerCommand::Authorize(Ok(room_state)))
                                        .await;
                                    waiting_for_authorize.store(false, Ordering::SeqCst);
                                }
                                return;
                            } else {
                                warn!("packet before authorization, ignoring: {cmd:?}");
                                return;
                            }
                        }
                        let user = this.get().map(|it| Arc::clone(&it.user)).unwrap();
                        if let Some(resp) = LANGUAGE
                            .scope(Arc::new(user.lang.clone()), process(user, cmd))
                            .await
                        {
                            if let Err(err) = send_tx.send(resp).await {
                                error!(
                                    "failed to handle message, aborting connection {id}: {err:?}",
                                );
                                panicked.store(true, Ordering::SeqCst);
                                if let Err(err) = server.lost_con_tx.send(id).await {
                                    error!("failed to mark lost connection ({id}): {err:?}");
                                }
                            }
                        }
                    }
                }
            }),
        )
        .await?;
        let monitor_task_handle = tokio::spawn({
            let last_recv = Arc::clone(&last_recv);
            async move {
                loop {
                    let recv = *last_recv.lock().await;
                    time::sleep_until((recv + HEARTBEAT_DISCONNECT_TIMEOUT).into()).await;

                    if *last_recv.lock().await + HEARTBEAT_DISCONNECT_TIMEOUT > Instant::now() {
                        continue;
                    }

                    if let Err(err) = server.lost_con_tx.send(id).await {
                        error!("failed to mark lost connection ({id}): {err:?}");
                    }
                    break;
                }
            }
        });

        let user = rx.await?;

        let res = Arc::new(Self {
            id,
            stream,
            user,

            monitor_task_handle,
        });
        let _ = this.set(Arc::clone(&res));
        this_inited.notify_one();
        Ok(res)
    }

    pub fn version(&self) -> u8 {
        self.stream.version()
    }

    pub fn name(&self) -> &str {
        &self.user.name
    }

    pub async fn try_send(&self, cmd: ServerCommand) {
        if let Err(err) = self.stream.send(cmd).await {
            error!("failed to deliver command to {}: {err:?}", self.id);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.monitor_task_handle.abort();
    }
}

async fn process(user: Arc<User>, cmd: ClientCommand) -> Option<ServerCommand> {
    #[inline]
    fn err_to_str<T>(result: Result<T>) -> Result<T, String> {
        result.map_err(|it| it.to_string())
    }

    macro_rules! get_room {
        (~ $d:ident) => {
            let $d = match user.room.read().await.as_ref().map(Arc::clone) {
                Some(room) => room,
                None => {
                    warn!("no room");
                    return None;
                }
            };
        };
        ($d:ident) => {
            let $d = user
                .room
                .read()
                .await
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| anyhow!("no room"))?;
        };
        ($d:ident, $($pt:tt)*) => {
            let $d = user
                .room
                .read()
                .await
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| anyhow!("no room"))?;
            if !matches!(&*$d.state.read().await, $($pt)*) {
                bail!("invalid state");
            }
        };
    }
    match cmd {
        ClientCommand::Ping => unreachable!(),
        ClientCommand::Authorize { .. } => Some(ServerCommand::Authorize(Err(
            "repeated authorize".to_owned(),
        ))),
        ClientCommand::Chat { message } => {
            let res: Result<()> = async move {
                get_room!(room);
                room.send_as(&user, message.into_inner()).await;
                Ok(())
            }
            .await;
            Some(ServerCommand::Chat(err_to_str(res)))
        }
        ClientCommand::Touches { frames } => {
            get_room!(~ room);
            if room.is_live_room() {
                debug!("received {} touch events from {}", frames.len(), user.id);
                room.broadcast(ServerCommand::Touches { frames }).await;
            } else {
                warn!("received touch events in non-live mode");
            }
            None
        }
        ClientCommand::Judges { judges } => {
            get_room!(~ room);
            if room.is_live_room() {
                debug!("received {} judge events from {}", judges.len(), user.id);
                room.broadcast(ServerCommand::Judges { judges }).await;
            } else {
                warn!("received judge events in non-live mode");
            }
            None
        }
        ClientCommand::CreateRoom { id } => {
            let res: Result<()> = async move {
                let mut room_guard = user.room.write().await;
                if room_guard.is_some() {
                    bail!("already in room");
                }

                let mut map_guard = user.server.rooms.write().await;
                let room = Arc::new(Room::new(id.clone(), Arc::downgrade(&user)));
                match map_guard.entry(id.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(Arc::clone(&room));
                    }
                    Entry::Occupied(_) => {
                        bail!(tl!("create-id-occupied"));
                    }
                }
                room.send(Message::CreateRoom {
                    user: user.name.clone(),
                })
                .await;
                drop(map_guard);
                *room_guard = Some(room);

                info!(user = user.id, room = id.to_string(), "user create room");
                Ok(())
            }
            .await;
            Some(ServerCommand::CreateRoom(err_to_str(res)))
        }
        ClientCommand::JoinRoom { id } => {
            let res: Result<RoomState> = async move {
                let mut room_guard = user.room.write().await;
                if room_guard.is_some() {
                    bail!("already in room");
                }
                let room = user.server.rooms.read().await.get(&id).map(Arc::clone);
                let Some(room) = room else { bail!("room not found") };
                if !matches!(*room.state.read().await, InternalRoomState::SelectChart) {
                    bail!(tl!("join-game-ongoing"));
                }
                if !room.add_user(Arc::downgrade(&user)).await {
                    bail!(tl!("join-room-full"));
                }
                info!(user = user.id, room = id.to_string(), "user join room");
                room.send(Message::JoinRoom {
                    user: user.name.clone(),
                })
                .await;
                *room_guard = Some(Arc::clone(&room));
                Ok(room.client_room_state().await)
            }
            .await;
            Some(ServerCommand::JoinRoom(err_to_str(res)))
        }
        ClientCommand::LeaveRoom => {
            let res: Result<()> = async move {
                get_room!(room);
                // TODO is this necessary?
                // if !matches!(*room.state.read().await, InternalRoomState::SelectChart) {
                // bail!("game ongoing, can't leave");
                // }
                info!(
                    user = user.id,
                    room = room.id.to_string(),
                    "user leave room"
                );
                if room.on_user_leave(&user).await {
                    user.server.rooms.write().await.remove(&room.id);
                }
                Ok(())
            }
            .await;
            Some(ServerCommand::LeaveRoom(err_to_str(res)))
        }
        ClientCommand::SelectChart { id } => {
            let res: Result<()> = async move {
                get_room!(room, InternalRoomState::SelectChart);
                room.check_host(&user).await?;
                let span = debug_span!(
                    "select chart",
                    user = user.id,
                    room = room.id.to_string(),
                    chart = id,
                );
                async move {
                    trace!("fetch");
                    let res: Chart = reqwest::get(format!("{HOST}/chart/{id}"))
                        .await?
                        .error_for_status()?
                        .json()
                        .await?;
                    debug!("chart is {res:?}");
                    room.send(Message::SelectChart {
                        user: user.name.clone(),
                        name: res.name.clone(),
                        id: res.id,
                    })
                    .await;
                    *room.chart.write().await = Some(res);
                    room.on_state_change().await;
                    Ok(())
                }
                .instrument(span)
                .await
            }
            .await;
            Some(ServerCommand::SelectChart(err_to_str(res)))
        }

        ClientCommand::RequestStart => {
            let res: Result<()> = async move {
                get_room!(room, InternalRoomState::SelectChart);
                room.check_host(&user).await?;
                if room.chart.read().await.is_none() {
                    bail!(tl!("start-no-chart-selected"));
                }
                debug!(room = room.id.to_string(), "room wait for ready");
                room.send(Message::GameStart {
                    user: user.name.clone(),
                })
                .await;
                if room.users().await.len() == 1 {
                    info!(room = room.id.to_string(), "single game start");
                    room.send(Message::StartPlaying).await;
                    *room.state.write().await = InternalRoomState::Playing {
                        results: HashMap::new(),
                    };
                    room.on_state_change().await;
                } else {
                    *room.state.write().await = InternalRoomState::WaitForReady {
                        started: std::iter::once(user.id).collect::<HashSet<_>>(),
                    };
                    room.on_state_change().await;
                }
                Ok(())
            }
            .await;
            Some(ServerCommand::RequestStart(err_to_str(res)))
        }
        ClientCommand::Ready => {
            let res: Result<()> = async move {
                get_room!(room);
                let mut guard = room.state.write().await;
                if let InternalRoomState::WaitForReady { started } = guard.deref_mut() {
                    if !started.insert(user.id) {
                        bail!("already ready");
                    }
                    room.send(Message::Ready {
                        user: user.name.clone(),
                    })
                    .await;
                    drop(guard);
                    room.check_all_ready().await;
                }
                Ok(())
            }
            .await;
            Some(ServerCommand::Ready(err_to_str(res)))
        }
        ClientCommand::CancelReady => {
            let res: Result<()> = async move {
                get_room!(room);
                let mut guard = room.state.write().await;
                if let InternalRoomState::WaitForReady { started } = guard.deref_mut() {
                    if !started.remove(&user.id) {
                        bail!("not ready");
                    }
                    if room.check_host(&user).await.is_ok() {
                        room.send(Message::CancelGame {
                            user: user.name.clone(),
                        })
                        .await;
                        *guard = InternalRoomState::SelectChart;
                        drop(guard);
                        room.on_state_change().await;
                    } else {
                        room.send(Message::CancelReady {
                            user: user.name.clone(),
                        })
                        .await;
                    }
                }
                Ok(())
            }
            .await;
            Some(ServerCommand::CancelReady(err_to_str(res)))
        }
        ClientCommand::Played { id } => {
            let res: Result<()> = async move {
                get_room!(room);
                let res: Record = reqwest::get(format!("{HOST}/record/{id}"))
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if res.player != user.id {
                    bail!("invalid record");
                }
                debug!(
                    room = room.id.to_string(),
                    user = user.id,
                    "user played: {res:?}"
                );
                room.send(Message::Played {
                    user: user.name.clone(),
                    score: res.score,
                    accuracy: res.accuracy,
                    full_combo: res.full_combo,
                })
                .await;
                let mut guard = room.state.write().await;
                if let InternalRoomState::Playing { results } = guard.deref_mut() {
                    if results.insert(user.id, res).is_some() {
                        bail!("already uploaded");
                    }
                    drop(guard);
                    room.check_all_ready().await;
                }
                Ok(())
            }
            .await;
            Some(ServerCommand::Played(err_to_str(res)))
        }
    }
}
