//! Environment-based configuration (CONTEXT §6.1).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_EXTRACTOR_URL: &str = "http://extractor:8000";
const DEFAULT_DATA_DIR: &str = "/data";
const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_HEALTH_TIMEOUT_SECS: u64 = 600;
const DEFAULT_EXTRACTOR_TIMEOUT_SECS: u64 = 900;
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_LLM_MODEL: &str = "meta/muse-ember";
const DEFAULT_LLM_EFFORT: &str = "medium";
const LLM_EFFORTS: [&str; 3] = ["low", "medium", "high"];
const DEFAULT_EMBEDDING_MODEL: &str = "baai/bge-m3";
const DEFAULT_TTS_MODEL: &str = "google/gemini-3.8-flash-lite-tts";
const DEFAULT_TTS_VOICE: &str = "Kore";
/// Where the container image keeps the slide fonts.
const CONTAINER_FONT_DIR: &str = "/usr/local/share/dochestra/fonts";
const DEFAULT_UPLOAD_LIMIT_BYTES: usize = 200 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("invalid value {value:?} for {key}: {reason}")]
    Invalid {
        key: &'static str,
        value: String,
        reason: String,
    },
}

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub extractor_url: String,
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    pub extractor_health_required: bool,
    pub extractor_health_timeout: Duration,
    /// Per-request timeout of `POST /extract` (`EXTRACTOR_TIMEOUT_SECS`).
    pub extractor_timeout: Duration,
    /// Maximum request size of a bundle upload (`UPLOAD_LIMIT_BYTES`).
    pub upload_limit_bytes: usize,
    /// OpenRouter API key (`OPENROUTER_API_KEY`), shared by the LLM, embedding and TTS calls.
    /// `None` makes those nodes fail; the engine still starts.
    pub openrouter_api_key: Option<String>,
    /// OpenRouter base URL including the version path (`OPENROUTER_BASE_URL`).
    pub openrouter_base_url: String,
    pub llm_model: String,
    /// `low | medium | high` (`LLM_EFFORT`).
    pub llm_effort: String,
    pub embedding_model: String,
    pub tts_model: String,
    pub tts_voice: String,
    /// Directory with the slide fonts (`SLIDE_FONT_DIR`).
    pub slide_font_dir: PathBuf,
}

// Manual `Debug` so that logging a `Config` can never reveal the API key.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &"<redacted>")
            .field("extractor_url", &self.extractor_url)
            .field("data_dir", &self.data_dir)
            .field("bind", &self.bind)
            .field("extractor_health_required", &self.extractor_health_required)
            .field("extractor_health_timeout", &self.extractor_health_timeout)
            .field("extractor_timeout", &self.extractor_timeout)
            .field("upload_limit_bytes", &self.upload_limit_bytes)
            .field(
                "openrouter_api_key",
                &self.openrouter_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("openrouter_base_url", &self.openrouter_base_url)
            .field("llm_model", &self.llm_model)
            .field("llm_effort", &self.llm_effort)
            .field("embedding_model", &self.embedding_model)
            .field("tts_model", &self.tts_model)
            .field("tts_voice", &self.tts_voice)
            .field("slide_font_dir", &self.slide_font_dir)
            .finish()
    }
}

impl Config {
    /// Loads the configuration from process environment variables.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Loads the configuration through an arbitrary key lookup (testable).
    pub fn from_lookup<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let get = |key: &str| lookup(key).filter(|v| !v.trim().is_empty());

