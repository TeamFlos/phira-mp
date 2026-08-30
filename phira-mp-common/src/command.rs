use crate::{BinaryData, BinaryReader, BinaryWriter};
use anyhow::{Result, bail};
use half::f16;
use phira_mp_macros::BinaryData;
use std::{collections::HashMap, fmt::Display, sync::Arc};

type SResult<T> = Result<T, String>;

#[derive(Debug, Clone)]
pub struct CompactPos {
    pub(crate) x: f16,
    pub(crate) y: f16,
}

impl BinaryData for CompactPos {
    fn read_binary(r: &mut BinaryReader<'_>) -> Result<Self> {
        Ok(Self {
            x: f16::from_bits(r.read()?),
            y: f16::from_bits(r.read()?),
        })
    }

    fn write_binary(&self, w: &mut BinaryWriter<'_>) -> Result<()> {
        w.write_val(self.x.to_bits())?;
        w.write_val(self.y.to_bits())?;
        Ok(())
    }
}

impl CompactPos {
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: f16::from_f32(x),
            y: f16::from_f32(y),
        }
    }

    pub fn x(&self) -> f32 {
        self.x.to_f32()
    }

    pub fn y(&self) -> f32 {
        self.y.to_f32()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Varchar<const N: usize>(String);
impl<const N: usize> Display for Varchar<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl<const N: usize> TryFrom<String> for Varchar<N> {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > N {
            bail!("string too long");
        }
        Ok(Self(value))
    }
}
impl<const N: usize> BinaryData for Varchar<N> {
    fn read_binary(r: &mut BinaryReader<'_>) -> Result<Self> {
        let len = r.uleb()? as usize;
        if len > N {
            bail!("string too long");
        }
        Ok(Varchar(String::from_utf8_lossy(r.take(len)?).into_owned()))
    }

    fn write_binary(&self, w: &mut BinaryWriter<'_>) -> Result<()> {
        w.write(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomId(Varchar<20>);
impl RoomId {
    fn validate(self) -> Result<Self> {
        if self.0.0.is_empty()
            || !self
                .0
                .0
                .chars()
                .all(|it| it == '-' || it == '_' || it.is_ascii_alphanumeric())
        {
            bail!("invalid room id");
        }
        Ok(self)
    }
}

impl From<RoomId> for String {
    fn from(value: RoomId) -> Self {
        value.0.0
    }
}

impl TryFrom<String> for RoomId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        Self(value.try_into()?).validate()
    }
}

impl Display for RoomId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.0.fmt(f)
    }
}

impl BinaryData for RoomId {
    fn read_binary(r: &mut BinaryReader<'_>) -> Result<Self> {
        Self(Varchar::read_binary(r)?).validate()
    }

    fn write_binary(&self, w: &mut BinaryWriter<'_>) -> Result<()> {
        self.0.write_binary(w)
    }
}

impl<const N: usize> Varchar<N> {
    pub fn into_inner(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, BinaryData)]
pub struct TouchFrame {
    pub time: f32,
    pub points: Vec<(i8, CompactPos)>,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, BinaryData)]
pub enum Judgement {
    Perfect,
    Good,
    Bad,
    Miss,
    HoldPerfect,
    HoldGood,
}

#[derive(Debug, Clone, BinaryData)]
pub struct JudgeEvent {
    pub time: f32,
    pub line_id: u32,
    pub note_id: u32,
    pub judgement: Judgement,
}

#[derive(Debug, BinaryData)]
pub enum ClientCommand {
    Ping,

    Authenticate { token: Varchar<32> },
    Chat { message: Varchar<200> },

    Touches { frames: Arc<Vec<TouchFrame>> },
    Judges { judges: Arc<Vec<JudgeEvent>> },

    CreateRoom { id: RoomId },
    JoinRoom { id: RoomId, monitor: bool },
    LeaveRoom,
    LockRoom { lock: bool },
    CycleRoom { cycle: bool },

    SelectChart { id: i32 },
    RequestStart,
    Ready,
    CancelReady,
    Played { id: i32 },
    Abort,

    // LocalChart: 房主选择本地谱面分享给房间内玩家（id 为随机 UUID，8-4-4-4-12）
    SelectLocalChart { id: Varchar<40>, name: Varchar<64> },
    // LocalChart: 房主改选在线谱面，取消本地谱面分享
    SelectOnlineChart { id: i32 },
    // LocalChart: 房主通知服务端已就绪作为下载服务器，可以开始向玩家提供下载
    SendChart { addr: String, port: u16 },
    // LocalChart: 玩家通知服务端谱面下载完成，可以就绪
    DownloadReady,

    // 本地谱面分享：房主上传谱面包（经 game 连接，兼容内网穿透）
    UploadChart { id: Varchar<40>, data: Vec<u8> },
    // 本地谱面分享：玩家请求下载谱面包
    DownloadChart { id: Varchar<40> },
    // LocalChart: 房主取消本地谱面分享（删除服务端缓存、重置所有玩家就绪状态）
    CancelLocalChart,
    // LocalChart: 玩家取消已就绪（尚未开始游玩前可取消）
    CancelDownloadReady,

