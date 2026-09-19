use std::{
    collections::BTreeMap, error::Error, marker::PhantomData, ptr::null_mut, rc::Rc, str::from_utf8,
};

use bytes::{BufMut, Bytes, BytesMut};

use crate::{DecodeError, RawHostApiV1, Status};

const UI_QUERY: u32 = 0x100;
const MAX_UI_PAYLOAD: usize = 1024 * 1024;

/// Thread-safe error returned by plugin callbacks.
///
/// A boxed trait object lets plugin crates use their existing error types
/// without coupling the ABI to a particular error library.
pub type PluginError = Box<dyn Error + Send + Sync + 'static>;
/// Result type expected from plugin lifecycle callbacks.
pub type PluginResult<T> = Result<T, PluginError>;

bitflags::bitflags! {
    /// Selects which ACT events cross the managed/native boundary.
    ///
    /// Subscribe only to events the plugin handles to avoid unnecessary FFI
    /// calls. Flags can be combined with the bitwise OR operator.
    ///
    /// # Example
    ///
    /// ```
    /// use ffxiv_act_native::SubscriptionSet;
    ///
    /// let subscriptions = SubscriptionSet::ZONE_CHANGED
    ///     | SubscriptionSet::PARTY_LIST_CHANGED;
    /// ```
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct SubscriptionSet: u32 {
        /// Disables all managed event subscriptions.
        const NONE = 0;
        /// Receives packets read from the game connection.
        const NETWORK_RECEIVED = 1 << 0;
        /// Receives packets written to the game connection.
        const NETWORK_SENT = 1 << 1;
        /// Receives combatants as they enter the repository snapshot.
        const COMBATANT_ADDED = 1 << 2;
        /// Receives combatants as they leave the repository snapshot.
        const COMBATANT_REMOVED = 1 << 3;
        /// Receives notification when the local player changes.
        const PRIMARY_PLAYER_CHANGED = 1 << 4;
        /// Receives territory changes.
        const ZONE_CHANGED = 1 << 5;
        /// Receives updated local-player attributes.
        const PLAYER_STATS_CHANGED = 1 << 6;
        /// Receives party membership changes.
        const PARTY_LIST_CHANGED = 1 << 7;
        /// Receives raw ACT log lines.
        const LOG_LINE = 1 << 8;
        /// Receives parsed ACT log lines.
        const PARSED_LOG_LINE = 1 << 9;
        /// Receives game process attachment changes.
        const PROCESS_CHANGED = 1 << 10;
        /// Enables every managed event subscription.
        const ALL = (1 << 11) - 1;
    }
}

#[derive(Debug)]
/// Value returned by [`Plugin::init`] to install the plugin and its subscriptions.
pub struct PluginInit<P> {
    /// Initialized plugin state retained until shutdown.
    pub plugin: P,
    /// Events the managed shim should forward to the plugin.
    pub subscriptions: SubscriptionSet,
}

impl<P> PluginInit<P> {
    /// Couples initialized state with its event subscriptions.
    pub fn new(plugin: P, subscriptions: SubscriptionSet) -> Self {
        Self {
            plugin,
            subscriptions,
        }
    }
}

