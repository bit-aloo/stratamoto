use stratum_core::{
    common_messages_sv2::{
        ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_FEATURE_FLAGS,
        ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_PROTOCOL, SetupConnection,
        SetupConnectionError, SetupConnectionSuccess,
    },
    parsers_sv2::{AnyMessage, CommonMessages},
};

use crate::{
    connection::Connection,
    error::Result,
    roles::{Role, RoleConfig},
};

pub const ERROR_CODE_PROTOCOL_VERSION_MISMATCH: &str = "protocol-version-mismatch";

/// An upstream that answers `SetupConnection` for a single subprotocol.
///
/// It serves every connection that reaches its address, replying to whichever peer each
/// frame arrived from.
pub struct MockUpstream {
    config: RoleConfig,
}

impl MockUpstream {
    #[must_use]
    pub fn new(config: RoleConfig) -> Self {
        Self { config }
    }

    /// The answer the common protocol requires for a given `SetupConnection`.
    #[must_use]
    pub fn respond(config: &RoleConfig, setup: &SetupConnection<'_>) -> AnyMessage<'static> {
        if setup.protocol != config.protocol {
            return setup_error(0, ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_PROTOCOL);
        }

        let Some(used_version) = setup.get_version(config.min_version, config.max_version) else {
            return setup_error(0, ERROR_CODE_PROTOCOL_VERSION_MISMATCH);
        };

        let unsupported = setup.flags & !config.supported_flags;
        if unsupported != 0 {
            return setup_error(
                unsupported,
                ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_FEATURE_FLAGS,
            );
        }

        AnyMessage::Common(CommonMessages::SetupConnectionSuccess(
            SetupConnectionSuccess {
                used_version,
                flags: config.supported_flags,
            },
        ))
    }
}

impl Role for MockUpstream {
    async fn run(self, mut connection: Connection) -> Result<()> {
        loop {
            let mut frame = connection.recv().await?;
            let response = match frame.message() {
                Ok(AnyMessage::Common(CommonMessages::SetupConnection(setup))) => {
                    Self::respond(&self.config, &setup)
                }
                // Anything else is either not decodable or not expected before setup. A real
                // upstream closes the connection here; staying silent lets the oracle see the
                // absence of a reply.
                _ => continue,
            };
            connection.send(response).await?;
        }
    }
}

fn setup_error(flags: u32, error_code: &str) -> AnyMessage<'static> {
    AnyMessage::Common(CommonMessages::SetupConnectionError(SetupConnectionError {
        flags,
        error_code: error_code
            .to_string()
            .try_into()
            .expect("error codes fit in Str0255"),
    }))
}
