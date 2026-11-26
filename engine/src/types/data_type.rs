use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "UPPERCASE")]
#[ts(export)]
pub enum ScalarType {
    Pdf,
    Md,
    Json,
    Csv,
    Jpg,
    Mp3,
}

impl ScalarType {
    /// File extension without the leading dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Md => "md",
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Jpg => "jpg",
            Self::Mp3 => "mp3",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Md => "MD",
            Self::Json => "JSON",
            Self::Csv => "CSV",
            Self::Jpg => "JPG",
            Self::Mp3 => "MP3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataType {
    pub scalar: ScalarType,
    pub list: bool,
}

impl DataType {
    pub fn single(scalar: ScalarType) -> Self {
        Self {
            scalar,
            list: false,
        }
    }

    pub fn list_of(scalar: ScalarType) -> Self {
        Self { scalar, list: true }
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.list {
            write!(f, "LIST[{}]", self.scalar.as_str())
        } else {
            f.write_str(self.scalar.as_str())
        }
    }
}
