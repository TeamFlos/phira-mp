use anyhow::{Context, Error, Result};
use dashmap::DashMap;
use phira_mp_common::{
    ClientCommand, ClientRoomState, JoinRoomResponse, JudgeEvent, Message, RoomId, RoomState,
    ServerCommand, Stream, TouchFrame, UserInfo, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT,
};
use std::{
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{oneshot, Mutex, Notify, RwLock},
    task::JoinHandle,
    time,
};
use tracing::{error, trace, warn};

type Callback<T> = Mutex<Option<oneshot::Sender<T>>>;
type RCallback<T, E = String> = Mutex<Option<oneshot::Sender<Result<T, E>>>>;

pub const TIMEOUT: Duration = Duration::from_secs(7);

pub struct LivePlayer {
    pub touch_frames: Mutex<Vec<TouchFrame>>,
    pub judge_events: Mutex<Vec<JudgeEvent>>,
}

impl LivePlayer {
    pub fn new() -> Self {
        Self {
            touch_frames: Mutex::default(),
            judge_events: Mutex::default(),
        }
    }
}

struct State {
    delay: Mutex<Option<Duration>>,
    ping_notify: Notify,

    me: RwLock<Option<UserInfo>>,
    room: RwLock<Option<ClientRoomState>>,

    cb_authenticate: RCallback<(UserInfo, Option<ClientRoomState>)>,
    cb_chat: RCallback<()>,
    cb_create_room: RCallback<()>,
    cb_join_room: RCallback<JoinRoomResponse>,
    cb_leave_room: RCallback<()>,
    cb_lock_room: RCallback<()>,
    cb_cycle_room: RCallback<()>,
    cb_select_chart: RCallback<()>,
    cb_request_start: RCallback<()>,
    cb_ready: RCallback<()>,
    cb_cancel_ready: RCallback<()>,
    cb_played: RCallback<()>,
    cb_abort: RCallback<()>,

    live_players: DashMap<i32, Arc<LivePlayer>>,
    messages: Mutex<Vec<Message>>,
}

impl State {
    pub fn live_player(&self, player: i32) -> Arc<LivePlayer> {
        Arc::clone(
            &self
                .live_players
                .entry(player)
                .or_insert_with(|| Arc::new(LivePlayer::new())),
        )
    }
}

pub struct Client {
    state: Arc<State>,

    stream: Arc<Stream<ClientCommand, ServerCommand>>,

    ping_fail_count: Arc<AtomicU8>,
    ping_task_handle: JoinHandle<()>,
}

impl Client {
    pub async fn new(stream: TcpStream) -> Result<Self> {
        stream.set_nodelay(true)?;

        let state = Arc::new(State {
            delay: Mutex::default(),
            ping_notify: Notify::new(),

            me: RwLock::default(),
            room: RwLock::default(),

            cb_authenticate: Callback::default(),
            cb_chat: Callback::default(),
            cb_create_room: Callback::default(),
            cb_join_room: Callback::default(),
            cb_leave_room: Callback::default(),
            cb_lock_room: Callback::default(),
            cb_cycle_room: Callback::default(),
            cb_select_chart: Callback::default(),
            cb_request_start: Callback::default(),
            cb_ready: Callback::default(),
            cb_cancel_ready: Callback::default(),
            cb_played: Callback::default(),
            cb_abort: Callback::default(),

            live_players: DashMap::new(),
            messages: Mutex::default(),
        });
        let stream = Arc::new(
            Stream::new(
                Some(1),
                stream,
                Box::new({
                    let state = Arc::clone(&state);
                    move |_send_tx, cmd| process(Arc::clone(&state), cmd)
                }),
            )
            .await?,
        );

        let ping_fail_count = Arc::new(AtomicU8::default());
        let ping_task_handle = tokio::spawn({
            let ping_fail_count = Arc::clone(&ping_fail_count);
            let state = Arc::clone(&state);
            let stream = Arc::clone(&stream);
            async move {
                loop {
                    time::sleep(HEARTBEAT_INTERVAL).await;

                    let start = Instant::now();
                    if let Err(err) = stream.send(ClientCommand::Ping).await {
                        error!("failed to send heartbeat: {err:?}");
                    } else if time::timeout(HEARTBEAT_TIMEOUT, state.ping_notify.notified())
                        .await
                        .is_err()
                    {
                        warn!("heartbeat timeout");
                        ping_fail_count.fetch_add(1, Ordering::Relaxed);
                    } else {
                        ping_fail_count.store(0, Ordering::SeqCst);
                    }
                    let delay = start.elapsed();
                    *state.delay.lock().await = Some(delay);
                    trace!("sent heartbeat, delay: {delay:?}");
                }
            }
        });

        Ok(Self {
            state,

            stream,

            ping_fail_count,
            ping_task_handle,
        })
    }

    pub fn me(&self) -> Option<UserInfo> {
        self.state.me.blocking_read().clone()
    }

