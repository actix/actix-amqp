use std::fmt;

use bytes::Bytes;

use crate::codec::protocol::{
    Accepted, DeliveryState, Error, Rejected, Transfer as AmqpTransfer, TransferBody,
};
use crate::{codec::Decode, rcvlink::ReceiverLink, session::Session, State};

use super::AmqpError;

pub struct Transfer<S> {
    state: State<S>,
    frame: AmqpTransfer,
    link: ReceiverLink,
}

#[derive(Debug)]
pub enum Outcome {
    Accept,
    Reject,
    Error(Error),
}

impl<T> From<T> for Outcome
where
    T: Into<Error>,
{
    fn from(err: T) -> Self {
        Outcome::Error(err.into())
    }
}

impl Outcome {
    pub(crate) fn into_delivery_state(self) -> DeliveryState {
        match self {
            Outcome::Accept => DeliveryState::Accepted(Accepted {}),
            Outcome::Reject => DeliveryState::Rejected(Rejected { error: None }),
            Outcome::Error(e) => DeliveryState::Rejected(Rejected { error: Some(e) }),
        }
    }
}

impl<S> Transfer<S> {
    pub(crate) fn new(state: State<S>, frame: AmqpTransfer, link: ReceiverLink) -> Self {
        Transfer { state, frame, link }
    }

    pub fn state(&self) -> &S {
        self.state.get_ref()
    }

    pub fn state_mut(&mut self) -> &mut S {
        self.state.get_mut()
    }

    pub fn session(&self) -> &Session {
        self.link.session()
    }

    pub fn session_mut(&mut self) -> &mut Session {
        self.link.session_mut()
    }

    pub fn frame(&self) -> &AmqpTransfer {
        &self.frame
    }

    pub fn body(&self) -> Option<&Bytes> {
        match self.frame.body {
            Some(TransferBody::Data(ref b)) => Some(b),
            _ => None,
        }
    }

    pub fn load_message<T: Decode>(&self) -> Result<T, AmqpError> {
        if let Some(TransferBody::Data(ref b)) = self.frame.body {
            if let Ok((_, msg)) = T::decode(b) {
                Ok(msg)
            } else {
                Err(AmqpError::decode_error().description("Can not decode message"))
            }
        } else {
            Err(AmqpError::invalid_field().description("Unknown body"))
        }
    }
}

impl<S> fmt::Debug for Transfer<S> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Transfer<S>")
            .field("frame", &self.frame)
            .finish()
    }
}
