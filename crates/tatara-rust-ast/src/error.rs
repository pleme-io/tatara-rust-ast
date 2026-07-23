use thiserror::Error;

#[derive(Debug, Error)]
pub enum AstError {
    #[error("invalid identifier `{0}`: {1}")]
    InvalidIdent(String, String),

    #[error("syn parse failure: {0}")]
    SynParse(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml serialize: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("invalid spec: {0}")]
    InvalidSpec(String),
}