/// Lifecycle implemented by a native ACT plugin.
///
/// Callbacks are serialized, so implementations may mutate their state without
/// adding an internal lock. Returning an error stops later event delivery and
/// reports the message to ACT, preventing a failing plugin from repeatedly
/// crossing the FFI boundary.
pub trait Plugin: Send + 'static {
    /// Creates the plugin and selects the events it wants to receive.
    fn init(repository: Repository<'_>) -> PluginResult<PluginInit<Self>>
    where
        Self: Sized;

    /// Handles one subscribed ACT event.
    ///
    /// The event and repository borrow callback-owned data and must not be
    /// retained after this method returns.
    fn on_event(&mut self, repository: Repository<'_>, event: Event<'_>) -> PluginResult<()>;

    /// Releases resources before the managed shim unloads the plugin.
    ///
    /// Explicit shutdown exists so native resources can be released before ACT
    /// unloads the managed bridge.
    fn shutdown(&mut self, repository: Repository<'_>) -> PluginResult<()>;
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Numeric tags used to identify event payloads in ABI v1.
///
/// The explicit representation keeps Rust and the managed shim in agreement
/// without relying on compiler-specific enum layout.
pub enum EventKind {
    /// Incoming game network data.
    NetworkReceived = 0,
    /// Outgoing game network data.
    NetworkSent = 1,
    /// A combatant entered the current snapshot.
    CombatantAdded = 2,
    /// A combatant left the current snapshot.
    CombatantRemoved = 3,
    /// The local player identity changed.
    PrimaryPlayerChanged = 4,
    /// The current territory changed.
    ZoneChanged = 5,
    /// The local player's attributes changed.
    PlayerStatsChanged = 6,
    /// Party membership changed.
    PartyListChanged = 7,
    /// An unparsed ACT log line arrived.
    LogLine = 8,
    /// A parsed ACT log line arrived.
    ParsedLogLine = 9,
    /// ACT attached to or detached from a game process.
    ProcessChanged = 10,
    /// A control on the generated plugin tab produced an event.
    Ui = 11,
}

impl TryFrom<u32> for EventKind {
    type Error = DecodeError;

    /// Converts the event tag used by ABI v1 into its typed representation.
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NetworkReceived),
            1 => Ok(Self::NetworkSent),
            2 => Ok(Self::CombatantAdded),
            3 => Ok(Self::CombatantRemoved),
            4 => Ok(Self::PrimaryPlayerChanged),
            5 => Ok(Self::ZoneChanged),
            6 => Ok(Self::PlayerStatsChanged),
            7 => Ok(Self::PartyListChanged),
            8 => Ok(Self::LogLine),
            9 => Ok(Self::ParsedLogLine),
            10 => Ok(Self::ProcessChanged),
            11 => Ok(Self::Ui),
            _ => Err(DecodeError),
        }
    }
}

