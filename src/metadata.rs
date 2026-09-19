use dotscope::{
    CilAssembly, Error,
    metadata::{
        customattributes::{
            CustomAttributeArgument, CustomAttributeValue, encode_custom_attribute_value,
        },
        tables::{CustomAttributeRaw, MemberRefRaw, TableDataOwned, TableId, TypeRefRaw},
        typesystem::CilTypeReference,
    },
};
use editpe::{Image, types::VersionU32};

use crate::GenerateError;

const MAX_STRING_LEN: usize = 2048;

struct MetadataValue<'a> {
    name: &'static str,
    value: &'a str,
}

struct AttributeUpdate {
    attribute: CustomAttributeRaw,
    value: String,
}

impl<'a> MetadataValue<'a> {
    const fn new(name: &'static str, value: &'a str) -> Self {
        Self { name, value }
    }
}

/// Windows file metadata for the generated managed shim.
///
/// String values are limited to 2048 UTF-16 code units.
/// Keeping assembly and file versions separate allows .NET binding identity to
/// remain stable while the distributable file receives independent releases.
///
/// # Example
///
/// ```
/// use ffxiv_act_native::PluginMetadata;
///
/// let metadata = PluginMetadata {
///     assembly_version: [1, 0, 0, 0],
///     file_version: [1, 2, 3, 0],
///     product_version: "1.2.3".into(),
///     product_name: "Example ACT Plugin".into(),
///     ..PluginMetadata::default()
/// };
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginMetadata {
    /// Four-part version used by the .NET assembly identity.
    pub assembly_version: [u16; 4],
    /// Four-part numeric version shown in Windows file properties.
    pub file_version: [u16; 4],
    /// Display version, which may include a suffix such as `1.2.3-beta.1`.
    pub product_version: String,
    /// Short description shown in Windows file properties.
    pub file_description: String,
    /// Product family shown in Windows file properties.
    pub product_name: String,
    /// Publisher shown in Windows file properties.
    pub company_name: String,
    /// Copyright notice shown in Windows file properties.
    pub legal_copyright: String,
    /// Optional release or build note shown in Windows file properties.
    pub comments: String,
}

impl Default for PluginMetadata {
    fn default() -> Self {
        Self {
            assembly_version: [1, 0, 0, 0],
            file_version: [1, 0, 0, 0],
            product_version: "1.0.0".into(),
            file_description: String::new(),
            product_name: String::new(),
            company_name: String::new(),
            legal_copyright: String::new(),
            comments: String::new(),
        }
    }
}

pub(crate) fn validate(
    metadata: &PluginMetadata,
    assembly_name: &str,
) -> Result<(), GenerateError> {
    let original_filename = format!("{assembly_name}.dll");
    for value in [
        assembly_name,
        original_filename.as_str(),
        metadata.product_version.as_str(),
        metadata.file_description.as_str(),
        metadata.product_name.as_str(),
        metadata.company_name.as_str(),
        metadata.legal_copyright.as_str(),
        metadata.comments.as_str(),
    ] {
        if value.contains('\0') {
            return Err(GenerateError::InvalidConfig(
                "metadata strings must not contain NUL characters",
            ));
        }
        if value.encode_utf16().count() > MAX_STRING_LEN {
            return Err(GenerateError::InvalidConfig(
                "metadata strings must not exceed 2048 UTF-16 code units",
            ));
        }
    }
    parse_product_version(&metadata.product_version)?;
    Ok(())
}

pub(crate) fn apply_version_resource(
    mut output: Vec<u8>,
    assembly_name: &str,
    metadata: &PluginMetadata,
) -> Result<Vec<u8>, GenerateError> {
    let image = Image::parse(output.as_slice())
        .map_err(|error| GenerateError::Write(format!("{error:?}")))?;
    let mut version = image
        .resource_directory()
        .ok_or_else(|| GenerateError::Write("shim template has no resources".into()))?
        .get_version_info()
        .map_err(|error| GenerateError::Write(format!("{error:?}")))?
        .ok_or_else(|| GenerateError::Write("shim template has no version info".into()))?;
    version.info.file_version = version_u32(metadata.file_version);
    version.info.product_version = version_u32(parse_product_version(&metadata.product_version)?);
    let file_version = format_version(metadata.file_version);
    let original_filename = format!("{assembly_name}.dll");
    let strings = &mut version
        .strings
        .first_mut()
        .ok_or_else(|| GenerateError::Write("shim template has no version string table".into()))?
        .strings;
    for entry in [
        MetadataValue::new("Comments", &metadata.comments),
        MetadataValue::new("CompanyName", &metadata.company_name),
        MetadataValue::new("FileDescription", &metadata.file_description),
        MetadataValue::new("FileVersion", &file_version),
        MetadataValue::new("InternalName", assembly_name),
        MetadataValue::new("LegalCopyright", &metadata.legal_copyright),
        MetadataValue::new("OriginalFilename", &original_filename),
        MetadataValue::new("ProductName", &metadata.product_name),
        MetadataValue::new("ProductVersion", &metadata.product_version),
    ] {
        strings.insert(entry.name.into(), entry.value.into());
    }
    strings.shift_remove("LegalTrademarks");
    let replacement = version
        .try_build()
        .map_err(|error| GenerateError::Write(format!("{error:?}")))?;

    // Fixed template capacity avoids PE section rewriting; enlarge the placeholder if needed.
    let marker: Vec<_> = "VS_VERSION_INFO"
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let offset = output
        .windows(marker.len())
        .position(|bytes| bytes == marker)
        .and_then(|offset| offset.checked_sub(6))
        .ok_or_else(|| GenerateError::Write("could not locate shim version info".into()))?;
    let capacity = usize::from(u16::from_le_bytes([output[offset], output[offset + 1]]));
    if replacement.len() > capacity {
        return Err(GenerateError::InvalidConfig(
            "combined metadata exceeds the shim template capacity",
        ));
    }
    drop(image);
    output[offset..offset + capacity].fill(0);
    output[offset..offset + replacement.len()].copy_from_slice(&replacement);
    Ok(output)
}

