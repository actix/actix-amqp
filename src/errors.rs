use bytestring::ByteString;
use either::Either;
use ntex_amqp_codec::{protocol, AmqpCodecError, ProtocolIdError};

#[derive(Debug, Display, Clone)]
pub enum AmqpTransportError {
    Codec(AmqpCodecError),
    TooManyChannels,
    Disconnected,
    Timeout,
    #[display(fmt = "Connection closed, error: {:?}", _0)]
    Closed(Option<protocol::Error>),
    #[display(fmt = "Session ended, error: {:?}", _0)]
    SessionEnded(Option<protocol::Error>),
    #[display(fmt = "Link detached, error: {:?}", _0)]
    LinkDetached(Option<protocol::Error>),
}

impl From<AmqpCodecError> for AmqpTransportError {
    fn from(err: AmqpCodecError) -> Self {
        AmqpTransportError::Codec(err)
    }
}

#[derive(Debug, Display, From)]
pub enum SaslConnectError {
    ProtocolId(ProtocolIdError),
    AmqpError(AmqpCodecError),
    #[display(fmt = "Sasl error code: {:?}", _0)]
    Sasl(protocol::SaslCode),
    ExpectedOpenFrame,
    Disconnected,
    Io(std::io::Error),
}

impl From<Either<AmqpCodecError, std::io::Error>> for SaslConnectError {
    fn from(err: Either<AmqpCodecError, std::io::Error>) -> Self {
        match err {
            Either::Left(err) => SaslConnectError::AmqpError(err),
            Either::Right(err) => SaslConnectError::Io(err),
        }
    }
}

impl From<ProtocolNegotiationError> for SaslConnectError {
    fn from(err: ProtocolNegotiationError) -> Self {
        match err {
            ProtocolNegotiationError::Disconnected => SaslConnectError::Disconnected,
            ProtocolNegotiationError::Protocol(err) => SaslConnectError::ProtocolId(err),
            ProtocolNegotiationError::Io(err) => SaslConnectError::Io(err),
        }
    }
}

#[derive(Debug, Display)]
#[display(fmt = "Amqp error: {:?} {:?} ({:?})", err, description, info)]
pub struct AmqpError {
    err: Either<protocol::AmqpError, protocol::ErrorCondition>,
    description: Option<ByteString>,
    info: Option<protocol::Fields>,
}

impl AmqpError {
    pub fn new(err: protocol::AmqpError) -> Self {
        AmqpError {
            err: Either::Left(err),
            description: None,
            info: None,
        }
    }

    pub fn with_error(err: protocol::ErrorCondition) -> Self {
        AmqpError {
            err: Either::Right(err),
            description: None,
            info: None,
        }
    }

    pub fn internal_error() -> Self {
        Self::new(protocol::AmqpError::InternalError)
    }

    pub fn not_found() -> Self {
        Self::new(protocol::AmqpError::NotFound)
    }

    pub fn unauthorized_access() -> Self {
        Self::new(protocol::AmqpError::UnauthorizedAccess)
    }

    pub fn decode_error() -> Self {
        Self::new(protocol::AmqpError::DecodeError)
    }

    pub fn invalid_field() -> Self {
        Self::new(protocol::AmqpError::InvalidField)
    }

    pub fn not_allowed() -> Self {
        Self::new(protocol::AmqpError::NotAllowed)
    }

    pub fn not_implemented() -> Self {
        Self::new(protocol::AmqpError::NotImplemented)
    }

    pub fn description<T: AsRef<str>>(mut self, text: T) -> Self {
        self.description = Some(ByteString::from(text.as_ref()));
        self
    }

    pub fn set_description(mut self, text: ByteString) -> Self {
        self.description = Some(text);
        self
    }
}

impl Into<protocol::Error> for AmqpError {
    fn into(self) -> protocol::Error {
        let condition = match self.err {
            Either::Left(err) => err.into(),
            Either::Right(err) => err,
        };
        protocol::Error {
            condition,
            description: self.description,
            info: self.info,
        }
    }
}

#[derive(Debug, Display)]
#[display(fmt = "Link error: {:?} {:?} ({:?})", err, description, info)]
pub struct LinkError {
    err: Either<protocol::LinkError, protocol::ErrorCondition>,
    description: Option<ByteString>,
    info: Option<protocol::Fields>,
}

impl LinkError {
    pub fn new(error: protocol::ErrorCondition) -> Self {
        LinkError {
            err: Either::Right(error),
            description: None,
            info: None,
        }
    }

    pub fn force_detach() -> Self {
        LinkError {
            err: Either::Left(protocol::LinkError::DetachForced),
            description: None,
            info: None,
        }
    }

    pub fn redirect() -> Self {
        LinkError {
            err: Either::Left(protocol::LinkError::Redirect),
            description: None,
            info: None,
        }
    }

    pub fn text(mut self, text: &'static str) -> Self {
        self.description = Some(ByteString::from_static(text));
        self
    }

    pub fn description<T: AsRef<str>>(mut self, text: T) -> Self {
        self.description = Some(ByteString::from(text.as_ref()));
        self
    }

    pub fn set_description(mut self, text: ByteString) -> Self {
        self.description = Some(text);
        self
    }

    #[allow(clippy::mutable_key_type)]
    pub fn fields(mut self, fields: protocol::Fields) -> Self {
        self.info = Some(fields);
        self
    }
}

impl Into<protocol::Error> for LinkError {
    fn into(self) -> protocol::Error {
        let condition = match self.err {
            Either::Left(err) => err.into(),
            Either::Right(err) => err,
        };

        protocol::Error {
            condition,
            description: self.description,
            info: self.info,
        }
    }
}

#[derive(Debug, Display, From)]
pub enum ProtocolNegotiationError {
    Disconnected,
    Protocol(ProtocolIdError),
    Io(std::io::Error),
}

impl From<Either<ProtocolIdError, std::io::Error>> for ProtocolNegotiationError {
    fn from(err: Either<ProtocolIdError, std::io::Error>) -> Self {
        match err {
            Either::Left(err) => ProtocolNegotiationError::Protocol(err),
            Either::Right(err) => ProtocolNegotiationError::Io(err),
        }
    }
}