#[derive(Debug)]
/// Typed event delivered to [`Plugin::on_event`].
///
/// Borrowed strings and byte slices point into the callback payload, avoiding
/// allocations on high-frequency network and log events.
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
    Ui(UiEvent<'a>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// User interaction or validation failure from the generated ACT tab.
///
/// Control IDs are assigned by the plugin so one event can be matched directly
/// to the [`UiControl`] that produced it.
pub enum UiEvent<'a> {
    Clicked { id: u32 },
    CheckedChanged { id: u32, checked: bool },
    TextChanged { id: u32, text: &'a str },
    SelectionChanged { id: u32, selected: Option<u32> },
    NumberChanged { id: u32, value: i64 },
    Error { id: Option<u32>, message: &'a str },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Declarative control that can be added to the generated ACT tab.
///
/// The intentionally small control set keeps UI behavior in the managed shim
/// while plugin state remains in Rust.
pub enum UiControl {
    Row {
        id: u32,
    },
    Column {
        id: u32,
    },
    Label {
        id: u32,
        text: String,
    },
    Button {
        id: u32,
        text: String,
    },
    CheckBox {
        id: u32,
        text: String,
        checked: bool,
    },
    TextBox {
        id: u32,
        text: String,
    },
    ComboBox {
        id: u32,
        items: Vec<String>,
        selected: Option<u32>,
    },
    Number {
        id: u32,
        min: i64,
        max: i64,
        step: i64,
        value: i64,
    },
    LogList {
        id: u32,
        max_rows: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Incremental update sent to the generated ACT tab.
///
/// Commands avoid exposing managed UI objects across FFI. They are queued with
/// [`Repository::ui`].
///
/// # Example
///
/// ```no_run
/// # use ffxiv_act_native::{Repository, UiCommand, UiControl};
/// # fn add_button(repository: Repository<'_>) -> Result<(), Box<dyn std::error::Error>> {
/// repository.ui(UiCommand::Add {
///     parent: None,
///     control: UiControl::Button { id: 1, text: "Refresh".into() },
/// })?;
/// # Ok(())
/// # }
/// ```
pub enum UiCommand {
    Add {
        parent: Option<u32>,
        control: UiControl,
    },
    Remove {
        id: u32,
    },
    Clear,
    SetText {
        id: u32,
        text: String,
    },
    SetEnabled {
        id: u32,
        enabled: bool,
    },
    SetChecked {
        id: u32,
        checked: bool,
    },
    SetItems {
        id: u32,
        items: Vec<String>,
        selected: Option<u32>,
    },
    SetSelected {
        id: u32,
        selected: Option<u32>,
    },
    SetNumber {
        id: u32,
        value: i64,
    },
    AppendLog {
        id: u32,
        line: String,
    },
    ClearLog {
        id: u32,
    },
}

impl<'a> Event<'a> {
    /// Decodes an ABI v1 event payload.
    pub(crate) fn decode(kind: u32, payload: &'a [u8]) -> Result<Self, DecodeError> {
        let kind = EventKind::try_from(kind)?;
        let mut cursor = Cursor(payload);
        Ok(match kind {
            EventKind::NetworkReceived | EventKind::NetworkSent => cursor.network_event(kind)?,
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
            EventKind::Ui => Self::Ui(cursor.ui_event()?),
        })
    }
}

impl UiCommand {
    fn encode(&self) -> Result<Bytes, RepositoryError> {
        let mut bytes = BytesMut::new();
        match self {
            Self::Add { parent, control } => {
                bytes.put_u8(0);
                put_option_u32(&mut bytes, *parent);
                control.encode(&mut bytes)?;
            }
            Self::Remove { id } => {
                bytes.put_u8(1);
                bytes.put_u32_le(*id);
            }
            Self::Clear => bytes.put_u8(2),
            Self::SetText { id, text } => {
                bytes.put_u8(3);
                bytes.put_u32_le(*id);
                put_string(&mut bytes, text)?;
            }
            Self::SetEnabled { id, enabled } => {
                bytes.put_u8(4);
                bytes.put_u32_le(*id);
                bytes.put_u8(*enabled as u8);
            }
            Self::SetChecked { id, checked } => {
                bytes.put_u8(5);
                bytes.put_u32_le(*id);
                bytes.put_u8(*checked as u8);
            }
            Self::SetItems {
                id,
                items,
                selected,
            } => {
                bytes.put_u8(6);
                bytes.put_u32_le(*id);
                put_strings(&mut bytes, items)?;
                put_option_u32(&mut bytes, *selected);
            }
            Self::SetSelected { id, selected } => {
                bytes.put_u8(7);
                bytes.put_u32_le(*id);
                put_option_u32(&mut bytes, *selected);
            }
            Self::SetNumber { id, value } => {
                bytes.put_u8(8);
                bytes.put_u32_le(*id);
                bytes.put_i64_le(*value);
            }
            Self::AppendLog { id, line } => {
                bytes.put_u8(9);
                bytes.put_u32_le(*id);
                put_string(&mut bytes, line)?;
            }
            Self::ClearLog { id } => {
                bytes.put_u8(10);
                bytes.put_u32_le(*id);
            }
        }
        if bytes.len() > MAX_UI_PAYLOAD {
            return Err(RepositoryError::InvalidUi("UI command exceeds 1 MiB"));
        }
        Ok(bytes.freeze())
    }
}

impl UiControl {
    fn encode(&self, bytes: &mut BytesMut) -> Result<(), RepositoryError> {
        let (kind, id) = match self {
            Self::Row { id } => (0, *id),
            Self::Column { id } => (1, *id),
            Self::Label { id, .. } => (2, *id),
            Self::Button { id, .. } => (3, *id),
            Self::CheckBox { id, .. } => (4, *id),
            Self::TextBox { id, .. } => (5, *id),
            Self::ComboBox { id, .. } => (6, *id),
            Self::Number { id, .. } => (7, *id),
            Self::LogList { id, .. } => (8, *id),
        };
        bytes.put_u8(kind);
        bytes.put_u32_le(id);
        match self {
            Self::Row { .. } | Self::Column { .. } => {}
            Self::Label { text, .. } | Self::Button { text, .. } | Self::TextBox { text, .. } => {
                put_string(bytes, text)?;
            }
            Self::CheckBox { text, checked, .. } => {
                put_string(bytes, text)?;
                bytes.put_u8(*checked as u8);
            }
            Self::ComboBox {
                items, selected, ..
            } => {
                put_strings(bytes, items)?;
                put_option_u32(bytes, *selected);
            }
            Self::Number {
                min,
                max,
                step,
                value,
                ..
            } => {
                bytes.put_i64_le(*min);
                bytes.put_i64_le(*max);
                bytes.put_i64_le(*step);
                bytes.put_i64_le(*value);
            }
            Self::LogList { max_rows, .. } => bytes.put_u32_le(*max_rows),
        }
        Ok(())
    }
}

fn put_string(bytes: &mut BytesMut, value: &str) -> Result<(), RepositoryError> {
    let len =
        u32::try_from(value.len()).map_err(|_| RepositoryError::InvalidUi("string is too long"))?;
    bytes.put_u32_le(len);
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_strings(bytes: &mut BytesMut, values: &[String]) -> Result<(), RepositoryError> {
    let len =
        u32::try_from(values.len()).map_err(|_| RepositoryError::InvalidUi("too many items"))?;
    bytes.put_u32_le(len);
    for value in values {
        put_string(bytes, value)?;
    }
    Ok(())
}

fn put_option_u32(bytes: &mut BytesMut, value: Option<u32>) {
    bytes.put_u8(value.is_some() as u8);
    if let Some(value) = value {
        bytes.put_u32_le(value);
    }
}

#[derive(Clone, Debug, PartialEq)]
/// Network status effect attached to a [`Combatant`] snapshot.
///
/// Values mirror the public SDK model so plugins can inspect status ownership
/// and duration without depending on managed types.
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
/// Owned snapshot of one combatant from the ACT data repository.
///
/// The model is owned because repository query buffers are temporary; callers
/// may safely retain a snapshot after the callback returns.
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
/// Owned snapshot of the local player's combat attributes.
///
/// The fields mirror the public SDK `Player` model to preserve their original
/// meaning and numeric representation across the ABI.
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
    /// Removes and returns the next `len` bytes.
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
    fn option_u32(&mut self) -> Result<Option<u32>, DecodeError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u32()?)),
            _ => Err(DecodeError),
        }
    }
    fn ui_event(&mut self) -> Result<UiEvent<'a>, DecodeError> {
        let event = match self.u8()? {
            0 => UiEvent::Clicked { id: self.u32()? },
            1 => UiEvent::CheckedChanged {
                id: self.u32()?,
                checked: match self.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(DecodeError),
                },
            },
            2 => UiEvent::TextChanged {
                id: self.u32()?,
                text: self.str()?,
            },
            3 => UiEvent::SelectionChanged {
                id: self.u32()?,
                selected: self.option_u32()?,
            },
            4 => UiEvent::NumberChanged {
                id: self.u32()?,
                value: self.i64()?,
            },
            5 => UiEvent::Error {
                id: self.option_u32()?,
                message: self.str()?,
            },
            _ => return Err(DecodeError),
        };
        if !self.0.is_empty() {
            return Err(DecodeError);
        }
        Ok(event)
    }
    /// Decodes the shared wire shape of sent and received network events.
    fn network_event(&mut self, kind: EventKind) -> Result<Event<'a>, DecodeError> {
        let timestamp = self.i64()?;
        let connection = self.str()?;
        let bytes = self.bytes()?;
        Ok(match kind {
            EventKind::NetworkReceived => Event::NetworkReceived {
                connection,
                timestamp,
                bytes,
            },
            EventKind::NetworkSent => Event::NetworkSent {
                connection,
                timestamp,
                bytes,
            },
            _ => unreachable!("only network event kinds call this decoder"),
        })
    }
    /// Decodes an optional player record.
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
    /// Decodes an optional combatant and its network buffs.
    fn combatant(&mut self) -> Result<Option<Combatant>, DecodeError> {
        if self.u8()? == 0 {
            return Ok(None);
        }
        Ok(Some(Combatant {
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
            network_buffs: self.network_buffs()?,
        }))
    }

    /// Decodes the length-prefixed network buff list.
    fn network_buffs(&mut self) -> Result<Vec<Option<NetworkBuff>>, DecodeError> {
        let count = self.u32()? as usize;
        (0..count)
            .map(|_| {
                if self.u8()? == 0 {
                    return Ok(None);
                }
                Ok(Some(NetworkBuff {
                    buff_id: self.u16()?,
                    buff_extra: self.u16()?,
                    timestamp_ticks: self.i64()?,
                    duration: self.f32()?,
                    actor_id: self.u32()?,
                    actor_name: self.string()?,
                    target_id: self.u32()?,
                    target_name: self.string()?,
                }))
            })
            .collect()
    }
}