        let database_url = get("DATABASE_URL").ok_or(ConfigError::Missing("DATABASE_URL"))?;
        let extractor_url = get("EXTRACTOR_URL")
            .unwrap_or_else(|| DEFAULT_EXTRACTOR_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let data_dir =
            PathBuf::from(get("DATA_DIR").unwrap_or_else(|| DEFAULT_DATA_DIR.to_owned()));

        let bind_raw = get("ENGINE_BIND").unwrap_or_else(|| DEFAULT_BIND.to_owned());
        let bind = bind_raw
            .parse::<SocketAddr>()
            .map_err(|e| ConfigError::Invalid {
                key: "ENGINE_BIND",
                value: bind_raw.clone(),
                reason: e.to_string(),
            })?;

        let extractor_health_required = match get("EXTRACTOR_HEALTH_REQUIRED") {
            None => true,
            Some(raw) => parse_bool(&raw).ok_or_else(|| ConfigError::Invalid {
                key: "EXTRACTOR_HEALTH_REQUIRED",
                value: raw.clone(),
                reason: "expected true/false".to_owned(),
            })?,
        };

        let timeout_secs = match get("EXTRACTOR_HEALTH_TIMEOUT_SECS") {
            None => DEFAULT_HEALTH_TIMEOUT_SECS,
            Some(raw) => raw
                .trim()
                .parse::<u64>()
                .map_err(|e| ConfigError::Invalid {
                    key: "EXTRACTOR_HEALTH_TIMEOUT_SECS",
                    value: raw.clone(),
                    reason: e.to_string(),
                })?,
        };

        let extractor_timeout_secs = match get("EXTRACTOR_TIMEOUT_SECS") {
            None => DEFAULT_EXTRACTOR_TIMEOUT_SECS,
            Some(raw) => raw
                .trim()
                .parse::<u64>()
                .map_err(|e| ConfigError::Invalid {
                    key: "EXTRACTOR_TIMEOUT_SECS",
                    value: raw.clone(),
                    reason: e.to_string(),
                })?,
        };

        let upload_limit_bytes = match get("UPLOAD_LIMIT_BYTES") {
            None => DEFAULT_UPLOAD_LIMIT_BYTES,
            Some(raw) => raw
                .trim()
                .parse::<usize>()
                .map_err(|e| ConfigError::Invalid {
                    key: "UPLOAD_LIMIT_BYTES",
                    value: raw.clone(),
                    reason: e.to_string(),
                })?,
        };

        let openrouter_api_key = get("OPENROUTER_API_KEY").map(|k| k.trim().to_owned());
        let openrouter_base_url = get("OPENROUTER_BASE_URL")
            .unwrap_or_else(|| DEFAULT_OPENROUTER_BASE_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let llm_model = get("LLM_MODEL").unwrap_or_else(|| DEFAULT_LLM_MODEL.to_owned());
        let llm_effort = match get("LLM_EFFORT") {
            None => DEFAULT_LLM_EFFORT.to_owned(),
            Some(raw) => {
                let effort = raw.trim().to_ascii_lowercase();
                if !LLM_EFFORTS.contains(&effort.as_str()) {
                    return Err(ConfigError::Invalid {
                        key: "LLM_EFFORT",
                        value: raw,
                        reason: format!("expected one of {}", LLM_EFFORTS.join("|")),
                    });
                }
                effort
            }
        };
        let embedding_model =
            get("EMBEDDING_MODEL").unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_owned());
        let tts_model = get("TTS_MODEL").unwrap_or_else(|| DEFAULT_TTS_MODEL.to_owned());
        let tts_voice = get("TTS_VOICE").unwrap_or_else(|| DEFAULT_TTS_VOICE.to_owned());

        let slide_font_dir = get("SLIDE_FONT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(default_font_dir);

        Ok(Self {
            database_url,
            extractor_url,
            data_dir,
            bind,
            extractor_health_required,
            extractor_health_timeout: Duration::from_secs(timeout_secs),
            extractor_timeout: Duration::from_secs(extractor_timeout_secs),
            upload_limit_bytes,
            openrouter_api_key,
            openrouter_base_url,
            llm_model,
            llm_effort,
            embedding_model,
            tts_model,
            tts_voice,
            slide_font_dir,
        })
    }
}

/// The container's font directory when it exists, else the repository's `assets/fonts` (local
/// runs and tests).
fn default_font_dir() -> PathBuf {
    let container = PathBuf::from(CONTAINER_FONT_DIR);
    if container.is_dir() {
        container
    } else {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts"))
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn defaults_are_applied() {
        let cfg = Config::from_lookup(lookup(&[("DATABASE_URL", "postgres://x")])).unwrap();
        assert_eq!(cfg.database_url, "postgres://x");
        assert_eq!(cfg.extractor_url, "http://extractor:8000");
        assert_eq!(cfg.data_dir, PathBuf::from("/data"));
        assert_eq!(cfg.bind, "0.0.0.0:8080".parse::<SocketAddr>().unwrap());
        assert!(cfg.extractor_health_required);
        assert_eq!(cfg.extractor_health_timeout, Duration::from_secs(600));
        assert_eq!(cfg.upload_limit_bytes, 200 * 1024 * 1024);
        assert_eq!(cfg.extractor_timeout, Duration::from_secs(900));
        assert_eq!(cfg.openrouter_api_key, None);
        assert_eq!(cfg.openrouter_base_url, "https://openrouter.ai/api/v1");
        assert_eq!(cfg.llm_model, "meta/muse-ember");
        assert_eq!(cfg.llm_effort, "medium");
        assert_eq!(cfg.embedding_model, "baai/bge-m3");
        assert_eq!(cfg.tts_model, "google/gemini-3.8-flash-lite-tts");
        assert_eq!(cfg.tts_voice, "Kore");
    }

    #[test]
    fn openrouter_settings_are_parsed_and_validated() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("OPENROUTER_API_KEY", " sk-or-test "),
            ("OPENROUTER_BASE_URL", "http://localhost:9999/v1/"),
            ("LLM_MODEL", "some/model"),
            ("LLM_EFFORT", "HIGH"),
            ("EMBEDDING_MODEL", "some/embedder"),
            ("TTS_MODEL", "some/tts"),
            ("TTS_VOICE", "Puck"),
        ]))
        .unwrap();
        assert_eq!(cfg.openrouter_api_key.as_deref(), Some("sk-or-test"));
        assert_eq!(cfg.openrouter_base_url, "http://localhost:9999/v1");
        assert_eq!(cfg.llm_model, "some/model");
        assert_eq!(cfg.llm_effort, "high");
        assert_eq!(cfg.embedding_model, "some/embedder");
        assert_eq!(
            (cfg.tts_model.as_str(), cfg.tts_voice.as_str()),
            ("some/tts", "Puck")
        );

        // Empty values mean "unset".
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("OPENROUTER_API_KEY", ""),
            ("LLM_MODEL", ""),
            ("LLM_EFFORT", "  "),
        ]))
        .unwrap();
        assert_eq!(cfg.openrouter_api_key, None);
        assert_eq!(cfg.llm_model, "meta/muse-ember");
        assert_eq!(cfg.llm_effort, "medium");

        // Only low | medium | high are accepted.
        for bad in ["turbo", "xhigh", "max"] {
            let err = Config::from_lookup(lookup(&[
                ("DATABASE_URL", "postgres://x"),
                ("LLM_EFFORT", bad),
            ]))
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::Invalid {
                        key: "LLM_EFFORT",
                        ..
                    }
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_retired_provider_keys_are_ignored() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("LLM_API_KEY", "old"),
            ("LLM_BASE_URL", "http://old"),
            ("EMBEDDING_API_KEY", "old"),
            ("TTS_API_KEY", "old"),
        ]))
        .unwrap();
        assert_eq!(cfg.openrouter_api_key, None);
        assert_eq!(cfg.openrouter_base_url, "https://openrouter.ai/api/v1");
    }

    #[test]
    fn slide_font_dir_defaults_and_overrides() {
        let cfg = Config::from_lookup(lookup(&[("DATABASE_URL", "postgres://x")])).unwrap();
        assert!(
            cfg.slide_font_dir.join("NanumGothic-Regular.ttf").is_file(),
            "{:?}",
            cfg.slide_font_dir
        );
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("SLIDE_FONT_DIR", "/custom/fonts"),
        ]))
        .unwrap();
        assert_eq!(cfg.slide_font_dir, PathBuf::from("/custom/fonts"));
    }

    #[test]
    fn debug_output_never_contains_secrets() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://user:hunter2@db/x"),
            ("OPENROUTER_API_KEY", "sk-very-secret"),
        ]))
        .unwrap();
        let shown = format!("{cfg:?}");
        assert!(
            !shown.contains("sk-very-secret") && !shown.contains("hunter2"),
            "{shown}"
        );
    }

    #[test]
    fn extractor_timeout_is_configurable_and_validated() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("EXTRACTOR_TIMEOUT_SECS", "30"),
        ]))
        .unwrap();
        assert_eq!(cfg.extractor_timeout, Duration::from_secs(30));
        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("EXTRACTOR_TIMEOUT_SECS", "soon"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: "EXTRACTOR_TIMEOUT_SECS",
                ..
            }
        ));
    }

    #[test]
    fn upload_limit_is_configurable_and_validated() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("UPLOAD_LIMIT_BYTES", "1024"),
        ]))
        .unwrap();
        assert_eq!(cfg.upload_limit_bytes, 1024);
        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("UPLOAD_LIMIT_BYTES", "-5"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: "UPLOAD_LIMIT_BYTES",
                ..
            }
        ));
    }

    #[test]
    fn overrides_are_parsed() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("EXTRACTOR_URL", "http://localhost:9000/"),
            ("DATA_DIR", "/tmp/d"),
            ("ENGINE_BIND", "127.0.0.1:9090"),
            ("EXTRACTOR_HEALTH_REQUIRED", "false"),
            ("EXTRACTOR_HEALTH_TIMEOUT_SECS", "15"),
        ]))
        .unwrap();
        assert_eq!(cfg.extractor_url, "http://localhost:9000");
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/d"));
        assert_eq!(cfg.bind.port(), 9090);
        assert!(!cfg.extractor_health_required);
        assert_eq!(cfg.extractor_health_timeout, Duration::from_secs(15));
    }

    #[test]
    fn missing_database_url_is_error() {
        let err = Config::from_lookup(lookup(&[])).unwrap_err();
        assert_eq!(err, ConfigError::Missing("DATABASE_URL"));
        let err = Config::from_lookup(lookup(&[("DATABASE_URL", "  ")])).unwrap_err();
        assert_eq!(err, ConfigError::Missing("DATABASE_URL"));
    }

    #[test]
    fn invalid_bool_is_error() {
        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("EXTRACTOR_HEALTH_REQUIRED", "maybe"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: "EXTRACTOR_HEALTH_REQUIRED",
                ..
            }
        ));
    }

    #[test]
    fn invalid_number_and_bind_are_errors() {
        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("EXTRACTOR_HEALTH_TIMEOUT_SECS", "abc"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: "EXTRACTOR_HEALTH_TIMEOUT_SECS",
                ..
            }
        ));

        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("ENGINE_BIND", "not-an-addr"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: "ENGINE_BIND",
                ..
            }
        ));
    }
}
