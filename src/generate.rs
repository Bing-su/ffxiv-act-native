use std::{borrow::Cow, collections::BTreeSet, path::Path};

use dotnetdll::prelude::*;

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

fn parse<'a>(bytes: &'a [u8], input: &'static str) -> Result<Resolution<'a>, GenerateError> {
    Resolution::parse(bytes, ReadOptions::default()).map_err(|error| {
        GenerateError::InvalidAssembly {
            input,
            message: error.to_string(),
        }
    })
}

fn validate_config(config: &PluginConfig) -> Result<(), GenerateError> {
    if config.assembly_name.is_empty() || config.assembly_name.contains(['/', '\\', '\0']) {
        return Err(GenerateError::InvalidConfig(
            "assembly_name must be a non-empty simple name",
        ));
    }
    let native = Path::new(&config.native_dll_name);
    if native.file_name().and_then(|v| v.to_str()) != Some(config.native_dll_name.as_str())
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

fn validate_contracts(act: &Resolution<'_>, common: &Resolution<'_>) -> Result<(), GenerateError> {
    let plugin = find_type(act, "Advanced_Combat_Tracker", "IActPluginV1").ok_or(
        GenerateError::MissingType("Advanced_Combat_Tracker.IActPluginV1"),
    )?;
    require_methods(
        plugin,
        "Advanced_Combat_Tracker.IActPluginV1",
        &["InitPlugin", "DeInitPlugin"],
    )?;
    require_method_arity(
        plugin,
        "Advanced_Combat_Tracker.IActPluginV1",
        "InitPlugin",
        2,
    )?;
    require_method_arity(
        plugin,
        "Advanced_Combat_Tracker.IActPluginV1",
        "DeInitPlugin",
        0,
    )?;

    let subscription = find_type(common, "FFXIV_ACT_Plugin.Common", "IDataSubscription").ok_or(
        GenerateError::MissingType("FFXIV_ACT_Plugin.Common.IDataSubscription"),
    )?;
    let events: BTreeSet<_> = subscription
        .events
        .iter()
        .map(|event| event.name.as_ref())
        .collect();
    for event in COMMON_EVENTS {
        if !events.contains(event) {
            return Err(GenerateError::MissingMember {
                type_name: "FFXIV_ACT_Plugin.Common.IDataSubscription",
                member_kind: "event",
                member_name: event,
            });
        }
        let event_definition = subscription
            .events
            .iter()
            .find(|candidate| candidate.name == *event)
            .and_then(|candidate| {
                let delegate_name = candidate.delegate_type.show(common);
                common
                    .type_definitions
                    .iter()
                    .find(|ty| delegate_name.ends_with(ty.name.as_ref()))
            })
            .ok_or(GenerateError::IncompatibleSignature {
                type_name: "FFXIV_ACT_Plugin.Common.IDataSubscription",
                member_name: event,
            })?;
        let expected = match *event {
            "PrimaryPlayerChanged" => 0,
            "CombatantAdded" | "CombatantRemoved" | "PlayerStatsChanged" | "ProcessChanged" => 1,
            "ZoneChanged" | "PartyListChanged" => 2,
            _ => 3,
        };
        require_method_arity(
            event_definition,
            "FFXIV_ACT_Plugin.Common delegate",
            "Invoke",
            expected,
        )?;
    }

    let repository = find_type(common, "FFXIV_ACT_Plugin.Common", "IDataRepository").ok_or(
        GenerateError::MissingType("FFXIV_ACT_Plugin.Common.IDataRepository"),
    )?;
    require_methods(
        repository,
        "FFXIV_ACT_Plugin.Common.IDataRepository",
        REPOSITORY_METHODS,
    )?;
    for method in REPOSITORY_METHODS {
        require_method_arity(
            repository,
            "FFXIV_ACT_Plugin.Common.IDataRepository",
            method,
            usize::from(*method == "GetResourceDictionary"),
        )?;
    }
    for (namespace, name) in [
        ("FFXIV_ACT_Plugin.Common.Models", "Combatant"),
        ("FFXIV_ACT_Plugin.Common.Models", "NetworkBuff"),
        ("FFXIV_ACT_Plugin.Common.Models", "Player"),
        ("FFXIV_ACT_Plugin.Common", "Language"),
        ("FFXIV_ACT_Plugin.Common", "ResourceType"),
    ] {
        if find_type(common, namespace, name).is_none() {
            return Err(GenerateError::MissingType(match name {
                "Combatant" => "FFXIV_ACT_Plugin.Common.Models.Combatant",
                "NetworkBuff" => "FFXIV_ACT_Plugin.Common.Models.NetworkBuff",
                "Player" => "FFXIV_ACT_Plugin.Common.Models.Player",
                "Language" => "FFXIV_ACT_Plugin.Common.Language",
                _ => "FFXIV_ACT_Plugin.Common.ResourceType",
            }));
        }
    }
    Ok(())
}

fn find_type<'a>(
    res: &'a Resolution<'_>,
    namespace: &str,
    name: &str,
) -> Option<&'a TypeDefinition<'a>> {
    // TypeDefinition's data lifetime can outlive this borrow; the returned borrow cannot.
    res.type_definitions
        .iter()
        .find(|ty| ty.namespace.as_deref() == Some(namespace) && ty.name == name)
}