/// Borrowed access to managed ACT repository services during a callback.
///
/// The handle is deliberately neither `Send` nor `Sync`: managed callbacks may
/// arrive on different CLR threads, but repository use stays on the current
/// serialized callback.
///
/// # Example
///
/// ```no_run
/// # use ffxiv_act_native::{Repository, RepositoryError};
/// # fn inspect(repository: Repository<'_>) -> Result<(), RepositoryError> {
/// let territory = repository.current_territory_id()?;
/// let combatants = repository.combatants()?;
/// println!("{territory}: {} combatants", combatants.len());
/// # Ok(())
/// # }
/// ```
pub struct Repository<'a> {
    host: &'a RawHostApiV1,
    _not_send: PhantomData<Rc<()>>,
}

#[derive(Debug, thiserror::Error)]
/// Failure returned while calling a managed repository service.
pub enum RepositoryError {
    /// The managed host rejected or could not complete the query.
    #[error("managed query failed with status {0:?}")]
    Status(Status),
    /// The host returned bytes that do not match the expected wire format.
    #[error("managed repository returned malformed data")]
    Malformed,
    /// A UI command cannot be represented safely by the managed bridge.
    #[error("invalid UI command: {0}")]
    InvalidUi(&'static str),
}

impl<'a> Repository<'a> {
    /// Wraps the host API for the duration of one plugin callback.
    pub(crate) fn new(host: &'a RawHostApiV1) -> Self {
        Self {
            host,
            _not_send: PhantomData,
        }
    }

