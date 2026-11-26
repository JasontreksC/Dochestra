//! Environment-based configuration (CONTEXT §6.1).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_EXTRACTOR_URL: &str = "http://extractor:8000";
const DEFAULT_DATA_DIR: &str = "/data";
const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_HEALTH_TIMEOUT_SECS: u64 = 600;
const DEFAULT_EXTRACTOR_TIMEOUT_SECS: u64 = 900;
const DEFAULT_LLM_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_LLM_MODEL: &str = "claude-opus-5-5";
const DEFAULT_LLM_EFFORT: &str = "high";
const LLM_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const DEFAULT_EMBEDDING_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";
const DEFAULT_TTS_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_TTS_MODEL: &str = "tts-1";
const DEFAULT_TTS_VOICE: &str = "alloy";
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
    /// Anthropic API key (`LLM_API_KEY`); `None` makes LLM nodes fail, the engine still starts.
    pub llm_api_key: Option<String>,
    pub llm_base_url: String,
    pub llm_model: String,
    /// `low | medium | high | xhigh | max` (`LLM_EFFORT`).
    pub llm_effort: String,
    /// OpenAI API key for embeddings (`EMBEDDING_API_KEY`); `None` makes the rag node fail.
    pub embedding_api_key: Option<String>,
    pub embedding_base_url: String,
    pub embedding_model: String,
    /// OpenAI API key for text-to-speech (`TTS_API_KEY`); `None` makes the voice node fail.
    pub tts_api_key: Option<String>,
    pub tts_base_url: String,
    pub tts_model: String,
    pub tts_voice: String,
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
                "llm_api_key",
                &self.llm_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("llm_base_url", &self.llm_base_url)
            .field("llm_model", &self.llm_model)
            .field("llm_effort", &self.llm_effort)
            .field(
                "embedding_api_key",
                &self.embedding_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("embedding_base_url", &self.embedding_base_url)
            .field("embedding_model", &self.embedding_model)
            .field(
                "tts_api_key",
                &self.tts_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("tts_base_url", &self.tts_base_url)
            .field("tts_model", &self.tts_model)
            .field("tts_voice", &self.tts_voice)
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

        let llm_api_key = get("LLM_API_KEY").map(|k| k.trim().to_owned());
        let llm_base_url = get("LLM_BASE_URL")
            .unwrap_or_else(|| DEFAULT_LLM_BASE_URL.to_owned())
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

        let embedding_api_key = get("EMBEDDING_API_KEY").map(|k| k.trim().to_owned());
        let embedding_base_url = get("EMBEDDING_BASE_URL")
            .unwrap_or_else(|| DEFAULT_EMBEDDING_BASE_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let embedding_model =
            get("EMBEDDING_MODEL").unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_owned());

        let tts_api_key = get("TTS_API_KEY").map(|k| k.trim().to_owned());
        let tts_base_url = get("TTS_BASE_URL")
            .unwrap_or_else(|| DEFAULT_TTS_BASE_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let tts_model = get("TTS_MODEL").unwrap_or_else(|| DEFAULT_TTS_MODEL.to_owned());
        let tts_voice = get("TTS_VOICE").unwrap_or_else(|| DEFAULT_TTS_VOICE.to_owned());

        Ok(Self {
            database_url,
            extractor_url,
            data_dir,
            bind,
            extractor_health_required,
            extractor_health_timeout: Duration::from_secs(timeout_secs),
            extractor_timeout: Duration::from_secs(extractor_timeout_secs),
            upload_limit_bytes,
            llm_api_key,
            llm_base_url,
            llm_model,
            llm_effort,
            embedding_api_key,
            embedding_base_url,
            embedding_model,
            tts_api_key,
            tts_base_url,
            tts_model,
            tts_voice,
        })
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
        assert_eq!(cfg.llm_api_key, None);
        assert_eq!(cfg.llm_base_url, "https://api.anthropic.com");
        assert_eq!(cfg.llm_model, "claude-opus-5-5");
        assert_eq!(cfg.llm_effort, "high");
        assert_eq!(cfg.embedding_api_key, None);
        assert_eq!(cfg.embedding_base_url, "https://api.openai.com");
        assert_eq!(cfg.embedding_model, "text-embedding-3-small");
        assert_eq!(cfg.tts_api_key, None);
        assert_eq!(cfg.tts_base_url, "https://api.openai.com");
        assert_eq!(cfg.tts_model, "tts-1");
        assert_eq!(cfg.tts_voice, "alloy");
    }

    #[test]
    fn tts_settings_are_parsed() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("TTS_API_KEY", " sk-tts "),
            ("TTS_BASE_URL", "http://localhost:4321/"),
            ("TTS_MODEL", "gpt-4o-mini-tts"),
            ("TTS_VOICE", "nova"),
        ]))
        .unwrap();
        assert_eq!(cfg.tts_api_key.as_deref(), Some("sk-tts"));
        assert_eq!(cfg.tts_base_url, "http://localhost:4321");
        assert_eq!(cfg.tts_model, "gpt-4o-mini-tts");
        assert_eq!(cfg.tts_voice, "nova");
        assert!(!format!("{cfg:?}").contains("sk-tts"));
    }

    #[test]
    fn embedding_settings_are_parsed() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("EMBEDDING_API_KEY", " sk-emb "),
            ("EMBEDDING_BASE_URL", "http://localhost:1234/"),
            ("EMBEDDING_MODEL", "text-embedding-3-large"),
        ]))
        .unwrap();
        assert_eq!(cfg.embedding_api_key.as_deref(), Some("sk-emb"));
        assert_eq!(cfg.embedding_base_url, "http://localhost:1234");
        assert_eq!(cfg.embedding_model, "text-embedding-3-large");
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("sk-emb"), "{shown}");
    }

    #[test]
    fn llm_settings_are_parsed_and_validated() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("LLM_API_KEY", " sk-test "),
            ("LLM_BASE_URL", "http://localhost:9999/"),
            ("LLM_MODEL", "claude-sonnet-5-5"),
            ("LLM_EFFORT", "XHigh"),
        ]))
        .unwrap();
        assert_eq!(cfg.llm_api_key.as_deref(), Some("sk-test"));
        assert_eq!(cfg.llm_base_url, "http://localhost:9999");
        assert_eq!(cfg.llm_model, "claude-sonnet-5-5");
        assert_eq!(cfg.llm_effort, "xhigh");

        // Empty values mean "unset".
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("LLM_API_KEY", ""),
            ("LLM_MODEL", ""),
            ("LLM_EFFORT", "  "),
        ]))
        .unwrap();
        assert_eq!(cfg.llm_api_key, None);
        assert_eq!(cfg.llm_model, "claude-opus-5-5");
        assert_eq!(cfg.llm_effort, "high");

        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("LLM_EFFORT", "turbo"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: "LLM_EFFORT",
                ..
            }
        ));
    }

    #[test]
    fn debug_output_never_contains_secrets() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://user:hunter2@db/x"),
            ("LLM_API_KEY", "sk-very-secret"),
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