    pub fn user_name(&self, id: i32) -> String {
        self.user_name_opt(id).unwrap_or_else(|| "?".to_owned())
    }

    pub fn user_name_opt(&self, id: i32) -> Option<String> {
        self.state
            .room
            .blocking_read()
            .as_ref()
            .and_then(|it| it.users.get(&id).map(|it| it.name.clone()))
    }

    pub fn blocking_take_messages(&self) -> Vec<Message> {
        self.state.messages.blocking_lock().drain(..).collect()
    }

    pub fn blocking_state(&self) -> Option<ClientRoomState> {
        self.state.room.blocking_read().clone()
    }

    pub fn blocking_room_id(&self) -> Option<RoomId> {
        self.state
            .room
            .blocking_read()
            .as_ref()
            .map(|it| it.id.clone())
    }

    pub fn blocking_room_state(&self) -> Option<RoomState> {
        self.state.room.blocking_read().as_ref().map(|it| it.state)
    }

    pub async fn room_state(&self) -> Option<RoomState> {
        self.state.room.read().await.as_ref().map(|it| it.state)
    }

    pub fn blocking_is_host(&self) -> Option<bool> {
        self.state
            .room
            .blocking_read()
            .as_ref()
            .map(|it| it.is_host)
    }

    pub fn blocking_is_ready(&self) -> Option<bool> {
        self.state
            .room
            .blocking_read()
            .as_ref()
            .map(|it| it.is_ready)
    }

    pub async fn ping(&self) -> Result<Duration> {
        let start = Instant::now();
        self.stream.send(ClientCommand::Ping).await?;
        time::timeout(HEARTBEAT_TIMEOUT, self.state.ping_notify.notified())
            .await
            .context("heartbeat timeout")?;
        let delay = start.elapsed();
        *self.state.delay.lock().await = Some(delay);
        Ok(delay)
    }

    pub fn delay(&self) -> Option<Duration> {
        *self.state.delay.blocking_lock()
    }

    async fn rcall<R>(&self, payload: ClientCommand, cb: &RCallback<R>) -> Result<R> {
        self.stream.send(payload).await?;
        let (tx, rx) = oneshot::channel();
        *cb.lock().await = Some(tx);
        time::timeout(TIMEOUT, rx)
            .await
            .context("timeout")??
            .map_err(Error::msg)
    }

    #[inline]
    pub async fn authenticate(&self, token: impl Into<String>) -> Result<()> {
        let (me, room) = self
            .rcall(
                ClientCommand::Authenticate {
                    token: token.into().try_into()?,
                },
                &self.state.cb_authenticate,
            )
            .await?;
        *self.state.me.write().await = Some(me);
        *self.state.room.write().await = room;
        Ok(())
    }

    #[inline]
    pub async fn chat(&self, message: String) -> Result<()> {
        self.rcall(
            ClientCommand::Chat {
                message: message.try_into()?,
            },
            &self.state.cb_chat,
        )
        .await
    }

    #[inline]
    pub async fn create_room(&self, id: RoomId) -> Result<()> {
        self.rcall(
            ClientCommand::CreateRoom { id: id.clone() },
            &self.state.cb_create_room,
        )
        .await?;
        let me = self.state.me.read().await.clone().unwrap();
        *self.state.room.write().await = Some(ClientRoomState {
            id,
            state: RoomState::default(),
            live: false,
            locked: false,
            cycle: false,
            is_host: true,
            is_ready: false,
            users: std::iter::once((me.id, me)).collect(),
        });
        Ok(())
    }

    #[inline]
    pub async fn join_room(&self, id: RoomId, monitor: bool) -> Result<()> {
        let resp = self
            .rcall(
                ClientCommand::JoinRoom {
                    id: id.clone(),
                    monitor,
                },
                &self.state.cb_join_room,
            )
            .await?;
        *self.state.room.write().await = Some(ClientRoomState {
            id,
            state: resp.state,
            live: resp.live,
            locked: false,
            cycle: false,
            is_host: false,
            is_ready: false,
            users: resp.users.into_iter().map(|it| (it.id, it)).collect(),
        });
        Ok(())
    }

    #[inline]
    pub async fn leave_room(&self) -> Result<()> {
        self.rcall(ClientCommand::LeaveRoom, &self.state.cb_leave_room)
            .await?;
        *self.state.room.write().await = None;
        Ok(())
    }

    #[inline]
    pub async fn lock_room(&self, lock: bool) -> Result<()> {
        self.rcall(ClientCommand::LockRoom { lock }, &self.state.cb_lock_room)
            .await
    }

    #[inline]
    pub async fn cycle_room(&self, cycle: bool) -> Result<()> {
        self.rcall(
            ClientCommand::CycleRoom { cycle },
            &self.state.cb_cycle_room,
        )
        .await
    }

    #[inline]
    pub async fn select_chart(&self, id: i32) -> Result<()> {
        self.rcall(
            ClientCommand::SelectChart { id },
            &self.state.cb_select_chart,
        )
        .await
    }

