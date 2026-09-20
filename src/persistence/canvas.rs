use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::domain::CanvasLayout;

use super::codec::CanvasLayoutV1;

const CANVAS_FORMAT: &str = "openpodium-canvas-fragment";
const CANVAS_FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableCanvas {
    format: String,
    version: u32,
    canvas: CanvasLayoutV1,
}

pub fn export_canvas_fragment(layout: &CanvasLayout) -> Result<String, CanvasTransferError> {
    serde_json::to_string_pretty(&PortableCanvas {
        format: CANVAS_FORMAT.to_owned(),
        version: CANVAS_FORMAT_VERSION,
        canvas: CanvasLayoutV1::from(layout),
    })
    .map_err(CanvasTransferError::InvalidJson)
}

pub fn import_canvas_fragment(payload: &str) -> Result<CanvasLayout, CanvasTransferError> {
    let document: PortableCanvas =
        serde_json::from_str(payload).map_err(CanvasTransferError::InvalidJson)?;
    if document.format != CANVAS_FORMAT {
        return Err(CanvasTransferError::UnsupportedFormat(document.format));
    }
    if document.version != CANVAS_FORMAT_VERSION {
        return Err(CanvasTransferError::UnsupportedVersion {
            found: document.version,
            supported: CANVAS_FORMAT_VERSION,
        });
    }
    document
        .canvas
        .into_domain()
        .map_err(CanvasTransferError::InvalidCanvas)
}

#[derive(Debug)]
pub enum CanvasTransferError {
    InvalidJson(serde_json::Error),
    UnsupportedFormat(String),
    UnsupportedVersion { found: u32, supported: u32 },
    InvalidCanvas(String),
}

impl Display for CanvasTransferError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid canvas fragment JSON: {error}"),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "unsupported canvas fragment format {format:?}")
            }
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "unsupported canvas fragment version {found}; this build supports {supported}"
            ),
            Self::InvalidCanvas(detail) => write!(formatter, "invalid canvas fragment: {detail}"),
        }
    }
}

impl Error for CanvasTransferError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::UnsupportedFormat(_)
            | Self::UnsupportedVersion { .. }
            | Self::InvalidCanvas(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        CanvasNodeContent, CanvasPoint, CanvasSize, CanvasText, Connection, ConnectionId,
        ConnectionKind, Node, NodeId,
    };

    use super::*;

    #[test]
    fn portable_fragments_preserve_owned_content_and_connections() {
        let first = Node::with_content(
            NodeId::new(7),
            CanvasNodeContent::Text {
                markdown: CanvasText::new("Context").unwrap(),
            },
            CanvasPoint::new(20.0, 40.0).unwrap(),
            CanvasSize::new(240.0, 160.0).unwrap(),
        );
        let second = Node::with_content(
            NodeId::new(8),
            CanvasNodeContent::Text {
                markdown: CanvasText::new("Decision").unwrap(),
            },
            CanvasPoint::new(300.0, 40.0).unwrap(),
            CanvasSize::new(240.0, 160.0).unwrap(),
        );
        let layout = CanvasLayout::new(
            vec![first, second],
            vec![],
            vec![Connection::new(
                ConnectionId::new(3),
                NodeId::new(7),
                NodeId::new(8),
                ConnectionKind::Reference,
            )],
        );

        let payload = export_canvas_fragment(&layout).unwrap();
        assert_eq!(import_canvas_fragment(&payload).unwrap(), layout);
    }
}
