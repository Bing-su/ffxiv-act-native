use std::{error::Error, marker::PhantomData, rc::Rc};

use bytes::{Bytes, BytesMut};

use crate::{RawHostApiV1, Status};

pub type PluginError = Box<dyn Error + Send + Sync + 'static>;
pub type PluginResult<T> = Result<T, PluginError>;

pub trait Plugin: Send + 'static {
    fn init(ctx: CallContext<'_>) -> PluginResult<(Self, SubscriptionSet)>
    where
        Self: Sized;

    fn on_event(&mut self, ctx: CallContext<'_>, event: Event<'_>) -> PluginResult<()>;

    fn shutdown(&mut self, ctx: CallContext<'_>) -> PluginResult<()>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SubscriptionSet(u32);

impl SubscriptionSet {
    pub const NONE: Self = Self(0);
    pub const NETWORK_RECEIVED: Self = Self(1 << 0);
    pub const NETWORK_SENT: Self = Self(1 << 1);
    pub const COMBATANT_ADDED: Self = Self(1 << 2);
    pub const COMBATANT_REMOVED: Self = Self(1 << 3);
    pub const PRIMARY_PLAYER_CHANGED: Self = Self(1 << 4);
    pub const ZONE_CHANGED: Self = Self(1 << 5);
    pub const PLAYER_STATS_CHANGED: Self = Self(1 << 6);
    pub const PARTY_LIST_CHANGED: Self = Self(1 << 7);
    pub const LOG_LINE: Self = Self(1 << 8);
    pub const PARSED_LOG_LINE: Self = Self(1 << 9);
    pub const PROCESS_CHANGED: Self = Self(1 << 10);
    pub const ALL: Self = Self((1 << 11) - 1);

    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for SubscriptionSet {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for SubscriptionSet {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    NetworkReceived = 0,
    NetworkSent = 1,
    CombatantAdded = 2,
    CombatantRemoved = 3,
    PrimaryPlayerChanged = 4,
    ZoneChanged = 5,
    PlayerStatsChanged = 6,
    PartyListChanged = 7,
    LogLine = 8,
    ParsedLogLine = 9,
    ProcessChanged = 10,
}

#[derive(Debug)]
pub enum Event<'a> {
    NetworkReceived {
        connection: &'a str,
        timestamp: i64,
        bytes: &'a [u8],
    },
    NetworkSent {
        connection: &'a str,
        timestamp: i64,
        bytes: &'a [u8],
    },
    CombatantAdded(&'a [u8]),
    CombatantRemoved(&'a [u8]),
    PrimaryPlayerChanged,
    ZoneChanged {
        id: u32,
        name: &'a str,
    },
    PlayerStatsChanged(&'a [u8]),
    PartyListChanged {
        ids: Vec<u32>,
        party_size: i32,
    },
    LogLine {
        event_type: u32,
        seconds: u32,
        line: &'a str,
    },
    ParsedLogLine {
        event_type: u32,
        seconds: i32,
        line: &'a str,
    },
    ProcessChanged {
        process_id: u32,
    },
}

impl<'a> Event<'a> {
    pub(crate) fn decode(kind: u32, payload: &'a [u8]) -> Result<Self, DecodeError> {
        let kind = match kind {
            0 => EventKind::NetworkReceived,
            1 => EventKind::NetworkSent,
            2 => EventKind::CombatantAdded,
            3 => EventKind::CombatantRemoved,
            4 => EventKind::PrimaryPlayerChanged,
            5 => EventKind::ZoneChanged,
            6 => EventKind::PlayerStatsChanged,
            7 => EventKind::PartyListChanged,
            8 => EventKind::LogLine,
            9 => EventKind::ParsedLogLine,
            10 => EventKind::ProcessChanged,
            _ => return Err(DecodeError),
        };
        let mut cursor = Cursor(payload);
        Ok(match kind {
            EventKind::NetworkReceived | EventKind::NetworkSent => {
                let timestamp = cursor.i64()?;
                let connection = cursor.str()?;
                let bytes = cursor.bytes()?;
                if kind == EventKind::NetworkReceived {
                    Self::NetworkReceived {
                        connection,
                        timestamp,
                        bytes,
                    }
                } else {
                    Self::NetworkSent {
                        connection,
                        timestamp,
                        bytes,
                    }
                }
            }
            EventKind::CombatantAdded => Self::CombatantAdded(payload),
            EventKind::CombatantRemoved => Self::CombatantRemoved(payload),
            EventKind::PrimaryPlayerChanged => Self::PrimaryPlayerChanged,
            EventKind::ZoneChanged => Self::ZoneChanged {
                id: cursor.u32()?,
                name: cursor.str()?,
            },
            EventKind::PlayerStatsChanged => Self::PlayerStatsChanged(payload),
            EventKind::PartyListChanged => {
                let party_size = cursor.i32()?;
                let count = cursor.u32()? as usize;
                let mut ids = Vec::with_capacity(count);
                for _ in 0..count {
                    ids.push(cursor.u32()?);
                }
                Self::PartyListChanged { ids, party_size }
            }
            EventKind::LogLine => Self::LogLine {
                event_type: cursor.u32()?,
                seconds: cursor.u32()?,
                line: cursor.str()?,
            },
            EventKind::ParsedLogLine => Self::ParsedLogLine {
                event_type: cursor.u32()?,
                seconds: cursor.i32()?,
                line: cursor.str()?,
            },
            EventKind::ProcessChanged => Self::ProcessChanged {
                process_id: cursor.u32()?,
            },
        })
    }
}

#[derive(Debug)]
pub struct DecodeError;

struct Cursor<'a>(&'a [u8]);
impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        if self.0.len() < len {
            return Err(DecodeError);
        }
        let (head, tail) = self.0.split_at(len);
        self.0 = tail;
        Ok(head)
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.u32()? as usize;
        self.take(len)
    }
    fn str(&mut self) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| DecodeError)
    }
}

pub struct CallContext<'a> {
    repository: Repository<'a>,
}