    #[inline]
    pub async fn request_start(&self) -> Result<()> {
        self.rcall(ClientCommand::RequestStart, &self.state.cb_request_start)
            .await?;
        self.state.room.write().await.as_mut().unwrap().is_ready = true;
        Ok(())
    }

    #[inline]
    pub async fn ready(&self) -> Result<()> {
        self.rcall(ClientCommand::Ready, &self.state.cb_ready)
            .await?;
        self.state.room.write().await.as_mut().unwrap().is_ready = true;
        Ok(())
    }

    #[inline]
    pub async fn cancel_ready(&self) -> Result<()> {
        self.rcall(ClientCommand::CancelReady, &self.state.cb_cancel_ready)
            .await?;
        self.state.room.write().await.as_mut().unwrap().is_ready = false;
        Ok(())
    }

    #[inline]
    pub async fn played(&self, id: i32) -> Result<()> {
        self.rcall(ClientCommand::Played { id }, &self.state.cb_played)
            .await
    }

    #[inline]
    pub async fn abort(&self) -> Result<()> {
        self.rcall(ClientCommand::Abort, &self.state.cb_abort).await
    }

    pub fn ping_fail_count(&self) -> u8 {
        self.ping_fail_count.load(Ordering::Relaxed)
    }

    pub async fn send(&self, payload: ClientCommand) -> Result<()> {
        self.stream.send(payload).await
    }

    pub fn blocking_send(&self, payload: ClientCommand) -> Result<()> {
        self.stream.blocking_send(payload)
    }

    #[inline]
    pub fn live_player(&self, player: i32) -> Arc<LivePlayer> {
        self.state.live_player(player)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.ping_task_handle.abort();
    }
}

async fn process(state: Arc<State>, cmd: ServerCommand) {
    async fn cb<T>(cb: &Callback<T>, res: T) {
        let _ = cb.lock().await.take().unwrap().send(res);
    }
    match cmd {
        ServerCommand::Pong => {
            state.ping_notify.notify_one();
        }
        ServerCommand::Authenticate(res) => {
            cb(&state.cb_authenticate, res).await;
        }
        ServerCommand::Chat(res) => {
            cb(&state.cb_chat, res).await;
        }
        ServerCommand::Touches { player, frames } => {
            state
                .live_player(player)
                .touch_frames
                .lock()
                .await
                .extend(frames.iter().cloned());
        }
        ServerCommand::Judges { player, judges } => {
            state
                .live_player(player)
                .judge_events
                .lock()
                .await
                .extend(judges.iter().cloned());
        }
        ServerCommand::Message(msg) => {
            match msg {
                Message::LockRoom { lock } => {
                    state.room.write().await.as_mut().unwrap().locked = lock;
                }
                Message::CycleRoom { cycle } => {
                    state.room.write().await.as_mut().unwrap().cycle = cycle;
                }
                Message::LeaveRoom { user, .. } => {
                    state
                        .room
                        .write()
                        .await
                        .as_mut()
                        .unwrap()
                        .users
                        .remove(&user);
                }
                _ => {}
            }
            state.messages.lock().await.push(msg);
        }
        ServerCommand::ChangeState(room) => {
            state.live_players.clear();
            let mut guard = state.room.write().await;
            let state = guard.as_mut().unwrap();
            state.state = room;
            state.is_ready = state.is_host;
        }
        ServerCommand::ChangeHost(me_is_host) => {
            state.room.write().await.as_mut().unwrap().is_host = me_is_host;
        }

        ServerCommand::CreateRoom(res) => {
            cb(&state.cb_create_room, res).await;
        }
        ServerCommand::JoinRoom(res) => {
            cb(&state.cb_join_room, res).await;
        }
        ServerCommand::OnJoinRoom(user) => {
            if let Some(room) = state.room.write().await.as_mut() {
                room.live |= user.monitor;
                room.users.insert(user.id, user);
            }
        }
        ServerCommand::LeaveRoom(res) => {
            cb(&state.cb_leave_room, res).await;
        }
        ServerCommand::LockRoom(res) => {
            cb(&state.cb_lock_room, res).await;
        }
        ServerCommand::CycleRoom(res) => {
            cb(&state.cb_cycle_room, res).await;
        }
        ServerCommand::SelectChart(res) => {
            cb(&state.cb_select_chart, res).await;
        }
        ServerCommand::RequestStart(res) => {
            cb(&state.cb_request_start, res).await;
        }
        ServerCommand::Ready(res) => {
            cb(&state.cb_ready, res).await;
        }
        ServerCommand::CancelReady(res) => {
            cb(&state.cb_cancel_ready, res).await;
        }
        ServerCommand::Played(res) => {
            cb(&state.cb_played, res).await;
        }
        ServerCommand::Abort(res) => {
            cb(&state.cb_abort, res).await;
        }
    }
}