fn require_methods(
    ty: &TypeDefinition<'_>,
    type_name: &'static str,
    required: &'static [&'static str],
) -> Result<(), GenerateError> {
    let methods: BTreeSet<_> = ty
        .methods
        .iter()
        .map(|method| method.name.as_ref())
        .collect();
    for method in required {
        if !methods.contains(method) {
            return Err(GenerateError::MissingMember {
                type_name,
                member_kind: "method",
                member_name: method,
            });
        }
    }
    Ok(())
}

fn require_method_arity(
    ty: &TypeDefinition<'_>,
    type_name: &'static str,
    name: &'static str,
    arity: usize,
) -> Result<(), GenerateError> {
    if ty
        .methods
        .iter()
        .any(|method| method.name == name && method.signature.parameters.len() == arity)
    {
        Ok(())
    } else {
        Err(GenerateError::IncompatibleSignature {
            type_name,
            member_name: name,
        })
    }
}

fn emit(
    act: &Resolution<'_>,
    common: &Resolution<'_>,
    config: &PluginConfig,
) -> Result<Vec<u8>, GenerateError> {
    let act_assembly = act
        .assembly
        .as_ref()
        .ok_or(GenerateError::MissingAssemblyManifest {
            input: "Advanced Combat Tracker.exe",
        })?;
    let common_assembly =
        common
            .assembly
            .as_ref()
            .ok_or(GenerateError::MissingAssemblyManifest {
                input: "FFXIV_ACT_Plugin.Common.dll",
            })?;

    let module_name = format!("{}.dll", config.assembly_name);
    let mut output = Resolution::new(Module::new(module_name));
    let mut assembly = Assembly::new(config.assembly_name.clone());
    assembly.version = Version {
        major: 1,
        minor: 0,
        build: 0,
        revision: 0,
    };
    output.assembly = Some(assembly);

    let act_ref = output.push_assembly_reference(reference_from(act_assembly));
    // Keep the official SDK identity in metadata, but do not place Common types on
    // the entry class. A later helper can therefore be JIT-loaded after FFXIV starts.
    let common_ref = output.push_assembly_reference(reference_from(common_assembly));
    let mscorlib = copy_reference(act, &mut output, "mscorlib")?;
    let winforms = copy_reference(act, &mut output, "System.Windows.Forms")?;

    let object = output.push_type_reference(ExternalTypeReference::new(
        Some(Cow::Borrowed("System")),
        "Object",
        ResolutionScope::Assembly(mscorlib),
    ));
    let subscription_interface = output.push_type_reference(ExternalTypeReference::new(
        Some(Cow::Borrowed("FFXIV_ACT_Plugin.Common")),
        "IDataSubscription",
        ResolutionScope::Assembly(common_ref),
    ));
    let repository_interface = output.push_type_reference(ExternalTypeReference::new(
        Some(Cow::Borrowed("FFXIV_ACT_Plugin.Common")),
        "IDataRepository",
        ResolutionScope::Assembly(common_ref),
    ));
    let subscription_member: MemberType = BaseType::class(subscription_interface).into();
    let repository_member: MemberType = BaseType::class(repository_interface).into();
    let subscription_type: MethodType = subscription_member.clone().into();
    let repository_type: MethodType = repository_member.clone().into();

    // Common references are isolated in this non-public helper. The ACT entry
    // type below can load before Common is resolved, as recommended by ACT.
    let mut helper =
        TypeDefinition::new(Some(Cow::Borrowed("ActBridge.Generated")), "CommonBridge");
    helper.flags.sealed = true;
    helper.set_extends(object);
    let helper = output.push_type_definition(helper);
    let subscription_field = output.push_field(
        helper,
        Field::static_member(Accessibility::Private, "subscription", subscription_member),
    );
    let repository_field = output.push_field(
        helper,
        Field::static_member(Accessibility::Private, "repository", repository_member),
    );
    let helper_start = output.push_method(
        helper,
        Method::new(
            Accessibility::Assembly,
            msig! { static void (object, object) },
            "Start",
            Some(body::Method::new(asm! {
                LoadArgument 0;
                cast_class subscription_type.clone();
                store_static_field subscription_field;
                LoadArgument 1;
                cast_class repository_type.clone();
                store_static_field repository_field;
                Return;
            })),
        ),
    );
    let helper_stop = output.push_method(
        helper,
        Method::new(
            Accessibility::Assembly,
            msig! { static void () },
            "Stop",
            Some(body::Method::new(asm! {
                LoadNull;
                store_static_field subscription_field;
                LoadNull;
                store_static_field repository_field;
                Return;
            })),
        ),
    );
    let tab_page = output.push_type_reference(ExternalTypeReference::new(
        Some(Cow::Borrowed("System.Windows.Forms")),
        "TabPage",
        ResolutionScope::Assembly(winforms),
    ));
    let label = output.push_type_reference(ExternalTypeReference::new(
        Some(Cow::Borrowed("System.Windows.Forms")),
        "Label",
        ResolutionScope::Assembly(winforms),
    ));
    let interface = output.push_type_reference(ExternalTypeReference::new(
        Some(Cow::Borrowed("Advanced_Combat_Tracker")),
        "IActPluginV1",
        ResolutionScope::Assembly(act_ref),
    ));

    let mut entry = TypeDefinition::new(Some(Cow::Borrowed("ActBridge.Generated")), "Plugin");
    entry.flags.accessibility = dotnetdll::resolved::types::Accessibility::Public;
    entry.flags.sealed = true;
    entry.set_extends(object);
    entry.add_implementation(interface);
    let entry = output.push_type_definition(entry);
    ConstructorCache::new().define_default_ctor(&mut output, entry);

    let interface_type: MethodType = BaseType::class(interface).into();
    let tab_type: MethodType = BaseType::class(tab_page).into();
    let label_type: MethodType = BaseType::class(label).into();
    let init_declaration = output.push_method_reference(method_ref! {
        void @interface_type::InitPlugin(@tab_type, @label_type)
    });
    let deinit_declaration = output.push_method_reference(method_ref! {
        void @interface_type::DeInitPlugin()
    });
    let set_text = output.push_method_reference(method_ref! { void @label_type::set_Text(string) });

    let assembly_type: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System.Reflection")),
            "Assembly",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let path_type: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System.IO")),
            "Path",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let marshal_type: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System.Runtime.InteropServices")),
            "Marshal",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let get_assembly = output.push_method_reference(
        method_ref! { static @assembly_type @assembly_type::GetExecutingAssembly() },
    );
    let get_location =
        output.push_method_reference(method_ref! { string @assembly_type::get_Location() });
    let get_directory = output
        .push_method_reference(method_ref! { static string @path_type::GetDirectoryName(string) });
    let combine = output
        .push_method_reference(method_ref! { static string @path_type::Combine(string, string) });
    let alloc =
        output.push_method_reference(method_ref! { static nint @marshal_type::AllocHGlobal(int) });
    let free =
        output.push_method_reference(method_ref! { static void @marshal_type::FreeHGlobal(nint) });
    let write_i32 = output.push_method_reference(
        method_ref! { static void @marshal_type::WriteInt32(nint, int, int) },
    );
    let write_i64 = output.push_method_reference(
        method_ref! { static void @marshal_type::WriteInt64(nint, int, long) },
    );

    let native_handle = output.push_field(
        entry,
        Field::static_member(Accessibility::Private, "nativeHandle", ctype! { nint }),
    );
    let host_memory = output.push_field(
        entry,
        Field::static_member(Accessibility::Private, "hostMemory", ctype! { nint }),
    );
    let client_memory = output.push_field(
        entry,
        Field::static_member(Accessibility::Private, "clientMemory", ctype! { nint }),
    );

    let kernel32 = output.push_module_reference(ExternalModuleReference::new("kernel32.dll"));
    let native_module =
        output.push_module_reference(ExternalModuleReference::new(config.native_dll_name.clone()));
    let load_library = output.push_method(
        entry,
        Method {
            pinvoke: Some(PInvoke {
                no_mangle: true,
                character_set: CharacterSet::Unicode,
                ..PInvoke::new(kernel32, "LoadLibraryW")
            }),
            ..Method::new(
                Accessibility::Private,
                msig! { static nint (string) },
                "LoadLibraryW",
                None,
            )
        },
    );
    let free_library = output.push_method(
        entry,
        Method {
            pinvoke: Some(PInvoke {
                no_mangle: true,
                ..PInvoke::new(kernel32, "FreeLibrary")
            }),
            ..Method::new(
                Accessibility::Private,
                msig! { static bool (nint) },
                "FreeLibrary",
                None,
            )
        },
    );
    let native_entry = output.push_method(
        entry,
        Method {
            pinvoke: Some(PInvoke {
                no_mangle: true,
                ..PInvoke::new(native_module, "act_bridge_entry_v1")
            }),
            ..Method::new(
                Accessibility::Private,
                msig! { static int (nint, nint) },
                "act_bridge_entry_v1",
                None,
            )
        },
    );

    let act_globals: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("Advanced_Combat_Tracker")),
            "ActGlobals",
            ResolutionScope::Assembly(act_ref),
        )))
        .into();
    let form_act_main_member: MemberType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("Advanced_Combat_Tracker")),
            "FormActMain",
            ResolutionScope::Assembly(act_ref),
        )))
        .into();
    let form_act_main: MethodType = form_act_main_member.clone().into();
    let plugin_data_member: MemberType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("Advanced_Combat_Tracker")),
            "ActPluginData",
            ResolutionScope::Assembly(act_ref),
        )))
        .into();
    let plugin_data: MethodType = plugin_data_member.clone().into();
    let file_info_member: MemberType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System.IO")),
            "FileInfo",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let file_info: MethodType = file_info_member.clone().into();
    let list_ref = output.push_type_reference(ExternalTypeReference::new(
        Some(Cow::Borrowed("System.Collections.Generic")),
        "List`1",
        ResolutionScope::Assembly(mscorlib),
    ));
    let plugin_list: MethodType =
        BaseType::class(TypeSource::generic(list_ref, vec![plugin_data.clone()])).into();
    let enumerable: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System.Collections")),
            "IEnumerable",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let enumerator: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System.Collections")),
            "IEnumerator",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let system_type: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System")),
            "Type",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let property_info: MethodType =
        BaseType::class(output.push_type_reference(ExternalTypeReference::new(
            Some(Cow::Borrowed("System.Reflection")),
            "PropertyInfo",
            ResolutionScope::Assembly(mscorlib),
        )))
        .into();
    let object_type: MethodType = BaseType::class(object).into();

    let main_form = output
        .push_field_reference(field_ref! { #form_act_main_member @act_globals::oFormActMain });
    let get_plugins =
        output.push_method_reference(method_ref! { @plugin_list @form_act_main::get_ActPlugins() });
    let get_enumerator =
        output.push_method_reference(method_ref! { @enumerator @enumerable::GetEnumerator() });
    let move_next = output.push_method_reference(method_ref! { bool @enumerator::MoveNext() });
    let get_current =
        output.push_method_reference(method_ref! { object @enumerator::get_Current() });
    let plugin_file =
        output.push_field_reference(field_ref! { #file_info_member @plugin_data::pluginFile });
    let plugin_object = output.push_field_reference(field_ref! { object @plugin_data::pluginObj });
    let get_name = output.push_method_reference(method_ref! { string @file_info::get_Name() });
    let compare = output
        .push_method_reference(method_ref! { static int string::Compare(string, string, bool) });
    let get_type =
        output.push_method_reference(method_ref! { @system_type @object_type::GetType() });
    let get_property = output
        .push_method_reference(method_ref! { @property_info @system_type::GetProperty(string) });
    let get_value = output
        .push_method_reference(method_ref! { object @property_info::GetValue(object, object[]) });

    let find_services = output.push_method(
        entry,
        Method::new(
            Accessibility::Private,
            msig! { static bool () },
            "FindServices",
            Some(body::Method::with_locals(
                vec![
                    LocalVariable::new(enumerator.clone()),
                    LocalVariable::new(plugin_data.clone()),
                    LocalVariable::new(ctype! { object }),
                ],
                asm! {
                    load_static_field main_form;
                    call_virtual get_plugins;
                    call_virtual get_enumerator;
                    StoreLocal 0;
                    Branch check;
                @next
                    LoadLocal 0;
                    call_virtual get_current;
                    cast_class plugin_data.clone();
                    StoreLocal 1;
                    LoadLocal 1;
                    load_field plugin_file;
                    call_virtual get_name;
                    load_string "FFXIV_ACT_Plugin.dll";
                    LoadConstantInt32 1;
                    call compare;
                    BranchTruthy check;
                    LoadLocal 1;
                    load_field plugin_object;
                    Duplicate;
                    StoreLocal 2;
                    BranchFalsy check;
                    LoadLocal 2;
                    call_virtual get_type;
                    load_string "PluginStarted";
                    call_virtual get_property;
                    LoadLocal 2;
                    LoadNull;
                    call_virtual get_value;
                    UnboxIntoValue ctype! { bool };
                    BranchFalsy check;
                    LoadLocal 2;
                    call_virtual get_type;
                    load_string "DataSubscription";
                    call_virtual get_property;
                    LoadLocal 2;
                    LoadNull;
                    call_virtual get_value;
                    LoadLocal 2;
                    call_virtual get_type;
                    load_string "DataRepository";
                    call_virtual get_property;
                    LoadLocal 2;
                    LoadNull;
                    call_virtual get_value;
                    call helper_start;
                    LoadConstantInt32 1;
                    Return;
                @check
                    LoadLocal 0;
                    call_virtual move_next;
                    BranchTruthy next;
                    LoadConstantInt32 0;
                    Return;
                },
            )),
        ),
    );

    let init = output.push_method(
        entry,
        Method {
            virtual_member: true,
            sealed: true,
            ..Method::new(
                Accessibility::Public,
                msig! { void (@tab_type, @label_type) },
                "InitPlugin",
                Some(body::Method::new(asm! {
                    call find_services;
                    BranchTruthy services_found;
                    LoadArgument 2;
                    load_string "Rust bridge: enable FFXIV_ACT_Plugin first";
                    call_virtual set_text;
                    Return;
                @services_found
                    call get_assembly;
                    call_virtual get_location;
                    call get_directory;
                    load_string config.native_dll_name.clone();
                    call combine;
                    call load_library;
                    store_static_field native_handle;
                    load_static_field native_handle;
                    BranchTruthy native_loaded;
                    LoadArgument 2;
                    load_string "Rust bridge: native DLL load failed";
                    call_virtual set_text;
                    Return;
                @native_loaded
                    LoadConstantInt32 32;
                    call alloc;
                    store_static_field host_memory;
                    LoadConstantInt32 32;
                    call alloc;
                    store_static_field client_memory;
                    load_static_field host_memory;
                    LoadConstantInt32 0;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field host_memory;
                    LoadConstantInt32 8;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field host_memory;
                    LoadConstantInt32 16;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field host_memory;
                    LoadConstantInt32 24;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field host_memory;
                    LoadConstantInt32 0;
                    LoadConstantInt32 crate::ABI_VERSION as i32;
                    call write_i32;
                    load_static_field client_memory;
                    LoadConstantInt32 0;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field client_memory;
                    LoadConstantInt32 8;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field client_memory;
                    LoadConstantInt32 16;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field client_memory;
                    LoadConstantInt32 24;
                    LoadConstantInt64 0;
                    call write_i64;
                    load_static_field host_memory;
                    load_static_field client_memory;
                    call native_entry;
                    BranchFalsy native_started;
                    LoadArgument 2;
                    load_string "Rust bridge: plugin init failed";
                    call_virtual set_text;
                    Return;
                @native_started
                    LoadArgument 2;
                    load_string "Rust bridge loaded";
                    call_virtual set_text;
                    Return;
                })),
            )
        },
    );
    let deinit = output.push_method(
        entry,
        Method {
            virtual_member: true,
            sealed: true,
            ..Method::new(
                Accessibility::Public,
                msig! { void () },
                "DeInitPlugin",
                Some(body::Method::new(asm! {
                    call helper_stop;
                    load_static_field native_handle;
                    BranchFalsy done;
                    LoadConstantInt32 0;
                    Convert ConversionType::IntPtr;
                    LoadConstantInt32 0;
                    Convert ConversionType::IntPtr;
                    call native_entry;
                    Pop;
                    load_static_field host_memory;
                    call free;
                    load_static_field client_memory;
                    call free;
                    load_static_field native_handle;
                    call free_library;
                    Pop;
                @done
                    Return;
                })),
            )
        },
    );
    output[entry].overrides.push(MethodOverride {
        implementation: init.into(),
        declaration: init_declaration.into(),
    });
    output[entry].overrides.push(MethodOverride {
        implementation: deinit.into(),
        declaration: deinit_declaration.into(),
    });

    output
        .write(WriteOptions {
            // PE32 + ILONLY is the conventional AnyCPU managed image shape.
            is_32_bit: true,
            is_executable: false,
        })
        .map_err(|error| GenerateError::Write(error.to_string()))
}

fn reference_from(assembly: &Assembly<'_>) -> ExternalAssemblyReference<'static> {
    ExternalAssemblyReference {
        attributes: vec![],
        version: assembly.version,
        has_full_public_key: assembly.flags.has_full_public_key,
        public_key_or_token: assembly
            .public_key
            .as_ref()
            .map(|key| Cow::Owned(key.to_vec())),
        name: Cow::Owned(assembly.name.to_string()),
        culture: assembly
            .culture
            .as_ref()
            .map(|culture| Cow::Owned(culture.to_string())),
        hash_value: None,
    }
}

fn copy_reference(
    source: &Resolution<'_>,
    target: &mut Resolution<'static>,
    name: &'static str,
) -> Result<AssemblyRefIndex, GenerateError> {
    let reference = source
        .assembly_references
        .iter()
        .find(|reference| reference.name == name)
        .ok_or(GenerateError::MissingType(name))?;
    Ok(target.push_assembly_reference(ExternalAssemblyReference {
        attributes: vec![],
        version: reference.version,
        has_full_public_key: reference.has_full_public_key,
        public_key_or_token: reference
            .public_key_or_token
            .as_ref()
            .map(|value| Cow::Owned(value.to_vec())),
        name: Cow::Owned(reference.name.to_string()),
        culture: reference
            .culture
            .as_ref()
            .map(|value| Cow::Owned(value.to_string())),
        hash_value: reference
            .hash_value
            .as_ref()
            .map(|value| Cow::Owned(value.to_vec())),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_act() -> Vec<u8> {
        let mut res = Resolution::new(Module::new("Advanced Combat Tracker.exe"));
        res.assembly = Some(Assembly::new("Advanced Combat Tracker"));
        let mscorlib = res.push_assembly_reference(ExternalAssemblyReference::new("mscorlib"));
        res.push_assembly_reference(ExternalAssemblyReference::new("System.Windows.Forms"));
        let object = res.push_type_reference(type_ref! { System.Object in #mscorlib });
        let marker = res.push_type_definition(TypeDefinition::new(None, "Marker"));
        res[marker].set_extends(object);
        let iface = res.push_type_definition(TypeDefinition::new(
            Some("Advanced_Combat_Tracker".into()),
            "IActPluginV1",
        ));
        res[iface].flags.kind = Kind::Interface;
        res[iface].flags.abstract_type = true;
        res.push_method(
            iface,
            Method {
                virtual_member: true,
                abstract_member: true,
                ..Method::new(
                    Accessibility::Public,
                    msig! { void (object, object) },
                    "InitPlugin",
                    None,
                )
            },
        );
        res.push_method(
            iface,
            Method {
                virtual_member: true,
                abstract_member: true,
                ..Method::new(
                    Accessibility::Public,
                    msig! { void () },
                    "DeInitPlugin",
                    None,
                )
            },
        );
        res.write(WriteOptions {
            is_32_bit: false,
            is_executable: true,
        })
        .unwrap()
    }

    fn fixture_common() -> Vec<u8> {
        let mut res = Resolution::new(Module::new("FFXIV_ACT_Plugin.Common.dll"));
        res.assembly = Some(Assembly::new("FFXIV_ACT_Plugin.Common"));
        let mscorlib = res.push_assembly_reference(ExternalAssemblyReference::new("mscorlib"));
        let object = res.push_type_reference(type_ref! { System.Object in #mscorlib });
        let subscription = res.push_type_definition(TypeDefinition::new(
            Some("FFXIV_ACT_Plugin.Common".into()),
            "IDataSubscription",
        ));
        res[subscription].flags.kind = Kind::Interface;
        res[subscription].flags.abstract_type = true;
        let repository = res.push_type_definition(TypeDefinition::new(
            Some("FFXIV_ACT_Plugin.Common".into()),
            "IDataRepository",
        ));
        res[repository].flags.kind = Kind::Interface;
        res[repository].flags.abstract_type = true;
        let mut models = vec![];
        for (namespace, name) in [
            ("FFXIV_ACT_Plugin.Common.Models", "Combatant"),
            ("FFXIV_ACT_Plugin.Common.Models", "NetworkBuff"),
            ("FFXIV_ACT_Plugin.Common.Models", "Player"),
            ("FFXIV_ACT_Plugin.Common", "Language"),
            ("FFXIV_ACT_Plugin.Common", "ResourceType"),
        ] {
            models
                .push(res.push_type_definition(TypeDefinition::new(Some(namespace.into()), name)));
        }
        let delegates: Vec<_> = COMMON_EVENTS
            .iter()
            .map(|name| {
                res.push_type_definition(TypeDefinition::new(
                    Some("FFXIV_ACT_Plugin.Common".into()),
                    format!("{name}Delegate"),
                ))
            })
            .collect();
        for (name, delegate) in COMMON_EVENTS.iter().zip(delegates.iter().copied()) {
            let arity = match *name {
                "PrimaryPlayerChanged" => 0,
                "CombatantAdded" | "CombatantRemoved" | "PlayerStatsChanged" | "ProcessChanged" => {
                    1
                }
                "ZoneChanged" | "PartyListChanged" => 2,
                _ => 3,
            };
            let signature = dotnetdll::resolved::signature::ManagedMethod::instance(
                dotnetdll::resolved::signature::ReturnType::VOID,
                vec![dotnetdll::resolved::signature::Parameter::value(ctype! { object }); arity],
            );
            res.push_method(
                delegate,
                Method::new(Accessibility::Public, signature, "Invoke", None),
            );
        }
        for (name, delegate) in COMMON_EVENTS.iter().zip(delegates) {
            let handler_member: MemberType = BaseType::class(delegate).into();
            let handler_method: MethodType = handler_member.clone().into();
            let add = Method {
                virtual_member: true,
                abstract_member: true,
                ..Method::new(
                    Accessibility::Public,
                    msig! { void (@handler_method) },
                    format!("add_{name}"),
                    None,
                )
            };
            let remove = Method {
                virtual_member: true,
                abstract_member: true,
                ..Method::new(
                    Accessibility::Public,
                    msig! { void (@handler_method) },
                    format!("remove_{name}"),
                    None,
                )
            };
            res.push_event(
                subscription,
                dotnetdll::resolved::members::Event::new(
                    *name,
                    handler_member.clone(),
                    add,
                    remove,
                ),
            );
        }
        for name in REPOSITORY_METHODS {
            let signature = if *name == "GetResourceDictionary" {
                msig! { object (object) }
            } else {
                msig! { object () }
            };
            res.push_method(
                repository,
                Method {
                    virtual_member: true,
                    abstract_member: true,
                    ..Method::new(Accessibility::Public, signature, *name, None)
                },
            );
        }
        let mut ctors = ConstructorCache::new();
        for ty in models {
            res[ty].set_extends(object);
            ctors.define_default_ctor(&mut res, ty);
        }
        res.write(WriteOptions {
            is_32_bit: false,
            is_executable: false,
        })
        .unwrap()
    }

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
    fn emits_one_act_plugin_without_referencing_ffxiv_plugin() {
        let bytes = generate(
            &fixture_act(),
            &fixture_common(),
            &PluginConfig {
                assembly_name: "Example".into(),
                native_dll_name: "example.dll".into(),
            },
        )
        .unwrap();
        let generated = Resolution::parse(&bytes, ReadOptions::default()).unwrap();
        let implementations: Vec<_> = generated
            .type_definitions
            .iter()
            .filter(|ty| {
                ty.implements.iter().any(|(_, implemented)| {
                    implemented
                        .show(&generated)
                        .ends_with("Advanced_Combat_Tracker.IActPluginV1")
                })
            })
            .collect();
        assert_eq!(implementations.len(), 1);
        assert!(
            generated
                .assembly_references
                .iter()
                .any(|reference| reference.name == "FFXIV_ACT_Plugin.Common")
        );
        assert!(
            !generated
                .assembly_references
                .iter()
                .any(|reference| reference.name == "FFXIV_ACT_Plugin")
        );
        let entry = implementations[0];
        assert!(entry.fields.iter().all(|field| {
            !field
                .return_type
                .show(&generated)
                .contains("FFXIV_ACT_Plugin.Common")
        }));
        assert!(entry.methods.iter().all(|method| {
            !method
                .signature
                .show(&generated)
                .contains("FFXIV_ACT_Plugin.Common")
        }));
        let helper = find_type(&generated, "ActBridge.Generated", "CommonBridge").unwrap();
        assert!(helper.methods.iter().any(|method| method.name == "Stop"));
    }

    #[test]
    fn optional_real_sdk_generation() {
        let (Ok(act_path), Ok(common_path)) = (
            std::env::var("ACT_BRIDGE_ACT_EXE"),
            std::env::var("ACT_BRIDGE_COMMON_DLL"),
        ) else {
            return;
        };
        let act = std::fs::read(act_path).unwrap();
        let common = std::fs::read(common_path).unwrap();
        let bytes = generate(
            &act,
            &common,
            &PluginConfig {
                assembly_name: "ActBridge.Integration".into(),
                native_dll_name: "act_bridge_integration.dll".into(),
            },
        )
        .unwrap();
        Resolution::parse(&bytes, ReadOptions::default()).unwrap();
    }
}
