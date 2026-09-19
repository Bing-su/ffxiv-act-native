/// Indicates that bytes received through ABI v1 do not match its wire format.
///
/// The error intentionally carries no offset because payloads cross an FFI
/// boundary and callers can only reject the whole event safely.
#[derive(Debug, thiserror::Error)]
#[error("malformed ABI v1 payload")]
pub struct DecodeError;

/// Describes why a managed shim could not be generated.
///
/// Variants separate invalid inputs from template-writing failures so build
/// scripts can present a useful cause without parsing error text.
#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    /// An input could not be parsed as a managed .NET assembly.
    #[error("{input} is not a valid .NET assembly: {message}")]
    InvalidAssembly {
        /// Human-readable name of the input contract.
        input: &'static str,
        /// Parser-provided failure detail.
        message: String,
    },
    /// A managed assembly lacks the manifest needed to bind its identity.
    #[error("{input} has no assembly manifest")]
    MissingAssemblyManifest { input: &'static str },
    /// A type required by the shim contract is absent.
    #[error("required type {0} was not found")]
    MissingType(&'static str),
    /// A required method, event, module, or string is absent.
    #[error("{type_name} is missing required {member_kind} {member_name}")]
    MissingMember {
        /// Type or template being inspected.
        type_name: &'static str,
        /// Kind of member that was expected.
        member_kind: &'static str,
        /// Name of the expected member.
        member_name: &'static str,
    },
    /// A required member exists but cannot be called using the shim ABI.
    #[error("{type_name}.{member_name} has an ABI-incompatible signature")]
    IncompatibleSignature {
        /// Type that owns the incompatible member.
        type_name: &'static str,
        /// Name of the incompatible member.
        member_name: &'static str,
    },
    /// Plugin configuration violates a constraint required for safe emission.
    #[error("invalid plugin configuration: {0}")]
    InvalidConfig(&'static str),
    /// The validated template could not be rewritten or serialized.
    #[error("could not write managed shim: {0}")]
    Write(String),
}
