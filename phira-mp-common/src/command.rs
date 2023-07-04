use crate::{BinaryData, BinaryReader, BinaryWriter};
use anyhow::{bail, Result};
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
        if self.0 .0.is_empty()
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
        value.0 .0
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
        self.0 .0.fmt(f)
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
}

#[derive(Debug, BinaryData, Clone, Copy)]
pub enum RoomState {
    SelectChart(Option<i32>),
    WaitingForReady,
    Playing,
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
}
