use std::{env, fs, path::PathBuf};

use act_bridge_gen::{PluginConfig, generate};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=ACT_BRIDGE_ACT_EXE");
    println!("cargo:rerun-if-env-changed=ACT_BRIDGE_COMMON_DLL");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let act = env::var_os("ACT_BRIDGE_ACT_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("APPDATA").unwrap())
                .join(r"Advanced Combat Tracker\Advanced Combat Tracker.exe")
        });
    let common = env::var_os("ACT_BRIDGE_COMMON_DLL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            manifest.join(r"..\..\FFXIV_ACT_Plugin_SDK_3.0.3.0\SDK\FFXIV_ACT_Plugin.Common.dll")
        });
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap())
        .ancestors()
        .nth(3)
        .unwrap()
        .join("ActBridgeExample.ACT.dll");

    fs::write(
        output,
        generate(
            &fs::read(act)?,
            &fs::read(common)?,
            &PluginConfig {
                assembly_name: "ActBridgeExample.ACT".into(),
                native_dll_name: "ffxiv_plugin.dll".into(),
            },
        )?,
    )?;
    Ok(())
}
