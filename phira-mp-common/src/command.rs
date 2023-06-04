use crate::{BinaryData, BinaryReader, BinaryWriter};
use anyhow::{bail, Result};
use half::f16;
use phira_mp_macros::BinaryData;
use std::{fmt::Display, sync::Arc};
use uuid::Uuid;

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

#[derive(Debug)]
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
        Self::try_from(r.read::<String>()?)
    }

    fn write_binary(&self, w: &mut BinaryWriter<'_>) -> Result<()> {
        w.write(&self.0)
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
    pub points: Vec<CompactPos>,
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

    Authorize { token: Varchar<32> },
    Chat { message: Varchar<200> },

    Touches { frames: Arc<Vec<TouchFrame>> },
    Judges { judges: Arc<Vec<JudgeEvent>> },

    CreateRoom,
    JoinRoom { id: Uuid },
    LeaveRoom,

    SelectChart { id: i32 },
    RequestStart,
    Ready,
    CancelReady,
    Played { id: i32 },
}

#[derive(Clone, Debug, BinaryData)]
pub enum Message {
    Chat {
        user_id: i32,
        user: String,
        content: String,
    },
    CreateRoom {
        user: String,
    },
    JoinRoom {
        user: String,
    },
    LeaveRoom {
        user: String,
    },
    NewHost {
        user: String,
    },
    SelectChart {
        user: String,
        name: String,
        id: i32,
    },
    GameStart {
        user: String,
    },
    Ready {
        user: String,
    },
    CancelReady {
        user: String,
    },
    CancelGame {
        user: String,
    },
    StartPlaying,
    Played {
        user: String,
        score: i32,
        accuracy: f32,
        full_combo: bool,
    },
    GameEnd,
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

#[derive(Debug, BinaryData, Clone)]
pub struct ClientRoomState {
    pub id: Uuid,
    pub state: RoomState,
    pub is_host: bool,
    pub is_ready: bool,
}

#[derive(Clone, Debug, BinaryData)]
pub enum ServerCommand {
    Pong,

    Authorize(SResult<Option<ClientRoomState>>),
    Chat(SResult<()>),

    Touches { frames: Arc<Vec<TouchFrame>> },
    Judges { judges: Arc<Vec<JudgeEvent>> },

    Message(Message),
    ChangeState(RoomState),
    ChangeHost(bool),

    CreateRoom(SResult<Uuid>),
    JoinRoom(SResult<RoomState>),
    LeaveRoom(SResult<()>),

    SelectChart(SResult<()>),
    RequestStart(SResult<()>),
    Ready(SResult<()>),
    CancelReady(SResult<()>),
    Played(SResult<()>),
    GameEnd,
}