pub(crate) fn apply_assembly_attributes(
    output: &mut CilAssembly,
    assembly_name: &str,
    metadata: &PluginMetadata,
) -> Result<(), GenerateError> {
    let file_version = format_version(metadata.file_version);
    let replacements = [
        MetadataValue::new("AssemblyTitleAttribute", assembly_name),
        MetadataValue::new("AssemblyDescriptionAttribute", &metadata.file_description),
        MetadataValue::new("AssemblyCompanyAttribute", &metadata.company_name),
        MetadataValue::new("AssemblyProductAttribute", &metadata.product_name),
        MetadataValue::new("AssemblyCopyrightAttribute", &metadata.legal_copyright),
        MetadataValue::new("AssemblyTrademarkAttribute", ""),
        MetadataValue::new("AssemblyFileVersionAttribute", &file_version),
        MetadataValue::new(
            "AssemblyInformationalVersionAttribute",
            &metadata.product_version,
        ),
    ];
    let view = output.view();
    let tables = view
        .tables()
        .ok_or_else(|| GenerateError::Write("shim template has no metadata tables".into()))?;
    let strings = view
        .strings()
        .ok_or_else(|| GenerateError::Write("shim template has no string heap".into()))?;
    let member_refs: Vec<_> = tables
        .table::<MemberRefRaw>()
        .into_iter()
        .flat_map(|table| table.iter().filter_map(Result::ok))
        .collect();
    let type_refs: Vec<_> = tables
        .table::<TypeRefRaw>()
        .into_iter()
        .flat_map(|table| table.iter().filter_map(Result::ok))
        .collect();
    let attributes: Vec<_> = tables
        .table::<CustomAttributeRaw>()
        .into_iter()
        .flat_map(|table| table.iter().filter_map(Result::ok))
        .filter(|attribute| attribute.parent.tag == TableId::Assembly)
        .filter_map(|attribute| {
            let member = member_refs
                .iter()
                .find(|member| member.rid == attribute.constructor.row)?;
            let type_ref = type_refs
                .iter()
                .find(|type_ref| type_ref.rid == member.class.row)?;
            let name = strings.get(type_ref.type_name as usize).ok()?;
            let value = replacements
                .iter()
                .find(|replacement| replacement.name == name)?
                .value;
            Some(AttributeUpdate {
                attribute: attribute.clone(),
                value: value.to_owned(),
            })
        })
        .collect();
    if attributes.len() != replacements.len() {
        return Err(GenerateError::Write(
            "shim template is missing assembly metadata attributes".into(),
        ));
    }

    for AttributeUpdate {
        mut attribute,
        value,
    } in attributes
    {
        let blob = encode_custom_attribute_value(&CustomAttributeValue {
            fixed_args: vec![CustomAttributeArgument::String(value)],
            named_args: Vec::new(),
            constructor: CilTypeReference::None,
            blob_index: 0,
        })
        .map_err(write_error)?;
        attribute.value = output.blob_add(&blob).map_err(write_error)?.placeholder();
        output
            .table_row_update(
                TableId::CustomAttribute,
                attribute.rid,
                TableDataOwned::CustomAttribute(attribute),
            )
            .map_err(write_error)?;
    }
    Ok(())
}

fn version_u32([major, minor, build, revision]: [u16; 4]) -> VersionU32 {
    VersionU32 {
        major: (u32::from(major) << 16) | u32::from(minor),
        minor: (u32::from(build) << 16) | u32::from(revision),
    }
}

fn parse_product_version(value: &str) -> Result<[u16; 4], GenerateError> {
    let mut version = [0; 4];
    for (output, input) in version.iter_mut().zip(value.split('.')) {
        let digits: String = input.chars().take_while(char::is_ascii_digit).collect();
        *output = if digits.is_empty() {
            0
        } else {
            digits.parse().map_err(|_| {
                GenerateError::InvalidConfig("product_version components must not exceed 65535")
            })?
        };
    }
    Ok(version)
}

fn format_version(version: [u16; 4]) -> String {
    format!(
        "{}.{}.{}.{}",
        version[0], version[1], version[2], version[3]
    )
}

fn write_error(error: Error) -> GenerateError {
    GenerateError::Write(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_utf16_metadata() {
        let metadata = PluginMetadata {
            comments: "x".repeat(MAX_STRING_LEN + 1),
            ..Default::default()
        };

        assert!(validate(&metadata, "Example").is_err());
        assert!(validate(&PluginMetadata::default(), &"x".repeat(MAX_STRING_LEN)).is_err());
    }

    #[test]
    fn rejects_out_of_range_product_version() {
        let metadata = PluginMetadata {
            product_version: "65536.1.2".into(),
            ..Default::default()
        };

        assert!(validate(&metadata, "Example").is_err());
    }

    #[test]
    fn template_holds_maximum_metadata() {
        let value = "x".repeat(MAX_STRING_LEN);
        let metadata = PluginMetadata {
            product_version: value.clone(),
            file_description: value.clone(),
            product_name: value.clone(),
            company_name: value.clone(),
            legal_copyright: value.clone(),
            comments: value,
            ..Default::default()
        };

        validate(&metadata, "Example").unwrap();
        apply_version_resource(
            include_bytes!("../managed/Shim.template.dll").to_vec(),
            "Example",
            &metadata,
        )
        .unwrap();
    }
}
