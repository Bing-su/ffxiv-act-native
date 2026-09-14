use std::{
    env,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
};

use ffxiv_act_native::{
    Event, Plugin, PluginInit, PluginResult, Repository, SubscriptionSet, export_plugin,
};

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
        Ok(PluginInit::new(Self(log), SubscriptionSet::ZONE_CHANGED))
    }

    fn on_event(&mut self, _: Repository<'_>, event: Event<'_>) -> PluginResult<()> {
        if let Event::ZoneChanged { id, name } = event {
            writeln!(self.0, "zone changed: {name} ({id})")?;
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
