use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use super::Credentials;

const SECRET_BYTES: usize = 32;
const SESSION_BYTES: usize = 16;
const TOKEN_CONTEXT: &[u8] = b"openpodium-ipc-capability-v1\0";

pub struct CapabilityIssuer {
    secret: [u8; SECRET_BYTES],
    session_id: String,
}

impl CapabilityIssuer {
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, AuthenticationError> {
        let path = path.as_ref();
        let secret = match create_secret(path) {
            Ok(secret) => secret,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => read_secret(path)?,
            Err(source) => {
                return Err(AuthenticationError::FileAccess {
                    operation: "create",
                    path: path.to_owned(),
                    source,
                });
            }
        };
        verify_permissions(path)?;

        let mut session = [0_u8; SESSION_BYTES];
        getrandom::fill(&mut session).map_err(AuthenticationError::Random)?;
        Ok(Self {
            secret,
            session_id: encode_hex(&session),
        })
    }

    pub fn credentials(&self, workspace_id: u64, agent_id: u64) -> Credentials {
        Credentials {
            workspace_id,
            agent_id,
            token: self.token(workspace_id, agent_id),
        }
    }

    pub fn authenticates(&self, credentials: &Credentials) -> bool {
        if credentials.workspace_id == 0 || credentials.agent_id == 0 {
            return false;
        }
        constant_time_eq(
            credentials.token.as_bytes(),
            self.token(credentials.workspace_id, credentials.agent_id)
                .as_bytes(),
        )
    }

    fn token(&self, workspace_id: u64, agent_id: u64) -> String {
        let mut hasher = blake3::Hasher::new_keyed(&self.secret);
        hasher.update(TOKEN_CONTEXT);
        hasher.update(self.session_id.as_bytes());
        hasher.update(&workspace_id.to_le_bytes());
        hasher.update(&agent_id.to_le_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

fn create_secret(path: &Path) -> io::Result<[u8; SECRET_BYTES]> {
    let mut secret = [0_u8; SECRET_BYTES];
    getrandom::fill(&mut secret).map_err(|error| io::Error::other(error.to_string()))?;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(encode_hex(&secret).as_bytes())?;
    file.sync_all()?;
    Ok(secret)
}

fn read_secret(path: &Path) -> Result<[u8; SECRET_BYTES], AuthenticationError> {
    let encoded = fs::read_to_string(path).map_err(|source| AuthenticationError::FileAccess {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;
    decode_secret(encoded.trim()).ok_or_else(|| AuthenticationError::InvalidSecret {
        path: path.to_owned(),
    })
}

#[cfg(unix)]
fn verify_permissions(path: &Path) -> Result<(), AuthenticationError> {
    let permissions = fs::metadata(path)
        .map_err(|source| AuthenticationError::FileAccess {
            operation: "inspect",
            path: path.to_owned(),
            source,
        })?
        .permissions()
        .mode();
    if permissions & 0o077 != 0 {
        return Err(AuthenticationError::InsecurePermissions {
            path: path.to_owned(),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_permissions(_path: &Path) -> Result<(), AuthenticationError> {
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_secret(encoded: &str) -> Option<[u8; SECRET_BYTES]> {
    if encoded.len() != SECRET_BYTES * 2 {
        return None;
    }
    let mut decoded = [0_u8; SECRET_BYTES];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (decode_nibble(pair[0])? << 4) | decode_nibble(pair[1])?;
    }
    Some(decoded)
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left = left.get(index).copied().unwrap_or(0);
        let right = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

#[derive(Debug)]
pub enum AuthenticationError {
    FileAccess {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidSecret {
        path: PathBuf,
    },
    InsecurePermissions {
        path: PathBuf,
    },
    Random(getrandom::Error),
}

impl Display for AuthenticationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileAccess {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} IPC secret {}: {source}",
                path.display()
            ),
            Self::InvalidSecret { path } => write!(
                formatter,
                "IPC secret {} is invalid; remove it while OpenPodium is stopped to regenerate it",
                path.display()
            ),
            Self::InsecurePermissions { path } => write!(
                formatter,
                "IPC secret {} is accessible by another local user; set its permissions to 0600",
                path.display()
            ),
            Self::Random(source) => {
                write!(formatter, "failed to obtain secure randomness: {source}")
            }
        }
    }
}

impl Error for AuthenticationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FileAccess { source, .. } => Some(source),
            Self::InvalidSecret { .. } | Self::InsecurePermissions { .. } | Self::Random(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn installation_secret_is_reused_but_session_credentials_rotate() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ipc-secret");
        let first = CapabilityIssuer::load_or_create(&path).unwrap();
        let first_credentials = first.credentials(4, 7);
        assert!(first.authenticates(&first_credentials));

        let second = CapabilityIssuer::load_or_create(&path).unwrap();
        let second_credentials = second.credentials(4, 7);
        assert_ne!(first_credentials.token, second_credentials.token);
        assert!(!second.authenticates(&first_credentials));
        assert!(second.authenticates(&second_credentials));
    }

    #[test]
    fn capability_is_bound_to_workspace_and_agent() {
        let temp = TempDir::new().unwrap();
        let issuer = CapabilityIssuer::load_or_create(temp.path().join("ipc-secret")).unwrap();
        let valid = issuer.credentials(4, 7);
        assert!(issuer.authenticates(&valid));

        let mut wrong_workspace = valid.clone();
        wrong_workspace.workspace_id = 5;
        assert!(!issuer.authenticates(&wrong_workspace));

        let mut wrong_agent = valid.clone();
        wrong_agent.agent_id = 8;
        assert!(!issuer.authenticates(&wrong_agent));

        let mut wrong_token = valid;
        let replacement = if wrong_token.token.starts_with('0') {
            "1"
        } else {
            "0"
        };
        wrong_token.token.replace_range(..1, replacement);
        assert!(!issuer.authenticates(&wrong_token));
    }

    #[cfg(unix)]
    #[test]
    fn secret_file_is_private_to_the_local_user() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ipc-secret");
        CapabilityIssuer::load_or_create(&path).unwrap();

        let mode = fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