impl<'a> CallContext<'a> {
    pub(crate) fn new(repository: Repository<'a>) -> Self {
        Self { repository }
    }
    pub fn repository(&self) -> &Repository<'a> {
        &self.repository
    }
}

pub struct Repository<'a> {
    host: &'a RawHostApiV1,
    _not_send: PhantomData<Rc<()>>,
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("managed repository query failed with status {0:?}")]
    Status(Status),
    #[error("managed repository returned malformed data")]
    Malformed,
}

impl<'a> Repository<'a> {
    pub(crate) fn new(host: &'a RawHostApiV1) -> Self {
        Self {
            host,
            _not_send: PhantomData,
        }
    }

    pub fn query(&self, query: u32, request: &[u8]) -> Result<Bytes, RepositoryError> {
        let Some(query_fn) = self.host.query else {
            return Err(RepositoryError::Status(Status::NotInitialized));
        };
        let mut required = 0usize;
        let status = unsafe {
            query_fn(
                self.host.context,
                query,
                request.as_ptr(),
                request.len(),
                std::ptr::null_mut(),
                0,
                &mut required,
            )
        };
        if status != Status::Ok && status != Status::BufferTooSmall {
            return Err(RepositoryError::Status(status));
        }
        for _ in 0..3 {
            let mut output = BytesMut::zeroed(required);
            let mut next = required;
            let status = unsafe {
                query_fn(
                    self.host.context,
                    query,
                    request.as_ptr(),
                    request.len(),
                    output.as_mut_ptr(),
                    output.len(),
                    &mut next,
                )
            };
            if status == Status::Ok {
                if next > output.len() {
                    return Err(RepositoryError::Malformed);
                }
                output.truncate(next);
                return Ok(output.freeze());
            }
            if status != Status::BufferTooSmall {
                return Err(RepositoryError::Status(status));
            }
            required = next;
        }
        Err(RepositoryError::Status(Status::BufferTooSmall))
    }

    pub fn current_player_id(&self) -> Result<u32, RepositoryError> {
        self.u32_query(1)
    }
    pub fn current_territory_id(&self) -> Result<u32, RepositoryError> {
        self.u32_query(2)
    }
    pub fn selected_language(&self) -> Result<i32, RepositoryError> {
        let bytes = self.query(3, &[])?;
        bytes
            .get(..4)
            .and_then(|v| v.try_into().ok())
            .map(i32::from_le_bytes)
            .ok_or(RepositoryError::Malformed)
    }
    pub fn game_version(&self) -> Result<String, RepositoryError> {
        String::from_utf8(self.query(4, &[])?.to_vec()).map_err(|_| RepositoryError::Malformed)
    }
    pub fn is_chat_log_available(&self) -> Result<bool, RepositoryError> {
        self.query(5, &[])?
            .first()
            .map(|v| *v != 0)
            .ok_or(RepositoryError::Malformed)
    }
    pub fn raw_combatants(&self) -> Result<Bytes, RepositoryError> {
        self.query(6, &[])
    }
    pub fn raw_player(&self) -> Result<Bytes, RepositoryError> {
        self.query(7, &[])
    }
    pub fn raw_resource_dictionary(&self, resource_type: i32) -> Result<Bytes, RepositoryError> {
        self.query(8, &resource_type.to_le_bytes())
    }
    pub fn current_process_id(&self) -> Result<u32, RepositoryError> {
        self.u32_query(9)
    }
    pub fn server_timestamp_ticks(&self) -> Result<i64, RepositoryError> {
        let bytes = self.query(10, &[])?;
        bytes
            .get(..8)
            .and_then(|v| v.try_into().ok())
            .map(i64::from_le_bytes)
            .ok_or(RepositoryError::Malformed)
    }
    pub fn antivirus_names(&self) -> Result<Vec<String>, RepositoryError> {
        let bytes = self.query(11, &[])?;
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| RepositoryError::Malformed)?;
        Ok(if text.is_empty() {
            vec![]
        } else {
            text.split('\0').map(str::to_owned).collect()
        })
    }
    pub fn game_region(&self) -> Result<u8, RepositoryError> {
        self.query(12, &[])?
            .first()
            .copied()
            .ok_or(RepositoryError::Malformed)
    }

    fn u32_query(&self, query: u32) -> Result<u32, RepositoryError> {
        let bytes = self.query(query, &[])?;
        bytes
            .get(..4)
            .and_then(|v| v.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or(RepositoryError::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;

    #[test]
    fn decodes_zone_event_and_subscription_bits() {
        let mut payload = BytesMut::new();
        payload.put_u32_le(42);
        payload.put_u32_le(4);
        payload.extend_from_slice(b"Test");
        assert!(matches!(
            Event::decode(5, &payload),
            Ok(Event::ZoneChanged {
                id: 42,
                name: "Test"
            })
        ));
        assert!(
            (SubscriptionSet::ZONE_CHANGED | SubscriptionSet::LOG_LINE)
                .contains(SubscriptionSet::LOG_LINE)
        );
    }
}
