use std::{
    env,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
};

use ffxiv_act_native::{
    Event, Plugin, PluginInit, PluginResult, Repository, SubscriptionSet, UiCommand, UiControl,
    UiEvent, export_plugin,
};

const OPTIONS: u32 = 1;
const ENABLED: u32 = 2;
const REFRESH: u32 = 3;
const LANGUAGE: u32 = 4;
const LIMIT: u32 = 5;
const LOG: u32 = 6;
const CONTROLS: u32 = 7;

struct FfxivPlugin(BufWriter<File>);

impl Plugin for FfxivPlugin {
    fn init(repository: Repository<'_>) -> PluginResult<PluginInit<Self>> {
        let mut log = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(env::temp_dir().join("FfxivActNativeExample.log"))?,
        );
        writeln!(
            log,
            "started; current player={:#010X}",
            repository.current_player_id()?
        )?;
        repository.ui(UiCommand::Add {
            parent: None,
            control: UiControl::Column { id: CONTROLS },
        })?;
        repository.ui(UiCommand::Add {
            parent: Some(CONTROLS),
            control: UiControl::Row { id: OPTIONS },
        })?;
        repository.ui(UiCommand::Add {
            parent: Some(OPTIONS),
            control: UiControl::CheckBox {
                id: ENABLED,
                text: "Enabled".into(),
                checked: true,
            },
        })?;
        repository.ui(UiCommand::Add {
            parent: Some(OPTIONS),
            control: UiControl::Button {
                id: REFRESH,
                text: "Refresh".into(),
            },
        })?;
        repository.ui(UiCommand::Add {
            parent: Some(CONTROLS),
            control: UiControl::ComboBox {
                id: LANGUAGE,
                items: vec!["English".into(), "Japanese".into()],
                selected: Some(0),
            },
        })?;
        repository.ui(UiCommand::Add {
            parent: Some(CONTROLS),
            control: UiControl::Number {
                id: LIMIT,
                min: 1,
                max: 100,
                step: 1,
                value: 10,
            },
        })?;
        repository.ui(UiCommand::Add {
            parent: None,
            control: UiControl::LogList {
                id: LOG,
                max_rows: 1_000,
            },
        })?;
        Ok(PluginInit::new(Self(log), SubscriptionSet::ZONE_CHANGED))
    }

    fn on_event(&mut self, repository: Repository<'_>, event: Event<'_>) -> PluginResult<()> {
        let message = match event {
            Event::ZoneChanged { id, name } => Some(format!("zone changed: {name} ({id})")),
            Event::Ui(UiEvent::Clicked { id: REFRESH }) => Some("refresh clicked".into()),
            Event::Ui(UiEvent::CheckedChanged {
                id: ENABLED,
                checked,
            }) => Some(format!("enabled: {checked}")),
            Event::Ui(UiEvent::SelectionChanged {
                id: LANGUAGE,
                selected,
            }) => Some(format!("language: {selected:?}")),
            Event::Ui(UiEvent::NumberChanged { id: LIMIT, value }) => {
                Some(format!("limit: {value}"))
            }
            Event::Ui(UiEvent::Error { id, message }) => {
                writeln!(self.0, "UI error for {id:?}: {message}")?;
                self.0.flush()?;
                None
            }
            _ => None,
        };
        if let Some(message) = message {
            repository.ui(UiCommand::AppendLog {
                id: LOG,
                line: message.clone(),
            })?;
            writeln!(self.0, "{message}")?;
            self.0.flush()?;
        }
        Ok(())
    }

    fn shutdown(&mut self, _: Repository<'_>) -> PluginResult<()> {
        writeln!(self.0, "stopped")?;
        self.0.flush()?;
        Ok(())
    }
}

export_plugin!(FfxivPlugin);
