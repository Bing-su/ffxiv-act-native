use std::{
    collections::BTreeMap, error::Error, marker::PhantomData, ptr::null_mut, rc::Rc, str::from_utf8,
};

use bytes::{Bytes, BytesMut};

use crate::{RawHostApiV1, Status};

pub type PluginError = Box<dyn Error + Send + Sync + 'static>;
pub type PluginResult<T> = Result<T, PluginError>;

pub trait Plugin: Send + 'static {
    fn init(repository: Repository<'_>) -> PluginResult<(Self, SubscriptionSet)>
    where
        Self: Sized;

    fn on_event(&mut self, repository: Repository<'_>, event: Event<'_>) -> PluginResult<()>;

    fn shutdown(&mut self, repository: Repository<'_>) -> PluginResult<()>;
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct SubscriptionSet: u32 {
        const NONE = 0;
        const NETWORK_RECEIVED = 1 << 0;
        const NETWORK_SENT = 1 << 1;
        const COMBATANT_ADDED = 1 << 2;
        const COMBATANT_REMOVED = 1 << 3;
        const PRIMARY_PLAYER_CHANGED = 1 << 4;
        const ZONE_CHANGED = 1 << 5;
        const PLAYER_STATS_CHANGED = 1 << 6;
        const PARTY_LIST_CHANGED = 1 << 7;
        const LOG_LINE = 1 << 8;
        const PARSED_LOG_LINE = 1 << 9;
        const PROCESS_CHANGED = 1 << 10;
        const ALL = (1 << 11) - 1;
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
    CombatantAdded(Combatant),
    CombatantRemoved(Combatant),
    PrimaryPlayerChanged,
    ZoneChanged {
        id: u32,
        name: &'a str,
    },
    PlayerStatsChanged(Player),
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
            EventKind::CombatantAdded => {
                Self::CombatantAdded(cursor.combatant()?.ok_or(DecodeError)?)
            }
            EventKind::CombatantRemoved => {
                Self::CombatantRemoved(cursor.combatant()?.ok_or(DecodeError)?)
            }
            EventKind::PrimaryPlayerChanged => Self::PrimaryPlayerChanged,
            EventKind::ZoneChanged => Self::ZoneChanged {
                id: cursor.u32()?,
                name: cursor.str()?,
            },
            EventKind::PlayerStatsChanged => {
                Self::PlayerStatsChanged(cursor.player()?.ok_or(DecodeError)?)
            }
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

#[derive(Clone, Debug, PartialEq)]
pub struct NetworkBuff {
    pub buff_id: u16,
    pub buff_extra: u16,
    pub timestamp_ticks: i64,
    pub duration: f32,
    pub actor_id: u32,
    pub actor_name: String,
    pub target_id: u32,
    pub target_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Combatant {
    pub id: u32,
    pub owner_id: u32,
    pub kind: u8,
    pub job: i32,
    pub level: i32,
    pub name: String,
    pub current_hp: u32,
    pub max_hp: u32,
    pub current_mp: u32,
    pub max_mp: u32,
    pub current_cp: u32,
    pub max_cp: u32,
    pub current_gp: u32,
    pub max_gp: u32,
    pub is_casting: bool,
    pub cast_buff_id: u32,
    pub cast_target_id: u32,
    pub cast_duration_current: f32,
    pub cast_duration_max: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub heading: f32,
    pub current_world_id: u32,
    pub world_id: u32,
    pub world_name: String,
    pub bnpc_name_id: u32,
    pub bnpc_id: u32,
    pub target_id: u32,
    pub effective_distance: u8,
    pub party_type: i32,
    pub address: i64,
    pub order: i32,
    pub network_buffs: Vec<Option<NetworkBuff>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub job_id: u32,
    pub strength: u32,
    pub dexterity: u32,
    pub vitality: u32,
    pub intelligence: u32,
    pub mind: u32,
    pub piety: u32,
    pub attack: u32,
    pub direct_hit: u32,
    pub critical_hit: u32,
    pub attack_magic_potency: u32,
    pub heal_magic_potency: u32,
    pub determination: u32,
    pub skill_speed: u32,
    pub spell_speed: u32,
    pub tenacity: u32,
    pub local_content_id: u64,
}

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
    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut value = [0; N];
        value.copy_from_slice(self.take(N)?);
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.u32()? as usize;
        self.take(len)
    }
    fn str(&mut self) -> Result<&'a str, DecodeError> {
        from_utf8(self.bytes()?).map_err(|_| DecodeError)
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        Ok(self.str()?.to_owned())
    }
    fn player(&mut self) -> Result<Option<Player>, DecodeError> {
        if self.u8()? == 0 {
            return Ok(None);
        }
        Ok(Some(Player {
            job_id: self.u32()?,
            strength: self.u32()?,
            dexterity: self.u32()?,
            vitality: self.u32()?,
            intelligence: self.u32()?,
            mind: self.u32()?,
            piety: self.u32()?,
            attack: self.u32()?,
            direct_hit: self.u32()?,
            critical_hit: self.u32()?,
            attack_magic_potency: self.u32()?,
            heal_magic_potency: self.u32()?,
            determination: self.u32()?,
            skill_speed: self.u32()?,
            spell_speed: self.u32()?,
            tenacity: self.u32()?,
            local_content_id: self.u64()?,
        }))
    }
    fn combatant(&mut self) -> Result<Option<Combatant>, DecodeError> {
        if self.u8()? == 0 {
            return Ok(None);
        }
        let mut value = Combatant {
            id: self.u32()?,
            owner_id: self.u32()?,
            kind: self.u8()?,
            job: self.i32()?,
            level: self.i32()?,
            name: self.string()?,
            current_hp: self.u32()?,
            max_hp: self.u32()?,
            current_mp: self.u32()?,
            max_mp: self.u32()?,
            current_cp: self.u32()?,
            max_cp: self.u32()?,
            current_gp: self.u32()?,
            max_gp: self.u32()?,
            is_casting: self.u8()? != 0,
            cast_buff_id: self.u32()?,
            cast_target_id: self.u32()?,
            cast_duration_current: self.f32()?,
            cast_duration_max: self.f32()?,
            pos_x: self.f32()?,
            pos_y: self.f32()?,
            pos_z: self.f32()?,
            heading: self.f32()?,
            current_world_id: self.u32()?,
            world_id: self.u32()?,
            world_name: self.string()?,
            bnpc_name_id: self.u32()?,
            bnpc_id: self.u32()?,
            target_id: self.u32()?,
            effective_distance: self.u8()?,
            party_type: self.i32()?,
            address: self.i64()?,
            order: self.i32()?,
            network_buffs: Vec::new(),
        };
        let count = self.u32()? as usize;
        value.network_buffs.reserve(count);
        for _ in 0..count {
            if self.u8()? == 0 {
                value.network_buffs.push(None);
                continue;
            }
            value.network_buffs.push(Some(NetworkBuff {
                buff_id: self.u16()?,
                buff_extra: self.u16()?,
                timestamp_ticks: self.i64()?,
                duration: self.f32()?,
                actor_id: self.u32()?,
                actor_name: self.string()?,
                target_id: self.u32()?,
                target_name: self.string()?,
            }));
        }
        Ok(Some(value))
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
                null_mut(),
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
        let bytes = self.query(4, &[])?;
        Cursor(&bytes)
            .string()
            .map_err(|_| RepositoryError::Malformed)
    }
    pub fn is_chat_log_available(&self) -> Result<bool, RepositoryError> {
        self.query(5, &[])?
            .first()
            .map(|v| *v != 0)
            .ok_or(RepositoryError::Malformed)
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
        let mut cursor = Cursor(&bytes);
        let count = cursor.u32().map_err(|_| RepositoryError::Malformed)? as usize;
        (0..count)
            .map(|_| cursor.string().map_err(|_| RepositoryError::Malformed))
            .collect()
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

    pub fn combatants(&self) -> Result<Vec<Combatant>, RepositoryError> {
        let bytes = self.query(6, &[])?;
        let mut cursor = Cursor(&bytes);
        let count = cursor.u32().map_err(|_| RepositoryError::Malformed)? as usize;
        (0..count)
            .map(|_| {
                cursor
                    .combatant()
                    .map_err(|_| RepositoryError::Malformed)?
                    .ok_or(RepositoryError::Malformed)
            })
            .collect()
    }

    pub fn player(&self) -> Result<Option<Player>, RepositoryError> {
        let bytes = self.query(7, &[])?;
        Cursor(&bytes)
            .player()
            .map_err(|_| RepositoryError::Malformed)
    }

    pub fn resource_dictionary(
        &self,
        resource_type: i32,
    ) -> Result<BTreeMap<u32, String>, RepositoryError> {
        let bytes = self.query(8, &resource_type.to_le_bytes())?;
        let mut cursor = Cursor(&bytes);
        let count = cursor.u32().map_err(|_| RepositoryError::Malformed)? as usize;
        (0..count)
            .map(|_| {
                Ok((
                    cursor.u32().map_err(|_| RepositoryError::Malformed)?,
                    cursor.string().map_err(|_| RepositoryError::Malformed)?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use std::{ffi::c_void, ptr::copy_nonoverlapping};

    use crate::ABI_VERSION;

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

    #[test]
    fn decodes_owned_player_model() {
        let mut payload = BytesMut::new();
        payload.put_u8(1);
        for value in 1..=16 {
            payload.put_u32_le(value);
        }
        payload.put_u64_le(99);
        let Event::PlayerStatsChanged(player) = Event::decode(6, &payload).unwrap() else {
            panic!("wrong event");
        };
        assert_eq!(player.job_id, 1);
        assert_eq!(player.tenacity, 16);
        assert_eq!(player.local_content_id, 99);
    }

    unsafe extern "system" fn query_with_retry(
        _: *mut c_void,
        _: u32,
        _: *const u8,
        _: usize,
        output: *mut u8,
        output_len: usize,
        required: *mut usize,
    ) -> Status {
        let value = b"bytes";
        unsafe { *required = value.len() };
        if output.is_null() || output_len < value.len() {
            return Status::BufferTooSmall;
        }
        unsafe { copy_nonoverlapping(value.as_ptr(), output, value.len()) };
        Status::Ok
    }

    #[test]
    fn repository_retries_owned_bytes_buffer() {
        let host = RawHostApiV1 {
            abi_version: ABI_VERSION,
            context: null_mut(),
            query: Some(query_with_retry),
            set_status: None,
        };
        assert_eq!(Repository::new(&host).query(0, &[]).unwrap(), &b"bytes"[..]);
    }
}
