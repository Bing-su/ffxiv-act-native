#[derive(Debug, thiserror::Error)]
#[error("malformed ABI v1 payload")]
pub struct DecodeError;

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