    // 新版客户端直传最终成绩；追加在枚举末尾，保持原版命令编号兼容。
    PlayedWithScore {
        id: i32,
        score: u32,
        accuracy: f32,
        full_combo: bool,
        max_combo: u32,
        perfect: u32,
        good: u32,
        bad: u32,
        miss: u32,
    },
}

#[derive(Clone, Debug, BinaryData)]
pub enum Message {
    Chat {
        user: i32,
        content: String,
    },
    CreateRoom {
        user: i32,
    },
    JoinRoom {
        user: i32,
        name: String,
    },
    LeaveRoom {
        user: i32,
        name: String,
    },
    NewHost {
        user: i32,
    },
    SelectChart {
        user: i32,
        name: String,
        id: i32,
    },
    GameStart {
        user: i32,
    },
    Ready {
        user: i32,
    },
    CancelReady {
        user: i32,
    },
    CancelGame {
        user: i32,
    },
    StartPlaying,
    Played {
        user: i32,
        score: i32,
        accuracy: f32,
        full_combo: bool,
    },
    GameEnd,
    Abort {
        user: i32,
    },
    LockRoom {
        lock: bool,
    },
    CycleRoom {
        cycle: bool,
    },
    // LocalChart: 房主选择了本地谱面
    SelectLocalChart {
        user: i32,
        name: String,
        id: String,
    },
    // LocalChart: 房主通知服务端上传服务器已就绪
    SendChart {
        user: i32,
    },
    // LocalChart: 玩家下载完成，通知房主
    DownloadReady {
        user: i32,
    },
}

#[derive(Debug, BinaryData, Clone, Copy)]
pub enum RoomState {
    SelectChart(Option<i32>),
    WaitingForReady,
    Playing,
    // 房主选择本地谱面，房间进入本地分享阶段（谱面 id 通过 ChangeLocalChart 命令下发）
    LocalChart,
}

impl Default for RoomState {
    fn default() -> Self {
        Self::SelectChart(None)
    }
}

#[derive(Clone, Debug, BinaryData)]
pub struct UserInfo {
    pub id: i32,
    pub name: String,
    pub monitor: bool,
}

#[derive(Debug, BinaryData, Clone)]
pub struct ClientRoomState {
    pub id: RoomId,
    pub state: RoomState,
    pub live: bool,
    pub locked: bool,
    pub cycle: bool,
    pub is_host: bool,
    pub is_ready: bool,
    pub users: HashMap<i32, UserInfo>,
}

#[derive(Debug, BinaryData, Clone)]
pub struct JoinRoomResponse {
    pub state: RoomState,
    pub users: Vec<UserInfo>,
    pub live: bool,
}

#[derive(Clone, Debug, BinaryData)]
pub enum ServerCommand {
    Pong,

    Authenticate(SResult<(UserInfo, Option<ClientRoomState>)>),
    Chat(SResult<()>),

    Touches {
        player: i32,
        frames: Arc<Vec<TouchFrame>>,
    },
    Judges {
        player: i32,
        judges: Arc<Vec<JudgeEvent>>,
    },

    Message(Message),
    ChangeState(RoomState),
    ChangeHost(bool),

    CreateRoom(SResult<()>),
    JoinRoom(SResult<JoinRoomResponse>),
    OnJoinRoom(UserInfo),
    LeaveRoom(SResult<()>),
    LockRoom(SResult<()>),
    CycleRoom(SResult<()>),

    SelectChart(SResult<()>),
    RequestStart(SResult<()>),
    Ready(SResult<()>),
    CancelReady(SResult<()>),
    Played(SResult<()>),
    Abort(SResult<()>),

    // LocalChart: 服务端下发本地谱面分享状态（房间所有人、包括房主都会收到）
    ChangeLocalChart {
        local: bool,
        chart_id: String,
    },
    // LocalChart: 服务端通知房主启动本地的下载服务器（房主收到后应开 HTTP 服务器并发 SendChart）
    StartServing {
        chart_id: String,
        chart_name: String,
    },
    // LocalChart: 服务端通知各非房主玩家开始从房主下载谱面
    StartDownload {
        host_id: i32,
        host_name: String,
        addr: String,
        port: u16,
        chart_id: String,
        chart_name: String,
    },
    // LocalChart: 服务端通知房主所有玩家已下载完成，可以开始游戏
    HostReady,
    // LocalChart: 服务端通知所有客户端本地谱面分享已取消（重置就绪状态，仍停留在选谱/分享阶段）
    LocalChartCanceled,
    // 回执
    SelectLocalChart(SResult<()>),
    SelectOnlineChart(SResult<()>),
    SendChart(SResult<()>),
    DownloadReady(SResult<()>),
    CancelLocalChart(SResult<()>),
    CancelDownloadReady(SResult<()>),

    // 本地谱面分享：上传回执 / 下载数据
    UploadChart(SResult<()>),
    DownloadChart(SResult<Vec<u8>>),
}
