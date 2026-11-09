//! One error enum with stable machine codes and exit-code mapping.

use std::path::PathBuf;

/// Process exit codes (stable contract, see behavior contract §1.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    General = 1,
    Usage = 2,
    Config = 3,
    NoFiles = 4,
    Partial = 5,
    Provider = 10,
    Auth = 11,
}

impl ExitCode {
    pub fn code(self) -> i32 {
        self as i32
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    Usage {
        message: String,
        hint: Option<String>,
    },
    #[error("{message}")]
    Config {
        message: String,
        hint: Option<String>,
    },
    #[error("{message}")]
    NoFiles {
        message: String,
        hint: Option<String>,
    },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{message}")]
    Conflict { message: String },
    #[error("{message}")]
    Provider {
        message: String,
        retryable: bool,
        hint: Option<String>,
    },
    #[error("{message}")]
    Auth { message: String },
    /// Text extraction / OCR failures with their own stable code
    /// (`pdfium_missing`, `ocr_models_missing`, `ocr_unavailable`, `ocr_failed`, …).
    #[error("{message}")]
    Extract {
        code: &'static str,
        message: String,
        hint: Option<String>,
    },
    /// An operation refused up-front (nothing changed), e.g. `undo_blocked`.
    #[error("{message}")]
    Blocked {
        message: String,
        hint: Option<String>,
    },
    #[error("{0}")]
    General(String),
}

impl Error {
    pub fn config(message: impl Into<String>) -> Self {
        Error::Config {
            message: message.into(),
            hint: None,
        }
    }

    pub fn config_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Error::Config {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Stable machine-readable code (`{"error":{"code":…}}`).
    pub fn code(&self) -> &'static str {
        match self {
            Error::Usage { .. } => "usage_error",
            Error::Config { .. } => "config_error",
            Error::NoFiles { .. } => "no_files",
            Error::Io { .. } => "io_error",
            Error::Conflict { .. } => "conflict",
            Error::Provider { .. } => "provider_error",
            Error::Auth { .. } => "auth_error",
            Error::Extract { code, .. } => code,
            Error::Blocked { .. } => "undo_blocked",
            Error::General(_) => "general_error",
        }
    }

    pub fn hint(&self) -> Option<&str> {
        match self {
            Error::Usage { hint, .. }
            | Error::Config { hint, .. }
            | Error::NoFiles { hint, .. }
            | Error::Extract { hint, .. }
            | Error::Provider { hint, .. }
            | Error::Blocked { hint, .. } => hint.as_deref(),
            Error::Auth { .. } => Some("Check ai.api_key (or the ${VAR} it references)."),
            _ => None,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Error::Provider {
                retryable: true,
                ..
            }
        )
    }

    /// Exit code when this error terminates the whole command.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Error::Usage { .. } => ExitCode::Usage,
            Error::Config { .. } => ExitCode::Config,
            Error::NoFiles { .. } => ExitCode::NoFiles,
            Error::Provider { .. } => ExitCode::Provider,
            Error::Auth { .. } => ExitCode::Auth,
            Error::Io { .. }
            | Error::Conflict { .. }
            | Error::Extract { .. }
            | Error::Blocked { .. }
            | Error::General(_) => ExitCode::General,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_exit_codes_are_stable() {
        let table: Vec<(Error, &str, i32)> = vec![
            (
                Error::Usage {
                    message: "u".into(),
                    hint: None,
                },
                "usage_error",
                2,
            ),
            (Error::config("c"), "config_error", 3),
            (
                Error::NoFiles {
                    message: "n".into(),
                    hint: None,
                },
                "no_files",
                4,
            ),
            (
                Error::io("/x", std::io::Error::other("boom")),
                "io_error",
                1,
            ),
            (
                Error::Conflict {
                    message: "x".into(),
                },
                "conflict",
                1,
            ),
            (
                Error::Provider {
                    message: "p".into(),
                    retryable: true,
                    hint: None,
                },
                "provider_error",
                10,
            ),
            (
                Error::Auth {
                    message: "a".into(),
                },
                "auth_error",
                11,
            ),
            (Error::General("g".into()), "general_error", 1),
        ];
        for (err, code, exit) in table {
            assert_eq!(err.code(), code);
            assert_eq!(err.exit_code().code(), exit, "{code}");
        }
        assert_eq!(ExitCode::Partial.code(), 5);
        assert!(
            Error::Provider {
                message: "p".into(),
                retryable: true,
                hint: None,
            }
            .retryable()
        );
        assert!(!Error::config("c").retryable());
    }
}