    /// Executes a raw managed repository query and returns its payload.
    ///
    /// This escape hatch supports ABI queries not yet covered by typed helpers;
    /// prefer those helpers when available so payload decoding remains central.
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
        if status == Status::Ok && required == 0 {
            return Ok(Bytes::new());
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

    /// Queues one update for the ACT plugin tab.
    ///
    /// UI mutation is routed through the host so Rust never owns or calls a
    /// managed control directly.
    pub fn ui(&self, command: UiCommand) -> Result<(), RepositoryError> {
        self.query(UI_QUERY, &command.encode()?).map(|_| ())
    }

    /// Returns the current player actor ID.
    pub fn current_player_id(&self) -> Result<u32, RepositoryError> {
        self.u32_query(1)
    }
    /// Returns the current territory ID.
    pub fn current_territory_id(&self) -> Result<u32, RepositoryError> {
        self.u32_query(2)
    }
    /// Returns the selected game language ID used by SDK resource lookups.
    pub fn selected_language(&self) -> Result<i32, RepositoryError> {
        let bytes = self.query(3, &[])?;
        bytes
            .get(..4)
            .and_then(|v| v.try_into().ok())
            .map(i32::from_le_bytes)
            .ok_or(RepositoryError::Malformed)
    }
    /// Returns the detected game version.
    pub fn game_version(&self) -> Result<String, RepositoryError> {
        let bytes = self.query(4, &[])?;
        Cursor(&bytes)
            .string()
            .map_err(|_| RepositoryError::Malformed)
    }
    /// Reports whether ACT can currently read the chat log.
    pub fn is_chat_log_available(&self) -> Result<bool, RepositoryError> {
        self.query(5, &[])?
            .first()
            .map(|v| *v != 0)
            .ok_or(RepositoryError::Malformed)
    }
    /// Returns the attached game process ID.
    pub fn current_process_id(&self) -> Result<u32, RepositoryError> {
        self.u32_query(9)
    }
    /// Returns the server timestamp in .NET ticks for comparison with SDK times.
    pub fn server_timestamp_ticks(&self) -> Result<i64, RepositoryError> {
        let bytes = self.query(10, &[])?;
        bytes
            .get(..8)
            .and_then(|v| v.try_into().ok())
            .map(i64::from_le_bytes)
            .ok_or(RepositoryError::Malformed)
    }
    /// Returns antivirus product names reported by the repository.
    pub fn antivirus_names(&self) -> Result<Vec<String>, RepositoryError> {
        let bytes = self.query(11, &[])?;
        let mut cursor = Cursor(&bytes);
        let count = cursor.u32().map_err(|_| RepositoryError::Malformed)? as usize;
        (0..count)
            .map(|_| cursor.string().map_err(|_| RepositoryError::Malformed))
            .collect()
    }
    /// Returns the configured game region ID.
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

    /// Returns the current combatant snapshot.
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

    /// Returns the current player stats, if a player is available.
    pub fn player(&self) -> Result<Option<Player>, RepositoryError> {
        let bytes = self.query(7, &[])?;
        Cursor(&bytes)
            .player()
            .map_err(|_| RepositoryError::Malformed)
    }

    /// Returns the requested localized resource dictionary.
    ///
    /// `resource_type` uses the numeric value of the public SDK `ResourceType`
    /// enum so the native ABI does not need to duplicate a versioned enum.
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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
        assert!(matches!(EventKind::try_from(5), Ok(EventKind::ZoneChanged)));
        assert!(EventKind::try_from(u32::MAX).is_err());
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

    #[test]
    fn encodes_ui_commands_and_decodes_ui_events() {
        let command = UiCommand::Add {
            parent: Some(7),
            control: UiControl::CheckBox {
                id: 8,
                text: "Enabled".into(),
                checked: true,
            },
        };
        assert_eq!(
            command.encode().unwrap(),
            &[
                0, 1, 7, 0, 0, 0, 4, 8, 0, 0, 0, 7, 0, 0, 0, b'E', b'n', b'a', b'b', b'l', b'e',
                b'd', 1,
            ][..]
        );

        let mut payload = BytesMut::new();
        payload.put_u8(2);
        payload.put_u32_le(8);
        payload.put_u32_le(2);
        payload.extend_from_slice(b"ok");
        assert!(matches!(
            Event::decode(11, &payload),
            Ok(Event::Ui(UiEvent::TextChanged { id: 8, text: "ok" }))
        ));
        payload.put_u8(0);
        assert!(Event::decode(11, &payload).is_err());
        assert!(Event::decode(11, &[99]).is_err());
        assert!(Event::decode(11, &[2, 1, 0]).is_err());
        assert!(Event::decode(11, &[1, 1, 0, 0, 0, 2]).is_err());
    }

    #[test]
    fn encodes_every_ui_command_and_control() {
        let controls = [
            UiControl::Row { id: 1 },
            UiControl::Column { id: 9 },
            UiControl::Label {
                id: 2,
                text: "label".into(),
            },
            UiControl::Button {
                id: 3,
                text: "button".into(),
            },
            UiControl::CheckBox {
                id: 4,
                text: "check".into(),
                checked: true,
            },
            UiControl::TextBox {
                id: 5,
                text: "text".into(),
            },
            UiControl::ComboBox {
                id: 6,
                items: vec!["item".into()],
                selected: Some(0),
            },
            UiControl::Number {
                id: 7,
                min: -1,
                max: 10,
                step: 1,
                value: 2,
            },
            UiControl::LogList {
                id: 8,
                max_rows: 1000,
            },
        ];
        for (kind, control) in controls.into_iter().enumerate() {
            let bytes = UiCommand::Add {
                parent: None,
                control,
            }
            .encode()
            .unwrap();
            assert_eq!(bytes[0], 0);
            assert_eq!(bytes[2], kind as u8);
        }

        let commands = [
            UiCommand::Remove { id: 1 },
            UiCommand::Clear,
            UiCommand::SetText {
                id: 1,
                text: "x".into(),
            },
            UiCommand::SetEnabled {
                id: 1,
                enabled: true,
            },
            UiCommand::SetChecked {
                id: 1,
                checked: false,
            },
            UiCommand::SetItems {
                id: 1,
                items: vec!["x".into()],
                selected: None,
            },
            UiCommand::SetSelected {
                id: 1,
                selected: Some(0),
            },
            UiCommand::SetNumber { id: 1, value: 2 },
            UiCommand::AppendLog {
                id: 1,
                line: "x".into(),
            },
            UiCommand::ClearLog { id: 1 },
        ];
        for (kind, command) in commands.into_iter().enumerate() {
            assert_eq!(command.encode().unwrap()[0], kind as u8 + 1);
        }

        assert!(matches!(
            UiCommand::SetText {
                id: 1,
                text: "x".repeat(MAX_UI_PAYLOAD)
            }
            .encode(),
            Err(RepositoryError::InvalidUi(_))
        ));
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

    static EMPTY_CALLS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "system" fn empty_query(
        _: *mut c_void,
        _: u32,
        _: *const u8,
        _: usize,
        _: *mut u8,
        _: usize,
        required: *mut usize,
    ) -> Status {
        EMPTY_CALLS.fetch_add(1, Ordering::SeqCst);
        unsafe { *required = 0 };
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

        EMPTY_CALLS.store(0, Ordering::SeqCst);
        let host = RawHostApiV1 {
            abi_version: ABI_VERSION,
            context: null_mut(),
            query: Some(empty_query),
            set_status: None,
        };
        assert!(Repository::new(&host).query(0, &[]).unwrap().is_empty());
        assert_eq!(EMPTY_CALLS.load(Ordering::SeqCst), 1);
    }
}
