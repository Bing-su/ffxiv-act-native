use std::path::Path;

use dotscope::{
    CilAssembly, CilObject, Error,
    metadata::{
        identity::{AssemblyIdentity, Identity},
        signatures::TypeSignature,
        tables::{AssemblyRaw, AssemblyRefRaw, ModuleRaw, ModuleRefRaw, TableDataOwned, TableId},
        typesystem::CilTypeRc,
    },
};
use sha1::{Digest, Sha1};

const COMMON_EVENTS: &[&str] = &[
    "NetworkReceived",
    "NetworkSent",
    "CombatantAdded",
    "CombatantRemoved",
    "PrimaryPlayerChanged",
    "ZoneChanged",
    "PlayerStatsChanged",
    "PartyListChanged",
    "LogLine",
    "ParsedLogLine",
    "ProcessChanged",
];
const REPOSITORY_METHODS: &[&str] = &[
    "GetSelectedLanguageID",
    "GetCurrentFFXIVProcess",
    "GetResourceDictionary",
    "GetCurrentTerritoryID",
    "GetCurrentPlayerID",
    "GetCombatantList",
    "GetPlayer",
    "GetServerTimestamp",
    "GetGameVersion",
    "IsChatLogAvailable",
    "GetAntiVirusNames",
    "GetGameRegion",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginConfig {
    pub assembly_name: String,
    pub native_dll_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("{input} is not a valid .NET assembly: {message}")]
    InvalidAssembly {
        input: &'static str,
        message: String,
    },
    #[error("{input} has no assembly manifest")]
    MissingAssemblyManifest { input: &'static str },
    #[error("required type {0} was not found")]
    MissingType(&'static str),
    #[error("{type_name} is missing required {member_kind} {member_name}")]
    MissingMember {
        type_name: &'static str,
        member_kind: &'static str,
        member_name: &'static str,
    },
    #[error("{type_name}.{member_name} has an ABI-incompatible signature")]
    IncompatibleSignature {
        type_name: &'static str,
        member_name: &'static str,
    },
    #[error("invalid plugin configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("could not write managed shim: {0}")]
    Write(String),
}

pub fn generate(
    act_exe: &[u8],
    ffxiv_common: &[u8],
    config: &PluginConfig,
) -> Result<Vec<u8>, GenerateError> {
    validate_config(config)?;
    let act = parse(act_exe, "Advanced Combat Tracker.exe")?;
    let common = parse(ffxiv_common, "FFXIV_ACT_Plugin.Common.dll")?;
    validate_contracts(&act, &common)?;
    emit(&act, &common, config)
}

fn parse(bytes: &[u8], input: &'static str) -> Result<CilObject, GenerateError> {
    CilObject::from_mem(bytes.to_vec()).map_err(|error| GenerateError::InvalidAssembly {
        input,
        message: error.to_string(),
    })
}

fn validate_config(config: &PluginConfig) -> Result<(), GenerateError> {
    if config.assembly_name.is_empty() || config.assembly_name.contains(['/', '\\', '\0']) {
        return Err(GenerateError::InvalidConfig(
            "assembly_name must be a non-empty simple name",
        ));
    }
    let native = Path::new(&config.native_dll_name);
    if native.file_name().and_then(|value| value.to_str()) != Some(config.native_dll_name.as_str())
        || !config
            .native_dll_name
            .to_ascii_lowercase()
            .ends_with(".dll")
    {
        return Err(GenerateError::InvalidConfig(
            "native_dll_name must be a .dll file name without a path",
        ));
    }
    Ok(())
}

fn validate_contracts(act: &CilObject, common: &CilObject) -> Result<(), GenerateError> {
    let plugin = find_type(act, "Advanced_Combat_Tracker.IActPluginV1").ok_or(
        GenerateError::MissingType("Advanced_Combat_Tracker.IActPluginV1"),
    )?;
    require_method(
        &plugin,
        "Advanced_Combat_Tracker.IActPluginV1",
        "InitPlugin",
        2,
        true,
    )?;
    require_method(
        &plugin,
        "Advanced_Combat_Tracker.IActPluginV1",
        "DeInitPlugin",
        0,
        true,
    )?;

    let subscription = find_type(common, "FFXIV_ACT_Plugin.Common.IDataSubscription").ok_or(
        GenerateError::MissingType("FFXIV_ACT_Plugin.Common.IDataSubscription"),
    )?;
    for name in COMMON_EVENTS {
        let event = subscription
            .events
            .iter()
            .find_map(|(_, event)| (event.name == *name).then_some(event))
            .ok_or(GenerateError::MissingMember {
                type_name: "FFXIV_ACT_Plugin.Common.IDataSubscription",
                member_kind: "event",
                member_name: name,
            })?;
        let delegate = event
            .event_type
            .upgrade()
            .ok_or(GenerateError::IncompatibleSignature {
                type_name: "FFXIV_ACT_Plugin.Common.IDataSubscription",
                member_name: name,
            })?;
        let arity = match *name {
            "PrimaryPlayerChanged" => 0,
            "CombatantAdded" | "CombatantRemoved" | "PlayerStatsChanged" | "ProcessChanged" => 1,
            "ZoneChanged" | "PartyListChanged" => 2,
            _ => 3,
        };
        require_method(
            &delegate,
            "FFXIV_ACT_Plugin.Common delegate",
            "Invoke",
            arity,
            true,
        )?;
    }

    let repository = find_type(common, "FFXIV_ACT_Plugin.Common.IDataRepository").ok_or(
        GenerateError::MissingType("FFXIV_ACT_Plugin.Common.IDataRepository"),
    )?;
    for name in REPOSITORY_METHODS {
        require_method(
            &repository,
            "FFXIV_ACT_Plugin.Common.IDataRepository",
            name,
            usize::from(*name == "GetResourceDictionary"),
            false,
        )?;
    }
    for name in [
        "FFXIV_ACT_Plugin.Common.Models.Combatant",
        "FFXIV_ACT_Plugin.Common.Models.NetworkBuff",
        "FFXIV_ACT_Plugin.Common.Models.Player",
        "FFXIV_ACT_Plugin.Common.Language",
        "FFXIV_ACT_Plugin.Common.ResourceType",
    ] {
        if find_type(common, name).is_none() {
            return Err(GenerateError::MissingType(name));
        }
    }
    Ok(())
}

fn find_type(assembly: &CilObject, fullname: &str) -> Option<CilTypeRc> {
    assembly.types().get_by_fullname(fullname, false)
}

fn require_method(
    ty: &CilTypeRc,
    type_name: &'static str,
    name: &'static str,
    arity: usize,
    returns_void: bool,
) -> Result<(), GenerateError> {
    let candidates = ty.find_methods(name);
    if candidates.is_empty() {
        return Err(GenerateError::MissingMember {
            type_name,
            member_kind: "method",
            member_name: name,
        });
    }
    if candidates.iter().any(|method| {
        method.signature.params.len() == arity
            && (!returns_void || matches!(method.signature.return_type.base, TypeSignature::Void))
    }) {
        Ok(())
    } else {
        Err(GenerateError::IncompatibleSignature {
            type_name,
            member_name: name,
        })
    }
}

fn emit(
    act: &CilObject,
    common: &CilObject,
    config: &PluginConfig,
) -> Result<Vec<u8>, GenerateError> {
    let act_identity = act
        .identity()
        .ok_or(GenerateError::MissingAssemblyManifest {
            input: "Advanced Combat Tracker.exe",
        })?;
    let common_identity = common
        .identity()
        .ok_or(GenerateError::MissingAssemblyManifest {
            input: "FFXIV_ACT_Plugin.Common.dll",
        })?;
    let mut output = CilAssembly::from_bytes(
        include_bytes!("../managed/Shim.template.dll").to_vec(),
    )
    .map_err(|error| GenerateError::InvalidAssembly {
        input: "managed shim template",
        message: error.to_string(),
    })?;

    replace_assembly_name(&mut output, &config.assembly_name)?;
    replace_assembly_reference(&mut output, "Advanced Combat Tracker", &act_identity)?;
    replace_assembly_reference(&mut output, "FFXIV_ACT_Plugin.Common", &common_identity)?;
    replace_module_reference(
        &mut output,
        "__ACT_BRIDGE_NATIVE__.dll",
        &config.native_dll_name,
    )?;
    replace_user_string(
        &mut output,
        "__ACT_BRIDGE_NATIVE__.dll",
        &config.native_dll_name,
    )?;
    output.to_memory().map_err(write_error)
}

fn replace_assembly_name(output: &mut CilAssembly, name: &str) -> Result<(), GenerateError> {
    let assembly_name = output.string_add(name).map_err(write_error)?.placeholder();
    let module_name = output
        .string_add(&format!("{name}.dll"))
        .map_err(write_error)?
        .placeholder();
    let tables = output
        .view()
        .tables()
        .ok_or(GenerateError::MissingAssemblyManifest {
            input: "managed shim template",
        })?;
    let mut assembly = tables
        .table::<AssemblyRaw>()
        .and_then(|table| table.get(1).ok().flatten())
        .ok_or(GenerateError::MissingAssemblyManifest {
            input: "managed shim template",
        })?;
    let mut module = tables
        .table::<ModuleRaw>()
        .and_then(|table| table.get(1).ok().flatten())
        .ok_or(GenerateError::MissingMember {
            type_name: "managed shim template",
            member_kind: "module",
            member_name: "<Module>",
        })?;
    assembly.name = assembly_name;
    assembly.major_version = 1;
    assembly.minor_version = 0;
    assembly.build_number = 0;
    assembly.revision_number = 0;
    module.name = module_name;
    output
        .table_row_update(TableId::Assembly, 1, TableDataOwned::Assembly(assembly))
        .map_err(write_error)?;
    output
        .table_row_update(TableId::Module, 1, TableDataOwned::Module(module))
        .map_err(write_error)
}

fn replace_assembly_reference(
    output: &mut CilAssembly,
    old_name: &'static str,
    identity: &AssemblyIdentity,
) -> Result<(), GenerateError> {
    let view = output.view();
    let strings = view.strings().ok_or(GenerateError::MissingType(old_name))?;
    let mut row = view
        .tables()
        .and_then(|tables| tables.table::<AssemblyRefRaw>())
        .and_then(|table| {
            table.iter().filter_map(Result::ok).find(|row| {
                strings
                    .get(row.name as usize)
                    .is_ok_and(|name| name == old_name)
            })
        })
        .ok_or(GenerateError::MissingType(old_name))?;
    let rid = row.rid;
    row.name = output
        .string_add(&identity.name)
        .map_err(write_error)?
        .placeholder();
    row.major_version = u32::from(identity.version.major);
    row.minor_version = u32::from(identity.version.minor);
    row.build_number = u32::from(identity.version.build);
    row.revision_number = u32::from(identity.version.revision);
    row.flags = 0;
    row.public_key_or_token = match &identity.strong_name {
        Some(strong_name) => output
            .blob_add(&public_key_token(strong_name))
            .map_err(write_error)?
            .placeholder(),
        None => 0,
    };
    row.culture = match identity.culture.as_deref() {
        Some(culture) if !culture.is_empty() => output
            .string_add(culture)
            .map_err(write_error)?
            .placeholder(),
        _ => 0,
    };
    row.hash_value = 0;
    output
        .table_row_update(TableId::AssemblyRef, rid, TableDataOwned::AssemblyRef(row))
        .map_err(write_error)
}

fn public_key_token(identity: &Identity) -> [u8; 8] {
    let value = match identity {
        Identity::Token(token) => return token.to_be_bytes(),
        Identity::PubKey(key) | Identity::EcmaKey(key) => Sha1::digest(key),
    };
    let mut token = [0; 8];
    token.copy_from_slice(&value[value.len() - 8..]);
    token.reverse();
    token
}

fn replace_module_reference(
    output: &mut CilAssembly,
    old_name: &'static str,
    new_name: &str,
) -> Result<(), GenerateError> {
    let view = output.view();
    let strings = view.strings().ok_or(GenerateError::MissingMember {
        type_name: "managed shim template",
        member_kind: "module reference",
        member_name: old_name,
    })?;
    let mut row = view
        .tables()
        .and_then(|tables| tables.table::<ModuleRefRaw>())
        .and_then(|table| {
            table.iter().filter_map(Result::ok).find(|row| {
                strings
                    .get(row.name as usize)
                    .is_ok_and(|name| name == old_name)
            })
        })
        .ok_or(GenerateError::MissingMember {
            type_name: "managed shim template",
            member_kind: "module reference",
            member_name: old_name,
        })?;
    let rid = row.rid;
    row.name = output
        .string_add(new_name)
        .map_err(write_error)?
        .placeholder();
    output
        .table_row_update(TableId::ModuleRef, rid, TableDataOwned::ModuleRef(row))
        .map_err(write_error)
}

fn replace_user_string(
    output: &mut CilAssembly,
    old_value: &'static str,
    new_value: &str,
) -> Result<(), GenerateError> {
    let indexes: Vec<_> = output
        .view()
        .userstrings()
        .into_iter()
        .flat_map(|heap| heap.iter())
        .filter_map(|(index, value)| (value.to_string_lossy() == old_value).then_some(index as u32))
        .collect();
    if indexes.is_empty() {
        return Err(GenerateError::MissingMember {
            type_name: "managed shim template",
            member_kind: "user string",
            member_name: old_value,
        });
    }
    for index in indexes {
        output
            .userstring_update(index, new_value)
            .map_err(write_error)?;
    }
    Ok(())
}

fn write_error(error: Error) -> GenerateError {
    GenerateError::Write(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        fs::{read, remove_file, write},
        path::{Path, PathBuf},
        process::{Command, id},
    };

    use dotscope::{ValidationConfig, metadata::tables::TypeDefRaw};

    const ACT: &[u8] = include_bytes!("../managed/fixtures/Advanced Combat Tracker.exe");
    const COMMON: &[u8] = include_bytes!("../managed/fixtures/FFXIV_ACT_Plugin.Common.dll");

    #[test]
    fn rejects_paths_and_non_dll_native_names() {
        for name in ["", "native", "dir/native.dll", "dir\\native.dll"] {
            let config = PluginConfig {
                assembly_name: "Test".into(),
                native_dll_name: name.into(),
            };
            assert!(validate_config(&config).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn emits_configured_managed_plugin() {
        let bytes = generate(
            ACT,
            COMMON,
            &PluginConfig {
                assembly_name: "Example".into(),
                native_dll_name: "example.dll".into(),
            },
        )
        .unwrap();
        let generated =
            CilObject::from_mem_with_validation(bytes, ValidationConfig::disabled()).unwrap();
        assert_eq!(generated.assembly().unwrap().name, "Example");
        assert_eq!(generated.module().unwrap().name, "Example.dll");
        assert!(
            generated
                .refs_assembly()
                .iter()
                .any(|entry| entry.value().name == "FFXIV_ACT_Plugin.Common")
        );
        assert!(
            !generated
                .refs_assembly()
                .iter()
                .any(|entry| entry.value().name == "FFXIV_ACT_Plugin")
        );
        assert!(
            generated
                .refs_module()
                .iter()
                .any(|entry| entry.value().name == "example.dll")
        );
        assert!(
            !generated
                .refs_module()
                .iter()
                .any(|entry| entry.value().name == "__ACT_BRIDGE_NATIVE__.dll")
        );
        let strings = generated.strings().unwrap();
        let plugin_types = generated
            .tables()
            .unwrap()
            .table::<TypeDefRaw>()
            .unwrap()
            .iter()
            .filter_map(Result::ok)
            .filter(|row| {
                strings
                    .get(row.type_name as usize)
                    .is_ok_and(|name| name == "Plugin")
            })
            .count();
        assert_eq!(plugin_types, 1);
    }

    #[test]
    #[cfg(windows)]
    fn managed_template_matches_source_when_csc_is_available() {
        let csc = PathBuf::from(env::var_os("WINDIR").unwrap_or_default())
            .join(r"Microsoft.NET\Framework\v4.0.30319\csc.exe");
        let act = env::var_os("ACT_BRIDGE_ACT_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env::var_os("APPDATA").unwrap_or_default())
                    .join(r"Advanced Combat Tracker\Advanced Combat Tracker.exe")
            });
        let common = env::var_os("ACT_BRIDGE_COMMON_DLL")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(r"FFXIV_ACT_Plugin_SDK_3.0.3.0\SDK\FFXIV_ACT_Plugin.Common.dll")
            });
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("managed/Shim.cs")
            .display()
            .to_string()
            .replace('/', r"\");
        if !csc.exists() || !act.exists() || !common.exists() {
            return;
        }

        let directory = env::temp_dir().join(format!("act-bridge-template-{}", id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("Shim.template.dll");
        let status = Command::new(csc)
            .args(["/nologo", "/target:library"])
            .arg(format!("/out:{}", output.display()))
            .arg(format!("/reference:{}", act.display()))
            .arg(format!("/reference:{}", common.display()))
            .arg("/reference:System.Windows.Forms.dll")
            .arg(source)
            .status()
            .unwrap();
        assert!(status.success(), "failed to compile managed/Shim.cs");

        fn normalize_compiler_ids(image: &mut [u8]) {
            let pe = u32::from_le_bytes(image[0x3c..0x40].try_into().unwrap()) as usize;
            image[pe + 8..pe + 12].fill(0);

            let object =
                CilObject::from_mem_with_validation(image.to_vec(), ValidationConfig::disabled())
                    .unwrap();
            let mvid = object.module().unwrap().mvid.to_bytes();
            let offset = image
                .windows(mvid.len())
                .position(|bytes| bytes == mvid)
                .unwrap();
            image[offset..offset + mvid.len()].fill(0);
        }

        let mut checked_in = include_bytes!("../managed/Shim.template.dll").to_vec();
        let mut rebuilt = read(&output).unwrap();
        normalize_compiler_ids(&mut checked_in);
        normalize_compiler_ids(&mut rebuilt);
        remove_file(output).unwrap();
        std::fs::remove_dir(directory).unwrap();

        assert_eq!(
            checked_in, rebuilt,
            "managed/Shim.template.dll is stale; rebuild it with `task shim`"
        );
    }

    #[test]
    fn optional_real_sdk_generation() {
        let (Ok(act_path), Ok(common_path)) = (
            env::var("ACT_BRIDGE_ACT_EXE"),
            env::var("ACT_BRIDGE_COMMON_DLL"),
        ) else {
            return;
        };
        let bytes = generate(
            &read(act_path).unwrap(),
            &read(common_path).unwrap(),
            &PluginConfig {
                assembly_name: "ActBridge.Integration".into(),
                native_dll_name: "act_bridge_integration.dll".into(),
            },
        )
        .unwrap();
        CilObject::from_mem_with_validation(bytes.clone(), ValidationConfig::disabled()).unwrap();
        if cfg!(windows) {
            let configured = env::var_os("ACT_BRIDGE_OUTPUT_DLL").map(PathBuf::from);
            let output = configured
                .clone()
                .unwrap_or_else(|| env::temp_dir().join(format!("act-bridge-{}.dll", id())));
            write(&output, bytes).unwrap();
            let result = Command::new("powershell.exe").args([
                "-NoProfile", "-Command",
                "$act=[Reflection.Assembly]::LoadFile($env:ACT_BRIDGE_ACT_EXE); $common=[Reflection.Assembly]::LoadFile($env:ACT_BRIDGE_COMMON_DLL); $shim=[Reflection.Assembly]::LoadFile($env:ACT_BRIDGE_GENERATED); $plugins=@($shim.GetTypes() | Where-Object { $_.GetInterfaces().FullName -contains 'Advanced_Combat_Tracker.IActPluginV1' }); if($plugins.Count -ne 1){ throw 'expected one ACT plugin type' }",
            ]).env("ACT_BRIDGE_GENERATED", &output).status().unwrap();
            if configured.is_none() {
                remove_file(output).unwrap();
            }
            assert!(
                result.success(),
                ".NET Framework rejected the generated shim"
            );
        }
    }
}
