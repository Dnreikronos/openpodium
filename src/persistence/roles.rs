use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::domain::{Content, Name, Role, RoleColor, RoleIcon, RoleId};

const ROLE_FORMAT: &str = "openpodium-role";
const ROLE_FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableRole {
    format: String,
    version: u32,
    role: PortableRoleBody,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableRoleBody {
    name: String,
    color: String,
    icon: String,
    instructions: String,
}

pub fn export_role(role: &Role) -> Result<String, RoleTransferError> {
    let document = PortableRole {
        format: ROLE_FORMAT.to_owned(),
        version: ROLE_FORMAT_VERSION,
        role: PortableRoleBody {
            name: role.name().as_str().to_owned(),
            color: role.color().as_str().to_owned(),
            icon: role.icon().as_str().to_owned(),
            instructions: role.instructions().as_str().to_owned(),
        },
    };
    serde_json::to_string_pretty(&document).map_err(RoleTransferError::InvalidJson)
}

pub fn import_role(payload: &str, id: RoleId) -> Result<Role, RoleTransferError> {
    let document: PortableRole =
        serde_json::from_str(payload).map_err(RoleTransferError::InvalidJson)?;
    if document.format != ROLE_FORMAT {
        return Err(RoleTransferError::UnsupportedFormat(document.format));
    }
    if document.version != ROLE_FORMAT_VERSION {
        return Err(RoleTransferError::UnsupportedVersion {
            found: document.version,
            supported: ROLE_FORMAT_VERSION,
        });
    }
    Ok(Role::with_appearance(
        id,
        Name::new(document.role.name)
            .map_err(|error| RoleTransferError::InvalidRole(error.to_string()))?,
        RoleColor::new(document.role.color)
            .map_err(|error| RoleTransferError::InvalidRole(error.to_string()))?,
        RoleIcon::new(document.role.icon)
            .map_err(|error| RoleTransferError::InvalidRole(error.to_string()))?,
        Content::new(document.role.instructions)
            .map_err(|error| RoleTransferError::InvalidRole(error.to_string()))?,
    ))
}

#[derive(Debug)]
pub enum RoleTransferError {
    InvalidJson(serde_json::Error),
    UnsupportedFormat(String),
    UnsupportedVersion { found: u32, supported: u32 },
    InvalidRole(String),
}

impl Display for RoleTransferError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid role JSON: {error}"),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "unsupported role format {format:?}")
            }
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "unsupported role format version {found}; this build supports {supported}"
            ),
            Self::InvalidRole(detail) => write!(formatter, "invalid role: {detail}"),
        }
    }
}

impl Error for RoleTransferError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::UnsupportedFormat(_) | Self::UnsupportedVersion { .. } | Self::InvalidRole(_) => {
                None
            }
        }
    }
}
